use crate::fetch::{fetch_image, validate_container};
use crate::reference::Reference;
use crate::storage::StorageMutable;
use anyhow::Context as _;

pub fn install(storage: &StorageMutable, container: &str, image: &str) -> anyhow::Result<()> {
    validate_container(container)?;

    if storage.is_container_taken(container) {
        anyhow::bail!("container `{container}` already exists");
    }

    let reference = Reference::parse(image)
        .with_context(|| format!("failed to parse image reference `{image}`"))?;
    let fetched_manifest = fetch_image(storage, &reference, false)?;

    storage
        .write_container(container, &fetched_manifest.digest, &reference)
        .with_context(|| format!("failed to publish container `{container}`"))?;

    eprintln!(
        "installed {image} as {container} with manifest {}",
        fetched_manifest.digest
    );

    Ok(())
}
