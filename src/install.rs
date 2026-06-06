use crate::context::Context;
use crate::manifest::{FetchedManifest, ImageIndex, ImageManifest};
use anyhow::{Context as _, anyhow, bail};
use flate2::read::GzDecoder;
use serde::Deserialize;
use std::fs;
use std::io::{BufRead, BufReader, Read};
use std::path::Path;
use tar::Archive;

const DOCKER_AUTH: &str = "https://auth.docker.io/token";
const MANIFEST_ACCEPT: &str = "application/vnd.oci.image.manifest.v1+json, application/vnd.docker.distribution.manifest.v2+json, application/vnd.oci.image.index.v1+json, application/vnd.docker.distribution.manifest.list.v2+json";
const LAYER_ACCEPT: &str = "application/vnd.oci.image.layer.v1.tar+gzip, application/vnd.docker.image.rootfs.diff.tar.gzip, application/octet-stream";

#[derive(Debug, Deserialize)]
struct AuthToken {
    token: Option<String>,
    access_token: Option<String>,
}

#[derive(Debug, Clone)]
struct ImageRef {
    registry: String,
    repository: String,
    reference: String,
}

pub fn install(ctx: &Context, image: &str) -> anyhow::Result<()> {
    let image_ref = ImageRef::parse(image)
        .with_context(|| format!("failed to parse image reference `{image}`"))?;
    let client = RegistryClient::new(image_ref.registry.clone(), image_ref.repository.clone())
        .with_context(|| {
            format!(
                "failed to initialize registry client for {}/{}",
                image_ref.registry, image_ref.repository
            )
        })?;
    let fetched_manifest = client
        .fetch_image_manifest(&image_ref.reference)
        .with_context(|| {
            format!(
                "failed to fetch manifest `{}` from {}/{}",
                image_ref.reference, image_ref.registry, image_ref.repository
            )
        })?;

    save_manifest(ctx, &fetched_manifest).with_context(|| {
        format!(
            "failed to save manifest for {}/{}:{} under {}",
            image_ref.registry,
            image_ref.repository,
            image_ref.reference,
            ctx.storage_path.display()
        )
    })?;

    eprintln!(
        "{} has {} layers",
        image,
        fetched_manifest.manifest.layers.len()
    );

    for layer in &fetched_manifest.manifest.layers {
        let digest_hash = layer
            .digest
            .split_once(':')
            .map(|(_, hash)| hash)
            .unwrap_or(&layer.digest);
        let output_dir = ctx.storage_path.join("layers").join(digest_hash);
        if output_dir.exists() {
            eprintln!(
                "layer {} already extracted to {}",
                layer.digest,
                output_dir.display()
            );
            continue;
        }

        fs::create_dir_all(&output_dir).with_context(|| {
            format!(
                "failed to create layer output directory {}",
                output_dir.display()
            )
        })?;
        match client.fetch_blob_reader(&layer.digest) {
            Ok(reader) => {
                extract_layer(reader, &output_dir)
                    .with_context(|| format!("failed to extract layer {}", layer.digest))?;
                eprintln!("extracted {} to {}", layer.digest, output_dir.display());
            }
            Err(err) => {
                let _ = fs::remove_dir_all(&output_dir);
                return Err(err).with_context(|| format!("failed to fetch layer {}", layer.digest));
            }
        }
    }

    Ok(())
}

