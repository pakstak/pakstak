use crate::auth;
use crate::digest;
use crate::manifest::{FetchedManifest, ImageIndex, ImageManifest};
use crate::reference::{Reference, Specifier};
use crate::storage::StorageMutable;
use anyhow::{Context as _, bail};
use std::io::{BufReader, Read};
use std::sync::Arc;
use ureq::tls::{RootCerts, TlsConfig};

const OCI_IMAGE_MANIFEST_MEDIA_TYPE: &str = "application/vnd.oci.image.manifest.v1+json";
const OCI_IMAGE_INDEX_MEDIA_TYPE: &str = "application/vnd.oci.image.index.v1+json";
const LAYER_ACCEPT: &str = "application/vnd.oci.image.layer.v1.tar+gzip";

pub struct RegistryClient {
    registry: String,
    repository: String,
    token: Option<String>,
    agent: ureq::Agent,
}

struct ManifestBytes {
    bytes: Vec<u8>,
    digest: String,
    media_type: ManifestMediaType,
}

impl RegistryClient {
    pub fn new(registry: String, repository: String) -> anyhow::Result<Self> {
        Ok(Self {
            registry,
            repository,
            token: None,
            agent: ureq::Agent::config_builder()
                .tls_config(
                    TlsConfig::builder()
                        .root_certs(RootCerts::PlatformVerifier)
                        .unversioned_rustls_crypto_provider(Arc::new(
                            rustls::crypto::ring::default_provider(),
                        ))
                        .build(),
                )
                .build()
                .new_agent(),
        })
    }

    pub fn fetch_image_manifest(
        &mut self,
        specifier: &Specifier,
    ) -> anyhow::Result<FetchedManifest> {
        let fetched = self
            .fetch_manifest_bytes(
                specifier,
                &[OCI_IMAGE_MANIFEST_MEDIA_TYPE, OCI_IMAGE_INDEX_MEDIA_TYPE].join(", "),
            )
            .with_context(|| {
                format!(
                    "failed to fetch manifest bytes for `{}`",
                    specifier.as_typeless_str()
                )
            })?;
        match fetched.media_type {
            ManifestMediaType::Index => self.handle_image_index(&fetched, specifier),
            ManifestMediaType::Manifest => {
                parse_image_manifest(fetched, specifier.as_typeless_str())
            }
            ManifestMediaType::Unknown => {
                let index_error = match self.handle_image_index(&fetched, specifier) {
                    Ok(manifest) => return Ok(manifest),
                    Err(error) => error,
                };
                parse_image_manifest(fetched, specifier.as_typeless_str()).with_context(|| {
                    format!(
                        "failed to parse as image manifest after \
                            fallback index parse failed: {index_error}"
                    )
                })
            }
        }
    }

    fn handle_image_index(
        &mut self,
        fetched: &ManifestBytes,
        specifier: &Specifier,
    ) -> anyhow::Result<FetchedManifest> {
        let index: ImageIndex = serde_json::from_slice(&fetched.bytes).with_context(|| {
            format!(
                "failed to parse manifest index `{}`",
                specifier.as_typeless_str()
            )
        })?;
        let descriptor_digest = index
            .manifests
            .iter()
            .find(|manifest| manifest.matches_current_platform())
            .or_else(|| index.manifests.first())
            .context("manifest index does not contain any manifests")?
            .digest
            .clone();

        // Otherwise the initial reference is an image index;
        // fetch the selected platform manifest by digest before fetching layers.
        let fetched = self
            .fetch_manifest_bytes(
                &Specifier::Digest(descriptor_digest.clone()),
                OCI_IMAGE_MANIFEST_MEDIA_TYPE,
            )
            .with_context(|| format!("failed to fetch platform manifest {descriptor_digest}"))?;
        parse_image_manifest(fetched, &descriptor_digest)
    }

    fn fetch_manifest_bytes(
        &mut self,
        specifier: &Specifier,
        accept: &str,
    ) -> anyhow::Result<ManifestBytes> {
        let url = format!(
            "https://{}/v2/{}/manifests/{}",
            self.registry,
            self.repository,
            specifier.as_typeless_str(),
        );
        let response = self
            .call_get_with_auth_retry(&url, |request| request.header("Accept", accept))
            .with_context(|| format!("registry request failed: GET {url}"))?;
        let media_type = ManifestMediaType::parse_header(response.headers().get("Content-Type"));
        let header_digest = response
            .headers()
            .get("Docker-Content-Digest")
            .context("registry response did not include Docker-Content-Digest header")?
            .to_str()
            .context("registry returned an invalid Docker-Content-Digest header")?
            .to_string();

        let mut bytes = Vec::new();
        let (_, body) = response.into_parts();
        body.into_reader()
            .read_to_end(&mut bytes)
            .with_context(|| format!("failed to read manifest response body from {url}"))?;
        digest::verify_bytes(&bytes, &header_digest)
            .with_context(|| format!("failed to verify manifest digest {header_digest}"))?;
        if let Specifier::Digest(requested_digest) = specifier {
            digest::verify_bytes(&bytes, requested_digest)
                .with_context(|| format!("failed to verify manifest digest {requested_digest}"))?;
        }
        Ok(ManifestBytes {
            bytes,
            digest: header_digest,
            media_type,
        })
    }

