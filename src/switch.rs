use crate::fetch::{fetch_image, validate_container};
use crate::reference::Specifier;
use crate::storage::StorageMutable;
use anyhow::Context as _;

pub fn switch(storage: &StorageMutable, container: &str, digest: &str) -> anyhow::Result<()> {
    validate_container(container)?;

    let mut reference = storage
        .read_container_reference(container)
        .with_context(|| format!("failed to read reference for container `{container}`"))?;
    reference.specifier = Specifier::Digest(digest.to_owned());

    fetch_image(storage, &reference, false).with_context(|| {
        format!("failed to fetch manifest {digest} for container `{container}`")
    })?;
    storage
        .write_container_manifest_digest(container, digest)
        .with_context(|| {
            format!("failed to publish switched manifest for container `{container}`")
        })?;

    eprintln!("switched {container} to {digest}");

    Ok(())
}
