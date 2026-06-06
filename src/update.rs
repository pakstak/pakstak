use crate::fetch::{ImageRef, fetch_image, validate_alias};
use crate::storage::StorageMutable;
use anyhow::Context as _;
use std::collections::HashSet;

pub fn update(storage: &StorageMutable, aliases: Vec<String>) -> anyhow::Result<()> {
    let mut aliases: HashSet<_> = aliases.into_iter().collect();

    for alias in aliases.iter() {
        validate_alias(alias)?;
    }

    if aliases.is_empty() {
        aliases.extend(storage.read_app_aliases()?);
    }

    for alias in aliases {
        if !storage.is_app_alias_taken(&alias) {
            continue;
        }

        let metadata = storage.read_app_metadata(&alias)?;
        let image_ref = ImageRef::from_metadata(&metadata);
        let fetched_manifest = fetch_image(storage, &image_ref).with_context(|| {
            format!(
                "failed to update app alias `{alias}` from image {}",
                metadata.source.image
            )
        })?;

        let current_manifest = storage.read_app_manifest_digest(&alias)?;
        if current_manifest == fetched_manifest.digest {
            eprintln!(
                "{alias} is already up to date at {}",
                fetched_manifest.digest
            );
            continue;
        }

        storage
            .write_app_manifest_digest(&alias, &fetched_manifest.digest)
            .with_context(|| {
                format!("failed to publish updated manifest for app alias `{alias}`")
            })?;
        eprintln!(
            "updated {alias} from {current_manifest} to {}",
            fetched_manifest.digest
        );
    }

    Ok(())
}
