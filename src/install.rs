use crate::fetch::{AppMetadata, ImageRef, fetch_image, validate_alias};
use crate::storage::StorageMutable;
use anyhow::Context as _;

pub fn install(storage: &StorageMutable, alias: &str, image: &str) -> anyhow::Result<()> {
    validate_alias(alias)?;

    if storage.is_app_alias_taken(alias) {
        anyhow::bail!("app alias `{alias}` already exists");
    }

    let image_ref = ImageRef::parse(image)
        .with_context(|| format!("failed to parse image reference `{image}`"))?;
    let fetched_manifest = fetch_image(storage, &image_ref)?;

    let metadata = AppMetadata::from_image(image, &image_ref);
    storage
        .write_app(alias, &fetched_manifest.digest, &metadata)
        .with_context(|| format!("failed to publish app alias `{alias}`"))?;

    eprintln!(
        "installed {image} as {alias} with manifest {}",
        fetched_manifest.digest
    );

    Ok(())
}
