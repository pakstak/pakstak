use crate::storage::StorageMutable;
use anyhow::Context as _;
use std::collections::HashSet;

pub fn prune(storage: &StorageMutable) -> anyhow::Result<()> {
    let used_manifests = read_used_manifests(storage)?;
    let used_layers = read_used_layers(storage, &used_manifests)?;

    let removed_manifests = prune_manifests(storage, &used_manifests)?;
    let removed_layers = prune_layers(storage, &used_layers)?;

    eprintln!("pruned {removed_manifests} manifests and {removed_layers} layers");

    Ok(())
}

fn read_used_manifests(storage: &StorageMutable) -> anyhow::Result<HashSet<String>> {
    let mut used_manifests = HashSet::new();
    for container in storage.read_containers()? {
        let manifest_digest = storage
            .read_container_manifest_digest(&container)
            .with_context(|| {
                format!("failed to read manifest digest for container `{container}`")
            })?;
        used_manifests.insert(manifest_digest);
    }
    Ok(used_manifests)
}

fn read_used_layers(
    storage: &StorageMutable,
    used_manifests: &HashSet<String>,
) -> anyhow::Result<HashSet<String>> {
    let mut used_layers = HashSet::new();
    for manifest_digest in used_manifests {
        let manifest = storage
            .read_manifest(manifest_digest)
            .with_context(|| format!("failed to read used manifest {manifest_digest}"))?;
        used_layers.extend(manifest.layers.into_iter().map(|layer| layer.digest));
    }
    Ok(used_layers)
}

fn prune_manifests(
    storage: &StorageMutable,
    used_manifests: &HashSet<String>,
) -> anyhow::Result<usize> {
    let mut removed = 0;
    for manifest_digest in storage.read_manifest_digests()? {
        if used_manifests.contains(&manifest_digest) {
            continue;
        }

        storage
            .remove_manifest(&manifest_digest)
            .with_context(|| format!("failed to prune manifest {manifest_digest}"))?;
        removed += 1;
    }
    Ok(removed)
}

fn prune_layers(storage: &StorageMutable, used_layers: &HashSet<String>) -> anyhow::Result<usize> {
    let mut removed = 0;
    for layer_digest in storage.read_layer_digests()? {
        if used_layers.contains(&layer_digest) {
            continue;
        }

        storage
            .remove_layer(&layer_digest)
            .with_context(|| format!("failed to prune layer {layer_digest}"))?;
        removed += 1;
    }
    Ok(removed)
}
