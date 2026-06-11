use crate::manifest::Descriptor;
use crate::storage::Storage;
use anyhow::{Context as _, bail};
use std::os::unix::process::CommandExt;
use std::process::Command;

pub fn run(storage: &Storage, container: &str, command: Vec<String>) -> anyhow::Result<()> {
    if command.is_empty() {
        bail!("run command cannot be empty");
    }

    storage.ensure_container_installed(container)?;

    let manifest_digest = storage.read_container_manifest_digest(container)?;
    let manifest = storage.read_manifest(&manifest_digest)?;

    let mut bwrap = Command::new("bwrap");

    bwrap.arg("--clearenv").arg("--unshare-all");

    const LAYER_LOCKS_DIR: &str = "/tmp/.layer_locks";

    let get_layer_path = |layer: &Descriptor| {
        storage
            .get_layer_path(&layer.digest)
            .with_context(|| format!("layer {} is missing; install the image first", layer.digest))
    };

    let layer_lock_paths = manifest
        .layers
        .iter()
        .enumerate()
        .map(|(index, layer)| {
            Ok((
                storage.create_layer_lock_file(&layer.digest)?,
                format!("{LAYER_LOCKS_DIR}/{index}"),
            ))
        })
        .collect::<anyhow::Result<Vec<_>>>()?;

    match manifest.layers.len() {
        0 => bail!("manifest does not contain any layers"),
        1 => {
            bwrap
                .arg("--ro-bind")
                .arg(get_layer_path(&manifest.layers[0])?)
                .arg("/");
        }
        _ => {
            for layer in &manifest.layers {
                bwrap.arg("--overlay-src").arg(get_layer_path(layer)?);
            }
            bwrap.arg("--ro-overlay").arg("/");
        }
    }

    bwrap
        .arg("--proc")
        .arg("/proc")
        .arg("--dev")
        .arg("/dev")
        .arg("--tmpfs")
        .arg("/tmp")
        .arg("--dir")
        .arg(LAYER_LOCKS_DIR);

    for (host_path, sandbox_path) in &layer_lock_paths {
        bwrap
            .arg("--ro-bind")
            .arg(host_path)
            .arg(sandbox_path)
            .arg("--lock-file")
            .arg(sandbox_path);
    }

    bwrap.args(command);

    Err(bwrap.exec()).context("failed to replace process with bwrap")
}
