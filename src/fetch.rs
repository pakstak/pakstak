use crate::context::Context;
use crate::manifest::{FetchedManifest, ImageIndex, ImageManifest};
use anyhow::{Context as _, anyhow, bail};
use flate2::read::GzDecoder;
use serde::{Deserialize, Serialize};
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

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ImageRef {
    pub registry: String,
    pub repository: String,
    pub reference: String,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct AppMetadata {
    pub source: AppSourceMetadata,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct AppSourceMetadata {
    pub image: String,
    pub registry: String,
    pub image_name: String,
    pub reference: String,
    pub tag: Option<String>,
    pub digest: Option<String>,
}

impl AppMetadata {
    pub fn from_image(image: &str, image_ref: &ImageRef) -> Self {
        Self {
            source: AppSourceMetadata {
                image: image.to_string(),
                registry: image_ref.registry.clone(),
                image_name: image_ref.repository.clone(),
                reference: image_ref.reference.clone(),
                tag: (!image_ref.reference.contains(':')).then_some(image_ref.reference.clone()),
                digest: image_ref
                    .reference
                    .contains(':')
                    .then_some(image_ref.reference.clone()),
            },
        }
    }
}

impl ImageRef {
    pub fn parse(input: &str) -> anyhow::Result<Self> {
        let name = if let Some((name, _digest)) = input.split_once('@') {
            name
        } else {
            let last_slash = input.rfind('/');
            let last_colon = input.rfind(':');
            if let Some(colon) = last_colon {
                if last_slash.is_none_or(|slash| colon > slash) {
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
                if last_slash.is_none_or(|slash| colon > slash) {
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

    pub fn from_metadata(metadata: &AppMetadata) -> Self {
        Self {
            registry: metadata.source.registry.clone(),
            repository: metadata.source.image_name.clone(),
            reference: metadata.source.reference.clone(),
        }
    }
}

pub struct RegistryClient {
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
    pub fn new(registry: String, repository: String) -> anyhow::Result<Self> {
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

    pub fn fetch_image_manifest(&self, reference: &str) -> anyhow::Result<FetchedManifest> {
        let fetched = self
            .fetch_manifest_bytes(reference)
            .with_context(|| format!("failed to fetch manifest bytes for `{reference}`"))?;
        if let Ok(manifest) = serde_json::from_slice::<ImageManifest>(&fetched.bytes)
            && manifest.schema_version == 2
        {
            let digest = fetched.digest.with_context(|| {
                format!("registry did not provide manifest digest for `{reference}`")
            })?;
            return Ok(FetchedManifest {
                manifest,
                bytes: fetched.bytes,
                digest,
            });
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

pub fn validate_alias(alias: &str) -> anyhow::Result<()> {
    if alias.is_empty() {
        bail!("app alias cannot be empty");
    }
    if alias == "." || alias == ".." {
        bail!("app alias `{alias}` is not allowed");
    }
    if !alias
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
    {
        bail!(
            "app alias `{alias}` contains invalid characters; use only ASCII letters, numbers, dots, underscores, and dashes"
        );
    }

    Ok(())
}

pub fn fetch_image(ctx: &Context, image_ref: &ImageRef) -> anyhow::Result<FetchedManifest> {
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

    let manifest_path = ctx
        .storage_path
        .join("manifests")
        .join(format!("{}.json", fetched_manifest.digest));
    if manifest_path.is_file() {
        eprintln!(
            "manifest {} already saved; skipping layer fetch",
            fetched_manifest.digest
        );
        return Ok(fetched_manifest);
    }

    fetch_layers(ctx, &client, &fetched_manifest)?;

    save_manifest(ctx, &fetched_manifest).with_context(|| {
        format!(
            "failed to save manifest for {}/{}:{} under {}",
            image_ref.registry,
            image_ref.repository,
            image_ref.reference,
            ctx.storage_path.display()
        )
    })?;

    Ok(fetched_manifest)
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

fn save_manifest(ctx: &Context, fetched_manifest: &FetchedManifest) -> anyhow::Result<String> {
    let output_dir = ctx.storage_path.join("manifests");
    fs::create_dir_all(&output_dir).with_context(|| {
        format!(
            "failed to create manifests directory {}",
            output_dir.display()
        )
    })?;

    let output_path = output_dir.join(format!("{}.json", fetched_manifest.digest));

    ctx.atomic_write(&output_path, &fetched_manifest.bytes)
        .with_context(|| format!("failed to write manifest to {}", output_path.display()))?;

    Ok(fetched_manifest.digest.clone())
}

fn fetch_layers(
    ctx: &Context,
    client: &RegistryClient,
    fetched_manifest: &FetchedManifest,
) -> anyhow::Result<()> {
    eprintln!(
        "manifest {} has {} layers",
        fetched_manifest.digest,
        fetched_manifest.manifest.layers.len()
    );

    for layer in &fetched_manifest.manifest.layers {
        let output_dir = ctx.storage_path.join("layers").join(&layer.digest);
        if output_dir.exists() {
            eprintln!(
                "layer {} already extracted to {}",
                layer.digest,
                output_dir.display()
            );
            continue;
        }

        let temporary_output_dir = ctx.temporary_directory_for(&output_dir)?;
        fs::create_dir_all(&temporary_output_dir).with_context(|| {
            format!(
                "failed to create temporary layer output directory {}",
                temporary_output_dir.display()
            )
        })?;
        match client.fetch_blob_reader(&layer.digest) {
            Ok(reader) => {
                extract_layer(reader, &temporary_output_dir)
                    .with_context(|| format!("failed to extract layer {}", layer.digest))?;
                ctx.publish_directory(&temporary_output_dir, &output_dir)
                    .with_context(|| {
                        format!(
                            "failed to publish layer {} to {}",
                            layer.digest,
                            output_dir.display()
                        )
                    })?;
                eprintln!("extracted {} to {}", layer.digest, output_dir.display());
            }
            Err(err) => {
                let _ = fs::remove_dir_all(&temporary_output_dir);
                return Err(err).with_context(|| format!("failed to fetch layer {}", layer.digest));
            }
        }
    }

    Ok(())
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