    fn fetch_blob_reader(&mut self, digest: &str) -> anyhow::Result<impl Read> {
        let url = format!(
            "https://{}/v2/{}/blobs/{}",
            self.registry, self.repository, digest
        );
        let response = self
            .call_get_with_auth_retry(&url, |request| request.header("Accept", LAYER_ACCEPT))
            .with_context(|| format!("registry request failed: GET {url}"))?;

        let (_, body) = response.into_parts();
        Ok(BufReader::new(body.into_reader()))
    }

    fn call_get_with_auth_retry<F>(
        &mut self,
        url: &str,
        configure: F,
    ) -> anyhow::Result<ureq::http::Response<ureq::Body>>
    where
        F: Fn(
            ureq::RequestBuilder<ureq::typestate::WithoutBody>,
        ) -> ureq::RequestBuilder<ureq::typestate::WithoutBody>,
    {
        match configure(self.request_get(url)).call() {
            Ok(response) => Ok(response),
            Err(ureq::Error::StatusCode(401)) => {
                let unauthorized = configure(self.request_get(url))
                    .config()
                    .http_status_as_error(false)
                    .build()
                    .call()
                    .with_context(|| {
                        format!("failed to fetch unauthorized registry response: GET {url}")
                    })?;
                let token = auth::token_from_unauthorized(&self.agent, &unauthorized)
                    .with_context(|| {
                        format!("failed to authenticate registry request: GET {url}")
                    })?;
                self.token = Some(token);
                configure(self.request_get(url))
                    .call()
                    .with_context(|| format!("authenticated registry request failed: GET {url}"))
            }
            Err(error) => Err(error.into()),
        }
    }

    fn request_get(&self, url: &str) -> ureq::RequestBuilder<ureq::typestate::WithoutBody> {
        let request = self.agent.get(url);
        match &self.token {
            Some(token) => request.header("Authorization", format!("Bearer {token}")),
            None => request,
        }
    }
}

fn parse_image_manifest(
    fetched: ManifestBytes,
    manifest_context: &str,
) -> anyhow::Result<FetchedManifest> {
    let manifest: ImageManifest = serde_json::from_slice(&fetched.bytes)
        .with_context(|| format!("failed to parse image manifest `{manifest_context}`"))?;
    if manifest.schema_version != 2 {
        bail!("unsupported image manifest schema");
    }
    Ok(FetchedManifest {
        manifest,
        bytes: fetched.bytes,
        digest: fetched.digest,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ManifestMediaType {
    Index,
    Manifest,
    Unknown,
}

impl ManifestMediaType {
    fn parse_header(header: Option<&ureq::http::HeaderValue>) -> Self {
        let Some(header) = header else {
            return Self::Unknown;
        };
        let media_type = match header.to_str() {
            Ok(media_type) => media_type,
            Err(error) => {
                eprintln!("failed to parse manifest Content-Type header: {error}");
                return Self::Unknown;
            }
        };

        media_type
            .split(';')
            .find_map(|s| match s.trim() {
                OCI_IMAGE_INDEX_MEDIA_TYPE => Some(Self::Index),
                OCI_IMAGE_MANIFEST_MEDIA_TYPE => Some(Self::Manifest),
                _ => None,
            })
            .unwrap_or_else(|| {
                eprintln!("unsupported manifest media type detected: `{media_type}`");
                Self::Unknown
            })
    }
}

pub fn fetch_image(
    storage: &StorageMutable,
    reference: &Reference,
    repair: bool,
) -> anyhow::Result<FetchedManifest> {
    let mut client = RegistryClient::new(reference.registry.clone(), reference.repository.clone())
        .with_context(|| format!("failed to initialize registry client for {reference}"))?;
    let fetched_manifest = client
        .fetch_image_manifest(&reference.specifier)
        .with_context(|| format!("failed to fetch manifest for {reference}"))?;

    let is_manifest_saved = storage
        .is_manifest_saved(&fetched_manifest.digest, repair)
        .with_context(|| format!("failed to verify manifest for {}", &fetched_manifest.digest))?;

    if is_manifest_saved && !repair {
        eprintln!("manifest {} is already installed", fetched_manifest.digest);
        return Ok(fetched_manifest);
    }

    fetch_layers(storage, &mut client, &fetched_manifest)?;

    if !is_manifest_saved {
        storage.write_manifest(&fetched_manifest.digest, &fetched_manifest.bytes)?;
    }

    Ok(fetched_manifest)
}

fn fetch_layers(
    storage: &StorageMutable,
    client: &mut RegistryClient,
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

        let reader = client
            .fetch_blob_reader(&layer.digest)
            .with_context(|| format!("failed to fetch layer {}", layer.digest))?;

        storage
            .write_layer(&layer.digest, reader)
            .with_context(|| format!("failed to fetch and extract layer {}", layer.digest))?;

        eprintln!("extracted {}", layer.digest);
    }

    Ok(())
}
