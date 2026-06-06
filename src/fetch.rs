use crate::digest;
use crate::manifest::{FetchedManifest, ImageIndex, ImageManifest};
use crate::reference::Reference;
use crate::storage::StorageMutable;
use anyhow::{Context as _, anyhow, bail};
use serde::Deserialize;
use std::io::{BufReader, Read};

const DOCKER_AUTH: &str = "https://auth.docker.io/token";
const MANIFEST_ACCEPT: &str = "application/vnd.oci.image.manifest.v1+json, application/vnd.docker.distribution.manifest.v2+json, application/vnd.oci.image.index.v1+json, application/vnd.docker.distribution.manifest.list.v2+json";
const LAYER_ACCEPT: &str = "application/vnd.oci.image.layer.v1.tar+gzip, application/vnd.docker.image.rootfs.diff.tar.gzip, application/octet-stream";

#[derive(Debug, Deserialize)]
struct AuthToken {
    token: Option<String>,
    access_token: Option<String>,
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
        let requested_digest = reference.contains(':').then_some(reference);
        if let Some(digest) = requested_digest.or(fetched.digest.as_deref()) {
            digest::verify_bytes(&fetched.bytes, digest)
                .with_context(|| format!("failed to verify manifest digest {digest}"))?;
        }
        // The initial reference may already point to a platform-specific image manifest.
        if let Ok(manifest) = serde_json::from_slice::<ImageManifest>(&fetched.bytes)
            && manifest.schema_version == 2
        {
            let digest = fetched
                .digest
                .or_else(|| requested_digest.map(str::to_string))
                .with_context(|| {
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

        // Otherwise the initial reference is an image index / manifest list;
        // fetch the selected platform manifest by digest before fetching layers.
        let fetched = self
            .fetch_manifest_bytes(&descriptor.digest)
            .with_context(|| format!("failed to fetch platform manifest {}", descriptor.digest))?;
        digest::verify_bytes(&fetched.bytes, &descriptor.digest).with_context(|| {
            format!(
                "failed to verify platform manifest digest {}",
                descriptor.digest
            )
        })?;
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

pub fn validate_container(container: &str) -> anyhow::Result<()> {
    if container.is_empty() {
        bail!("container name cannot be empty");
    }
    if container == "." || container == ".." {
        bail!("container name `{container}` is not allowed");
    }
    if !container
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
    {
        bail!(
            "container name `{container}` contains invalid characters; use only ASCII letters, numbers, dots, underscores, and dashes"
        );
    }

    Ok(())
}

pub fn fetch_image(
    storage: &StorageMutable,
    reference: &Reference,
) -> anyhow::Result<FetchedManifest> {
    let client = RegistryClient::new(reference.registry.clone(), reference.repository.clone())
        .with_context(|| format!("failed to initialize registry client for {reference}"))?;
    let fetched_manifest = client
        .fetch_image_manifest(reference.manifest_reference())
        .with_context(|| {
            format!(
                "failed to fetch manifest `{}` for {reference}",
                reference.manifest_reference()
            )
        })?;

    if storage.is_manifest_saved(&fetched_manifest.digest) {
        eprintln!(
            "manifest {} already saved; skipping layer fetch",
            fetched_manifest.digest
        );
        return Ok(fetched_manifest);
    }

    fetch_layers(storage, &client, &fetched_manifest)?;

    storage.write_manifest(&fetched_manifest.digest, &fetched_manifest.bytes)?;

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

fn fetch_layers(
    storage: &StorageMutable,
    client: &RegistryClient,
    fetched_manifest: &FetchedManifest,
) -> anyhow::Result<()> {
    eprintln!(
        "manifest {} has {} layers",
        fetched_manifest.digest,
        fetched_manifest.manifest.layers.len()
    );

    for layer in &fetched_manifest.manifest.layers {
        if storage.get_layer_path(&layer.digest).is_some() {
            eprintln!("layer {} already extracted", layer.digest);
            continue;
        }

        match client.fetch_blob_reader(&layer.digest) {
            Ok(reader) => {
                storage
                    .write_layer(&layer.digest, reader)
                    .with_context(|| {
                        format!("failed to fetch and extract layer {}", layer.digest)
                    })?;
                eprintln!("extracted {}", layer.digest);
            }
            Err(err) => {
                return Err(err).with_context(|| format!("failed to fetch layer {}", layer.digest));
            }
        }
    }

    Ok(())
}
