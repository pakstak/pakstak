use crate::fetch::fetch_image;
use crate::reference::Reference;
use crate::storage::StorageMutable;
use anyhow::{Context as _, bail};

pub fn install(storage: &StorageMutable, container: &str, image: &str) -> anyhow::Result<()> {
    validate_container_name(container)?;

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

pub fn validate_container_name(container: &str) -> anyhow::Result<()> {
    if container.is_empty() {
        bail!("container name cannot be empty");
    }
    if container == "." || container == ".." {
        bail!("container name `{container}` is not allowed");
    }
    if !container
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
    {
        bail!(
            "container name `{container}` contains invalid characters; use only ASCII letters, numbers, dots, underscores, and dashes"
        );
    }
    if !container
        .bytes()
        .next()
        .is_some_and(|byte| byte.is_ascii_alphanumeric())
    {
        bail!("container name `{container}` must start with an ASCII letter or number");
    }

    Ok(())
}
