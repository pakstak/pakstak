use crate::context::{Context, LockMode};
use crate::fetch::{ImageRef, fetch_image, validate_alias};
use anyhow::Context as _;
use std::collections::HashSet;
use std::fs;

pub fn update(ctx: &Context, aliases: Vec<String>) -> anyhow::Result<()> {
    let _lock = ctx.acquire_lock(LockMode::Exclusive)?;
    let apps_dir = ctx.storage_path.join("apps");

    let mut aliases: HashSet<_> = aliases.into_iter().collect();

    for alias in aliases.iter() {
        validate_alias(alias)?;
    }

    if aliases.is_empty() {
        for entry in fs::read_dir(&apps_dir)
            .with_context(|| format!("failed to read apps directory {}", apps_dir.display()))?
        {
            let entry = entry.with_context(|| {
                format!(
                    "failed to read apps directory entry under {}",
                    apps_dir.display()
                )
            })?;

            let alias = entry
                .file_name()
                .into_string()
                .map_err(|name| anyhow::anyhow!("app alias {:?} is not valid UTF-8", name))?;

            aliases.insert(alias);
        }
    }

    for alias in aliases {
        let app_dir = apps_dir.join(&alias);

        if !app_dir.is_dir() {
            continue;
        }

        let metadata_path = app_dir.join("metadata.json");
        let metadata = fs::read(&metadata_path)
            .with_context(|| format!("failed to read metadata {}", metadata_path.display()))?;
        let metadata = serde_json::from_slice(&metadata)
            .with_context(|| format!("failed to parse metadata {}", metadata_path.display()))?;

        let image_ref = ImageRef::from_metadata(&metadata);
        let fetched_manifest = fetch_image(ctx, &image_ref).with_context(|| {
            format!(
                "failed to update app alias `{alias}` from image {}",
                metadata.source.image
            )
        })?;

        let manifest_path = app_dir.join("manifest");
        let current_manifest = fs::read_to_string(&manifest_path).with_context(|| {
            format!(
                "failed to read app manifest digest {}",
                manifest_path.display()
            )
        })?;
        if current_manifest == fetched_manifest.digest {
            eprintln!(
                "{alias} is already up to date at {}",
                fetched_manifest.digest
            );
            continue;
        }

        ctx.atomic_write(&manifest_path, fetched_manifest.digest.as_bytes())
            .with_context(|| {
                format!(
                    "failed to publish updated manifest for app alias `{alias}` to {}",
                    manifest_path.display()
                )
            })?;
        eprintln!(
            "updated {alias} from {current_manifest} to {}",
            fetched_manifest.digest
        );
    }

    Ok(())
}
