use crate::context::{Context, LockMode};
use crate::fetch::{AppMetadata, ImageRef, fetch_image, validate_alias};
use anyhow::Context as _;
use std::fs;
use std::path::Path;

pub fn install(ctx: &Context, alias: &str, image: &str) -> anyhow::Result<()> {
    validate_alias(alias)?;
    let _lock = ctx.acquire_lock(LockMode::Exclusive)?;

    let app_dir = ctx.storage_path.join("apps").join(alias);
    if app_dir.exists() {
        anyhow::bail!(
            "app alias `{alias}` already exists at {}",
            app_dir.display()
        );
    }

    let image_ref = ImageRef::parse(image)
        .with_context(|| format!("failed to parse image reference `{image}`"))?;
    let fetched_manifest = fetch_image(ctx, &image_ref)?;

    publish_app(ctx, &app_dir, image, &image_ref, &fetched_manifest.digest).with_context(|| {
        format!(
            "failed to publish app alias `{alias}` to {}",
            app_dir.display()
        )
    })?;

    eprintln!(
        "installed {image} as {alias} with manifest {}",
        fetched_manifest.digest
    );

    Ok(())
}

fn publish_app(
    ctx: &Context,
    app_dir: &Path,
    image: &str,
    image_ref: &ImageRef,
    manifest_digest: &str,
) -> anyhow::Result<()> {
    let temporary_app_dir = ctx.temporary_directory_for(app_dir)?;
    fs::create_dir_all(&temporary_app_dir).with_context(|| {
        format!(
            "failed to create temporary app directory {}",
            temporary_app_dir.display()
        )
    })?;

    fs::write(temporary_app_dir.join("manifest"), manifest_digest).with_context(|| {
        format!(
            "failed to write temporary app manifest file in {}",
            temporary_app_dir.display()
        )
    })?;

    let metadata = AppMetadata::from_image(image, image_ref);
    let metadata =
        serde_json::to_vec_pretty(&metadata).context("failed to serialize app metadata")?;
    fs::write(temporary_app_dir.join("metadata.json"), metadata).with_context(|| {
        format!(
            "failed to write temporary app metadata file in {}",
            temporary_app_dir.display()
        )
    })?;

    ctx.publish_directory(&temporary_app_dir, app_dir)
}
