use crate::manifest::Descriptor;
use crate::storage::Storage;
use anyhow::{Context as _, bail};
use std::os::unix::process::CommandExt;
use std::process::Command;

const LAYER_LOCKS_DIR: &str = "/tmp/.layer_locks";

pub fn run(storage: &Storage, containers: Vec<String>, command: Vec<String>) -> anyhow::Result<()> {
    let mut bwrap = build_bwrap_command(storage, containers, command)?;

    Err(bwrap.exec()).context("failed to replace process with bwrap")
}

fn build_bwrap_command(
    storage: &Storage,
    containers: Vec<String>,
    command: Vec<String>,
) -> anyhow::Result<Command> {
    if containers.is_empty() {
        bail!("at least one container required");
    }

    if command.is_empty() {
        bail!("command cannot be empty");
    }

    let layers = read_stacked_layers(storage, &containers)?;

    let mut bwrap = Command::new("bwrap");

    bwrap.arg("--clearenv").arg("--unshare-all");

    let get_layer_path = |layer: &Descriptor| {
        storage
            .get_layer_path(&layer.digest)
            .with_context(|| format!("layer {} is missing; install the image first", layer.digest))
    };

    let layer_lock_paths = layers
        .iter()
        .enumerate()
        .map(|(index, layer)| {
            Ok((
                storage.create_layer_lock_file(&layer.digest)?,
                format!("{LAYER_LOCKS_DIR}/{index}"),
            ))
        })
        .collect::<anyhow::Result<Vec<_>>>()?;

    match layers.len() {
        0 => bail!("selected containers have no layers"),
        1 => {
            bwrap
                .arg("--ro-bind")
                .arg(get_layer_path(&layers[0])?)
                .arg("/");
        }
        _ => {
            for layer in &layers {
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

fn read_stacked_layers(
    storage: &Storage,
    containers: &[String],
) -> anyhow::Result<Vec<Descriptor>> {
    let mut layers = Vec::new();

    for container in containers {
        storage.ensure_container_installed(container)?;

        let manifest_digest = storage.read_container_manifest_digest(container)?;
        let manifest = storage.read_manifest(&manifest_digest)?;

        // We cannot stack same layer twice - overlayfs layers must not overlap
        for new_layer in manifest.layers {
            if layers
                .iter()
                .all(|old_layer: &Descriptor| new_layer.digest != old_layer.digest)
            {
                layers.push(new_layer)
            }
        }
    }

    Ok(layers)
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

        write_container(
            &storage_path,
            "test",
            "sha256:manifest",
            &["sha256:one", "sha256:two"],
        );

        let bwrap = build_bwrap_command(
            &storage,
            vec!["test".to_string()],
            vec!["/bin/sh".to_string()],
        )
        .unwrap();
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

    #[test]
    fn bwrap_stacks_containers_in_cli_order() {
        let temp_dir = TempDir::new().unwrap();
        let storage_path = temp_dir.path().join("storage");
        let storage = storage_in(&temp_dir).unwrap();

        write_container(&storage_path, "base", "sha256:base", &["sha256:base-layer"]);
        write_container(
            &storage_path,
            "runtime",
            "sha256:runtime",
            &["sha256:runtime-layer-one", "sha256:runtime-layer-two"],
        );
        write_container(&storage_path, "app", "sha256:app", &["sha256:app-layer"]);

        let bwrap = build_bwrap_command(
            &storage,
            vec!["base".to_string(), "runtime".to_string(), "app".to_string()],
            vec!["/bin/sh".to_string()],
        )
        .unwrap();
        let args: Vec<OsString> = bwrap.get_args().map(OsString::from).collect();
        let overlay_sources: Vec<OsString> = args
            .windows(2)
            .filter(|window| window[0] == "--overlay-src")
            .map(|window| window[1].clone())
            .collect();

        let expected_overlay_sources: Vec<OsString> = [
            "sha256:base-layer",
            "sha256:runtime-layer-one",
            "sha256:runtime-layer-two",
            "sha256:app-layer",
        ]
        .into_iter()
        .map(|digest| storage_path.join("layers").join(digest).into())
        .collect();

        assert_eq!(overlay_sources, expected_overlay_sources);
    }

    fn write_container(
        storage_path: &std::path::Path,
        container: &str,
        manifest_digest: &str,
        layer_digests: &[&str],
    ) {
        fs::create_dir_all(storage_path.join("containers").join(container)).unwrap();
        fs::write(
            storage_path
                .join("containers")
                .join(container)
                .join("manifest_digest"),
            manifest_digest,
        )
        .unwrap();
        fs::create_dir_all(storage_path.join("manifests")).unwrap();

        let layers = layer_digests
            .iter()
            .map(|digest| format!("{{\"digest\":\"{digest}\"}}"))
            .collect::<Vec<_>>()
            .join(",");
        fs::write(
            storage_path
                .join("manifests")
                .join(format!("{manifest_digest}.json")),
            format!("{{\"schemaVersion\":2,\"layers\":[{layers}]}}"),
        )
        .unwrap();

        for layer_digest in layer_digests {
            fs::create_dir_all(storage_path.join("layers").join(layer_digest)).unwrap();
        }
    }
}