impl ImageRef {
    fn parse(input: &str) -> anyhow::Result<Self> {
        let name = if let Some((name, _digest)) = input.split_once('@') {
            name
        } else {
            let last_slash = input.rfind('/');
            let last_colon = input.rfind(':');
            if let Some(colon) = last_colon {
                if last_slash.map_or(true, |slash| colon > slash) {
                    &input[..colon]
                } else {
                    input
                }
            } else {
                input
            }
        };

        let reference = if let Some((_name, digest)) = input.split_once('@') {
            digest
        } else {
            let last_slash = input.rfind('/');
            let last_colon = input.rfind(':');
            if let Some(colon) = last_colon {
                if last_slash.map_or(true, |slash| colon > slash) {
                    &input[colon + 1..]
                } else {
                    "latest"
                }
            } else {
                "latest"
            }
        };
        let mut parts = name.split('/');
        let first = parts.next().context("image name is empty")?;

        let (registry, repository) =
            if first.contains('.') || first.contains(':') || first == "localhost" {
                let rest = parts.collect::<Vec<_>>().join("/");
                if rest.is_empty() {
                    bail!("image repository is missing");
                }
                (first.to_string(), rest)
            } else {
                let repository = if name.contains('/') {
                    name.to_string()
                } else {
                    format!("library/{name}")
                };
                ("registry-1.docker.io".to_string(), repository)
            };

        Ok(Self {
            registry,
            repository,
            reference: reference.to_string(),
        })
    }
}

struct RegistryClient {
    registry: String,
    repository: String,
    token: Option<String>,
    agent: ureq::Agent,
}

struct ManifestBytes {
    bytes: Vec<u8>,
    digest: Option<String>,
}

impl RegistryClient {
    fn new(registry: String, repository: String) -> anyhow::Result<Self> {
        let token =
            if registry == "registry-1.docker.io" {
                Some(fetch_docker_token(&repository).with_context(|| {
                    format!("failed to fetch Docker Hub token for {repository}")
                })?)
            } else {
                None
            };

        Ok(Self {
            registry,
            repository,
            token,
            agent: ureq::AgentBuilder::new().build(),
        })
    }

    fn fetch_image_manifest(&self, reference: &str) -> anyhow::Result<FetchedManifest> {
        let fetched = self
            .fetch_manifest_bytes(reference)
            .with_context(|| format!("failed to fetch manifest bytes for `{reference}`"))?;
        if let Ok(manifest) = serde_json::from_slice::<ImageManifest>(&fetched.bytes) {
            if manifest.schema_version == 2 {
                let digest = fetched.digest.with_context(|| {
                    format!("registry did not provide manifest digest for `{reference}`")
                })?;
                return Ok(FetchedManifest {
                    manifest,
                    bytes: fetched.bytes,
                    digest,
                });
            }
        }

        let index: ImageIndex = serde_json::from_slice(&fetched.bytes)
            .with_context(|| format!("failed to parse manifest index `{reference}`"))?;
        let descriptor = index
            .manifests
            .iter()
            .find(|manifest| manifest.matches_current_platform())
            .or_else(|| index.manifests.first())
            .context("manifest index does not contain any manifests")?;

        let fetched = self
            .fetch_manifest_bytes(&descriptor.digest)
            .with_context(|| format!("failed to fetch platform manifest {}", descriptor.digest))?;
        let manifest: ImageManifest = serde_json::from_slice(&fetched.bytes)
            .with_context(|| format!("failed to parse image manifest {}", descriptor.digest))?;
        if manifest.schema_version != 2 {
            bail!("unsupported image manifest schema");
        }
        let digest = fetched.digest.unwrap_or_else(|| descriptor.digest.clone());
        Ok(FetchedManifest {
            manifest,
            bytes: fetched.bytes,
            digest,
        })
    }

    fn fetch_manifest_bytes(&self, reference: &str) -> anyhow::Result<ManifestBytes> {
        let url = format!(
            "https://{}/v2/{}/manifests/{}",
            self.registry, self.repository, reference
        );
        let response = self
            .request("GET", &url)
            .set("Accept", MANIFEST_ACCEPT)
            .call()
            .with_context(|| format!("registry request failed: GET {url}"))?;

        let digest = response.header("Docker-Content-Digest").map(str::to_string);
        let mut bytes = Vec::new();
        response
            .into_reader()
            .read_to_end(&mut bytes)
            .with_context(|| format!("failed to read manifest response body from {url}"))?;
        Ok(ManifestBytes { bytes, digest })
    }

