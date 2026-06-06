use crate::manifest::Descriptor;
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

    bwrap.arg("--clearenv").arg("--unshare-all");

    let get_layer_path = |layer: &Descriptor| {
        storage
            .get_layer_path(&layer.digest)
            .with_context(|| format!("layer {} is missing; install the image first", layer.digest))
    };

    match manifest.layers.len() {
        0 => bail!("manifest does not contain any layers"),
        1 => {
            bwrap
                .arg("--ro-bind")
                .arg(get_layer_path(&manifest.layers[0])?)
                .arg("/");
        }
        _ => {
            for layer in manifest.layers {
                bwrap.arg("--overlay-src").arg(get_layer_path(&layer)?);
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
        .arg("/tmp");

    bwrap.args(command);

    Err(bwrap.exec()).context("failed to replace process with bwrap")
}
