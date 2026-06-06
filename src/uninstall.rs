use crate::storage::StorageMutable;
use anyhow::Context as _;

pub fn uninstall(storage: &StorageMutable, container: &str) -> anyhow::Result<()> {
    storage
        .remove_container(container)
        .with_context(|| format!("failed to uninstall container `{container}`"))?;

    eprintln!("uninstalled {container}");

    Ok(())
}