    fn fetch_blob_reader(&self, digest: &str) -> anyhow::Result<Box<dyn Read>> {
        let url = format!(
            "https://{}/v2/{}/blobs/{}",
            self.registry, self.repository, digest
        );
        let response = self
            .request("GET", &url)
            .set("Accept", LAYER_ACCEPT)
            .call()
            .with_context(|| format!("registry request failed: GET {url}"))?;

        Ok(Box::new(BufReader::new(response.into_reader())))
    }

    fn request(&self, method: &str, url: &str) -> ureq::Request {
        let request = self.agent.request(method, url);
        match &self.token {
            Some(token) => request.set("Authorization", &format!("Bearer {token}")),
            None => request,
        }
    }
}

fn fetch_docker_token(repository: &str) -> anyhow::Result<String> {
    let url =
        format!("{DOCKER_AUTH}?service=registry.docker.io&scope=repository:{repository}:pull");
    let token: AuthToken = ureq::get(&url)
        .call()
        .with_context(|| format!("Docker auth request failed: GET {url}"))?
        .into_json()
        .context("failed to parse Docker auth token response")?;
    token
        .token
        .or(token.access_token)
        .ok_or_else(|| anyhow!("Docker auth response did not include a token"))
}

fn save_manifest(ctx: &Context, fetched_manifest: &FetchedManifest) -> anyhow::Result<()> {
    let output_dir = ctx.storage_path.join("manifests");
    fs::create_dir_all(&output_dir).with_context(|| {
        format!(
            "failed to create manifests directory {}",
            output_dir.display()
        )
    })?;

    let output_path = output_dir.join(format!(
        "{}.json",
        fetched_manifest
            .digest
            .split_once(':')
            .map(|(_, hash)| hash)
            .unwrap_or(&fetched_manifest.digest)
    ));

    fs::write(&output_path, &fetched_manifest.bytes)
        .with_context(|| format!("failed to write manifest to {}", output_path.display()))
}

fn extract_layer(reader: Box<dyn Read>, output_dir: &Path) -> anyhow::Result<()> {
    let mut peekable = BufReader::new(reader);
    let buffer = peekable
        .fill_buf()
        .context("failed to inspect layer bytes")?;
    let is_gzip = buffer.starts_with(&[0x1f, 0x8b]);

    if is_gzip {
        let decoder = GzDecoder::new(peekable);
        Archive::new(decoder).unpack(output_dir).with_context(|| {
            format!("failed to unpack gzip layer into {}", output_dir.display())
        })?;
    } else {
        Archive::new(peekable)
            .unpack(output_dir)
            .with_context(|| format!("failed to unpack tar layer into {}", output_dir.display()))?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_default_docker_library_image() {
        let image = ImageRef::parse("alpine").unwrap();
        assert_eq!(image.registry, "registry-1.docker.io");
        assert_eq!(image.repository, "library/alpine");
        assert_eq!(image.reference, "latest");
    }

    #[test]
    fn parses_docker_hub_namespace_image() {
        let image = ImageRef::parse("username/image:latest").unwrap();
        assert_eq!(image.registry, "registry-1.docker.io");
        assert_eq!(image.repository, "username/image");
        assert_eq!(image.reference, "latest");
    }

    #[test]
    fn parses_explicit_registry_tag() {
        let image = ImageRef::parse("ghcr.io/org/app:1.2.3").unwrap();
        assert_eq!(image.registry, "ghcr.io");
        assert_eq!(image.repository, "org/app");
        assert_eq!(image.reference, "1.2.3");
    }

    #[test]
    fn parses_digest_reference() {
        let image = ImageRef::parse("example.com/org/app@sha256:abc").unwrap();
        assert_eq!(image.registry, "example.com");
        assert_eq!(image.repository, "org/app");
        assert_eq!(image.reference, "sha256:abc");
    }
}
