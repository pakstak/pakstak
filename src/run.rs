use crate::storage::Storage;
use anyhow::{Context as _, bail};
use std::os::unix::process::CommandExt;
use std::process::Command;

pub fn run(storage: &Storage, container: &str, command: Vec<String>) -> anyhow::Result<()> {
    if command.is_empty() {
        bail!("run command cannot be empty");
    }

    let manifest_digest = storage.read_container_manifest_digest(container)?;
    let manifest = storage.read_manifest(&manifest_digest)?;

    let mut bwrap = Command::new("bwrap");
    bwrap
        .arg("--clearenv")
        .arg("--unshare-all")
        .arg("--share-net")
        .arg("--proc")
        .arg("/proc")
        .arg("--dev")
        .arg("/dev")
        .arg("--tmpfs")
        .arg("/tmp");

    let mut layer_paths = Vec::with_capacity(manifest.layers.len());
    for layer in &manifest.layers {
        let layer_path = storage.get_layer_path(&layer.digest).with_context(|| {
            format!("layer {} is missing; install the image first", layer.digest)
        })?;

        layer_paths.push(layer_path);
    }

    match layer_paths.as_slice() {
        [] => bail!("manifest does not contain any layers"),
        [layer_path] => {
            bwrap.arg("--ro-bind").arg(layer_path).arg("/");
        }
        layer_paths => {
            for layer_path in layer_paths {
                bwrap.arg("--overlay-src").arg(layer_path);
            }
            bwrap.arg("--ro-overlay").arg("/");
        }
    }

    bwrap.args(command);

    Err(bwrap.exec()).context("failed to replace process with bwrap")
}
