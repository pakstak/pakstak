use crate::fetch::fetch_image;
use crate::storage::StorageMutable;
use anyhow::Context as _;
use std::collections::HashSet;

pub fn update(storage: &StorageMutable, containers: Vec<String>) -> anyhow::Result<()> {
    let mut containers: HashSet<_> = containers.into_iter().collect();

    if containers.is_empty() {
        for container in storage.read_containers()? {
            containers.insert(container.context("failed to read installed container name")?);
        }
    }

    for container in containers {
        storage.ensure_container_installed(&container)?;

        let reference = storage.read_container_reference(&container)?;
        let fetched_manifest = fetch_image(storage, &reference, false).with_context(|| {
            format!("failed to update container `{container}` from {reference}")
        })?;

        let current_manifest = storage.read_container_manifest_digest(&container)?;
        if current_manifest == fetched_manifest.digest {
            eprintln!(
                "{container} is already up to date at {}",
                fetched_manifest.digest
            );
            continue;
        }

        storage
            .write_container_manifest_digest(&container, &fetched_manifest.digest)
            .with_context(|| {
                format!("failed to publish updated manifest for container `{container}`")
            })?;
        eprintln!(
            "updated {container} from {current_manifest} to {}",
            fetched_manifest.digest
        );
    }

    Ok(())
}
