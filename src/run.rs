use crate::context::{Context, LockMode};
use crate::manifest::ImageManifest;
use anyhow::{Context as _, bail};
use std::fs;
use std::os::unix::process::CommandExt;
use std::process::Command;

pub fn run(ctx: &Context, alias: &str, command: Vec<String>) -> anyhow::Result<()> {
    let _lock = ctx.acquire_lock(LockMode::Shared)?;

    if command.is_empty() {
        bail!("run command cannot be empty");
    }

    let app_manifest_path = ctx.storage_path.join("apps").join(alias).join("manifest");
    let manifest_digest = fs::read_to_string(&app_manifest_path).with_context(|| {
        format!(
            "failed to read app manifest {}",
            app_manifest_path.display()
        )
    })?;
    let manifest_digest = manifest_digest.trim();
    if manifest_digest.is_empty() {
        bail!("app manifest {} is empty", app_manifest_path.display());
    }

    let manifest_path = ctx
        .storage_path
        .join("manifests")
        .join(format!("{manifest_digest}.json"));
    let manifest_bytes = fs::read(&manifest_path)
        .with_context(|| format!("failed to read manifest {}", manifest_path.display()))?;
    let manifest: ImageManifest = serde_json::from_slice(&manifest_bytes)
        .with_context(|| format!("failed to parse manifest {}", manifest_path.display()))?;

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
        let layer_path = ctx.storage_path.join("layers").join(&layer.digest);
        if !layer_path.is_dir() {
            bail!(
                "layer {} is missing at {}; install the image first",
                layer.digest,
                layer_path.display()
            );
        }

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
