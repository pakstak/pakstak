use crate::fetch::validate_container;
use crate::storage::StorageMutable;
use anyhow::Context as _;

pub fn uninstall(storage: &StorageMutable, container: &str) -> anyhow::Result<()> {
    validate_container(container)?;

    storage
        .remove_container(container)
        .with_context(|| format!("failed to uninstall container `{container}`"))?;

    eprintln!("uninstalled {container}");

    Ok(())
}
