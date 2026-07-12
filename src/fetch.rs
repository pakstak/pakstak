use crate::auth;
use crate::digest;
use crate::manifest::{FetchedManifest, ImageIndex, ImageManifest};
use crate::reference::{Reference, Specifier};
use crate::storage::StorageMutable;
use anyhow::{Context as _, bail};
use std::collections::HashMap;
use std::io::{BufReader, Read};
use std::sync::Arc;
use ureq::tls::{RootCerts, TlsConfig};

const MANIFEST_MEDIA_TYPES: &[&str] = &[
    "application/vnd.oci.image.manifest.v1+json",
    "application/vnd.docker.distribution.manifest.v2+json",
];
const INDEX_MEDIA_TYPES: &[&str] = &[
    "application/vnd.oci.image.index.v1+json",
    "application/vnd.docker.distribution.manifest.list.v2+json",
];
const LAYER_MEDIA_TYPES: &[&str] = &[
    "application/vnd.oci.image.layer.v1.tar+gzip",
    "application/vnd.docker.image.rootfs.diff.tar.gzip",
];

pub struct RegistryClient {
    tokens: HashMap<String, String>,
    agent: ureq::Agent,
}

impl RegistryClient {
    pub fn new() -> Self {
        Self {
            tokens: HashMap::new(),
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
        }
    }

    pub fn fetch_image(
        &mut self,
        storage: &StorageMutable,
        reference: &Reference,
        repair: bool,
    ) -> anyhow::Result<FetchedManifest> {
        let fetched_manifest = self
            .fetch_image_manifest(reference)
            .with_context(|| format!("failed to fetch manifest for {reference}"))?;

        let is_manifest_saved = storage
            .is_manifest_saved(&fetched_manifest.digest, repair)
            .with_context(|| {
                format!("failed to verify manifest for {}", &fetched_manifest.digest)
            })?;

        if is_manifest_saved && !repair {
            eprintln!("manifest {} is already installed", fetched_manifest.digest);
            return Ok(fetched_manifest);
        }

        self.fetch_layers(storage, reference, &fetched_manifest)
            .with_context(|| {
                format!(
                    "failed to fetch layers for manifest {}",
                    fetched_manifest.digest
                )
            })?;

        if !is_manifest_saved {
            storage.write_manifest(&fetched_manifest.digest, &fetched_manifest.bytes)?;
        }

        Ok(fetched_manifest)
    }

    fn fetch_image_manifest(&mut self, reference: &Reference) -> anyhow::Result<FetchedManifest> {
        let media_types = [INDEX_MEDIA_TYPES, MANIFEST_MEDIA_TYPES].concat();
        let fetched = self
            .fetch_manifest_bytes(reference, &media_types)
            .context("failed to fetch manifest bytes")?;
        match fetched.media_type {
            ManifestMediaType::Index => self
                .handle_image_index(reference, &fetched)
                .context("failed to handle image index"),
            ManifestMediaType::Manifest => fetched.parse().context("failed to parse manifest"),
            ManifestMediaType::Unknown => fetched
                .parse()
                .or_else(|_| self.handle_image_index(reference, &fetched))
                .context("failed to parse as manifest and failed to handle as index"),
        }
    }

    fn handle_image_index(
        &mut self,
        reference: &Reference,
        fetched: &ManifestBytes,
    ) -> anyhow::Result<FetchedManifest> {
        let index: ImageIndex =
            serde_json::from_slice(&fetched.bytes).context("failed to parse manifest index")?;
        let descriptor_digest = &index
            .manifests
            .iter()
            .find(|manifest| manifest.matches_current_platform())
            .or_else(|| index.manifests.first())
            .context("manifest index does not contain any manifests")?
            .digest;

        let mut manifest_reference = reference.clone();
        manifest_reference.specifier = Specifier::Digest(descriptor_digest.to_owned());

        self.fetch_manifest_bytes(&manifest_reference, MANIFEST_MEDIA_TYPES)
            .with_context(|| format!("failed to fetch selected manifest {descriptor_digest}"))?
            .parse()
            .with_context(|| format!("failed to parse selected manifest {descriptor_digest}"))
    }

