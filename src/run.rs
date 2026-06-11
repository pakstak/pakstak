use crate::manifest::Descriptor;
use crate::storage::Storage;
use anyhow::{Context as _, bail};
use std::os::unix::process::CommandExt;
use std::process::Command;

pub fn run(storage: &Storage, container: &str, command: Vec<String>) -> anyhow::Result<()> {
    let mut bwrap = build_bwrap_command(storage, container, command)?;

    Err(bwrap.exec()).context("failed to replace process with bwrap")
}

fn build_bwrap_command(
    storage: &Storage,
    container: &str,
    command: Vec<String>,
) -> anyhow::Result<Command> {
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

    Ok(bwrap)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::tests::storage_in;
    use std::ffi::OsString;
    use std::fs;
    use temp_dir::TempDir;

    #[test]
    fn bwrap_locks_each_layer_inside_sandbox() {
        let temp_dir = TempDir::new().unwrap();
        let storage_path = temp_dir.path().join("storage");
        let storage = storage_in(&temp_dir).unwrap();

        fs::create_dir_all(storage_path.join("containers").join("test")).unwrap();
        fs::write(
            storage_path
                .join("containers")
                .join("test")
                .join("manifest_digest"),
            "sha256:manifest",
        )
        .unwrap();
        fs::create_dir_all(storage_path.join("manifests")).unwrap();
        fs::write(
            storage_path.join("manifests").join("sha256:manifest.json"),
            "{\"schemaVersion\":2,\"layers\":[{\"digest\":\"sha256:one\"},{\"digest\":\"sha256:two\"}]}",
        )
        .unwrap();
        fs::create_dir_all(storage_path.join("layers").join("sha256:one")).unwrap();
        fs::create_dir_all(storage_path.join("layers").join("sha256:two")).unwrap();

        let bwrap = build_bwrap_command(&storage, "test", vec!["/bin/sh".to_string()]).unwrap();
        let args: Vec<OsString> = bwrap.get_args().map(OsString::from).collect();
        let first_lock_mount = args
            .iter()
            .enumerate()
            .position(|(index, arg)| {
                arg == "--ro-bind"
                    && args.get(index + 2) == Some(&OsString::from("/tmp/.layer_locks/0"))
            })
            .expect("bwrap args should mount layer locks inside the sandbox");

        assert_eq!(
            &args[first_lock_mount..],
            &[
                OsString::from("--ro-bind"),
                storage_path
                    .join("locks")
                    .join("layers")
                    .join("sha256:one")
                    .into(),
                OsString::from("/tmp/.layer_locks/0"),
                OsString::from("--lock-file"),
                OsString::from("/tmp/.layer_locks/0"),
                OsString::from("--ro-bind"),
                storage_path
                    .join("locks")
                    .join("layers")
                    .join("sha256:two")
                    .into(),
                OsString::from("/tmp/.layer_locks/1"),
                OsString::from("--lock-file"),
                OsString::from("/tmp/.layer_locks/1"),
                OsString::from("/bin/sh"),
            ]
        );
    }
}
