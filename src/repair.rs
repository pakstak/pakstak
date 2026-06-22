use crate::digest;
use crate::fetch::RegistryClient;
use crate::manifest::ImageManifest;
use crate::reference::Specifier;
use crate::storage::StorageMutable;
use anyhow::Context as _;

pub fn repair(storage: &StorageMutable) -> anyhow::Result<()> {
    let mut client = RegistryClient::new();

    for container in storage.read_containers()? {
        let container = container.context("failed to read installed container name")?;
        let manifest_digest = storage
            .read_container_manifest_digest(&container)
            .with_context(|| {
                format!("failed to read manifest digest for container `{container}`")
            })?;
        let mut reference = storage
            .read_container_reference(&container)
            .with_context(|| format!("failed to read reference for container `{container}`"))?;
        reference.specifier = Specifier::Digest(manifest_digest);
        client
            .fetch_image(storage, &reference, true)
            .with_context(|| {
                format!("failed to repair container `{container}` from {reference}")
            })?;
    }

    let mut invalid_manifests = 0;
    let mut missing_layers = 0;

    for manifest_digest in storage.read_manifest_digests()? {
        let manifest_digest = manifest_digest.context("failed to read cached manifest digest")?;
        let manifest_bytes = storage
            .read_manifest_bytes(&manifest_digest)
            .with_context(|| format!("failed to check manifest {manifest_digest}"))?;

        if let Err(error) = digest::verify_bytes(&manifest_bytes, &manifest_digest) {
            eprintln!("manifest {manifest_digest} failed verification: {error}");
            invalid_manifests += 1;
            continue;
        };

        let manifest = match serde_json::from_slice::<ImageManifest>(&manifest_bytes) {
            Ok(manifest) => manifest,
            Err(error) => {
                eprintln!("manifest {manifest_digest} failed parsing: {error}");
                invalid_manifests += 1;
                continue;
            }
        };

        for layer in manifest.layers {
            if storage.get_layer_path(&layer.digest).is_none() {
                eprintln!(
                    "manifest {manifest_digest} references missing layer {}",
                    layer.digest
                );
                missing_layers += 1;
            }
        }
    }

    if invalid_manifests > 0 || missing_layers > 0 {
        anyhow::bail!(
            "repair check found: \
            {invalid_manifests} unused invalid manifests, \
            {missing_layers} unused missing layers; \
            to prune unused manifests and layers use `prune` command"
        );
    }

    eprintln!("repair completed");

    Ok(())
}