    fn fetch_manifest_bytes(
        &mut self,
        reference: &Reference,
        accepted_media_types: &[&str],
    ) -> anyhow::Result<ManifestBytes> {
        let accept = accepted_media_types.join(", ");
        let specifier = &reference.specifier;
        let path = format!(
            "/v2/{}/manifests/{}",
            reference.repository,
            specifier.as_typeless_str(),
        );
        let response = self
            .call_get_with_auth_retry(&reference.registry, &path, |request| {
                request.header("Accept", accept.as_str())
            })
            .with_context(|| format!("registry request failed: GET {path}"))?;
        let media_type = ManifestMediaType::parse_headers(response.headers());
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
            .with_context(|| format!("failed to read manifest response body from {path}"))?;
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

    fn fetch_blob_reader(
        &mut self,
        reference: &Reference,
        accept: &str,
    ) -> anyhow::Result<impl Read> {
        let digest = reference.specifier.as_typeless_str();
        let path = format!("/v2/{}/blobs/{digest}", reference.repository);
        let response = self
            .call_get_with_auth_retry(&reference.registry, &path, |request| {
                request.header("Accept", accept)
            })
            .with_context(|| format!("registry request failed: GET {path}"))?;

        let (_, body) = response.into_parts();
        Ok(BufReader::new(body.into_reader()))
    }

    fn call_get_with_auth_retry<F>(
        &mut self,
        registry: &str,
        path: &str,
        configure: F,
    ) -> anyhow::Result<ureq::http::Response<ureq::Body>>
    where
        F: Fn(
            ureq::RequestBuilder<ureq::typestate::WithoutBody>,
        ) -> ureq::RequestBuilder<ureq::typestate::WithoutBody>,
    {
        match configure(self.request_get(registry, path)).call() {
            Ok(response) => Ok(response),
            Err(ureq::Error::StatusCode(401)) => {
                let unauthorized = configure(self.agent.get(registry_url(registry, path)))
                    .config()
                    .http_status_as_error(false)
                    .build()
                    .call()
                    .with_context(|| {
                        format!("failed to fetch unauthorized registry response: GET {path}")
                    })?;
                let token = auth::token_from_unauthorized(&self.agent, &unauthorized)
                    .with_context(|| {
                        format!("failed to authenticate registry request: GET {path}")
                    })?;
                self.tokens.insert(registry.to_owned(), token);
                configure(self.request_get(registry, path))
                    .call()
                    .with_context(|| format!("authenticated registry request failed: GET {path}"))
            }
            Err(error) => Err(error.into()),
        }
    }

    fn request_get(
        &self,
        registry: &str,
        path: &str,
    ) -> ureq::RequestBuilder<ureq::typestate::WithoutBody> {
        let request = self.agent.get(registry_url(registry, path));
        match self.tokens.get(registry) {
            Some(token) => request.header("Authorization", format!("Bearer {token}")),
            None => request,
        }
    }

    fn fetch_layers(
        &mut self,
        storage: &StorageMutable,
        reference: &Reference,
        fetched_manifest: &FetchedManifest,
    ) -> anyhow::Result<()> {
        eprintln!(
            "manifest {} has {} layers",
            fetched_manifest.digest,
            fetched_manifest.manifest.layers.len()
        );

        let mut layer_reference = reference.clone();

        for layer in &fetched_manifest.manifest.layers {
            if !LAYER_MEDIA_TYPES.contains(&layer.media_type.as_str()) {
                bail!(
                    "unsupported layer media type `{}` \
                    for manifest manifest layer {}",
                    layer.media_type,
                    layer.digest,
                );
            }

            if storage.get_layer_path(&layer.digest).is_some() {
                eprintln!("layer {} already extracted", layer.digest);
                continue;
            }

            layer_reference.specifier = Specifier::Digest(layer.digest.clone());

            let reader = self
                .fetch_blob_reader(&layer_reference, &layer.media_type)
                .with_context(|| format!("failed to fetch layer {}", layer.digest))?;

            storage
                .write_layer(&layer.digest, reader)
                .with_context(|| format!("failed to fetch and extract layer {}", layer.digest))?;

            eprintln!("extracted {}", layer.digest);
        }

        Ok(())
    }
}

fn registry_url(registry: &str, path: &str) -> String {
    format!("https://{registry}{path}")
}

struct ManifestBytes {
    bytes: Vec<u8>,
    digest: String,
    media_type: ManifestMediaType,
}

impl ManifestBytes {
    fn parse(&self) -> anyhow::Result<FetchedManifest> {
        let manifest: ImageManifest =
            serde_json::from_slice(&self.bytes).context("parsing failed")?;
        if manifest.schema_version != 2 {
            bail!("unsupported image manifest schema");
        }
        Ok(FetchedManifest {
            manifest,
            bytes: self.bytes.clone(),
            digest: self.digest.clone(),
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ManifestMediaType {
    Index,
    Manifest,
    Unknown,
}

impl ManifestMediaType {
    fn parse_headers(headers: &ureq::http::HeaderMap) -> Self {
        let Some(header) = headers.get("Content-Type") else {
            return Self::Unknown;
        };

        let Ok(media_type) = header.to_str() else {
            return Self::Unknown;
        };

        if INDEX_MEDIA_TYPES.iter().any(|t| media_type.contains(t)) {
            return Self::Index;
        }

        if MANIFEST_MEDIA_TYPES.iter().any(|t| media_type.contains(t)) {
            return Self::Manifest;
        }

        Self::Unknown
    }
}
