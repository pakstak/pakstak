use crate::storage::{LayerLockResult, StorageMutable};
use anyhow::Context as _;
use std::collections::HashSet;

pub fn prune(storage: &StorageMutable) -> anyhow::Result<()> {
    let used_manifests = read_used_manifests(storage)?;
    let used_layers = read_used_layers(storage, &used_manifests)?;

    let removed_manifests = prune_manifests(storage, &used_manifests)?;
    let removed_layers = prune_layers(storage, &used_layers)?;
    cleanup_layer_locks(storage)?;

    eprintln!("pruned {removed_manifests} manifests and {removed_layers} layers");

    Ok(())
}

fn read_used_manifests(storage: &StorageMutable) -> anyhow::Result<HashSet<String>> {
    storage
        .read_containers()?
        .map(|container| {
            let container = container.context("failed to read installed container name")?;
            storage
                .read_container_manifest_digest(&container)
                .with_context(|| {
                    format!("failed to read manifest digest for container `{container}`")
                })
        })
        .collect()
}

fn read_used_layers(
    storage: &StorageMutable,
    used_manifests: &HashSet<String>,
) -> anyhow::Result<HashSet<String>> {
    let mut used_layers = HashSet::new();
    for manifest_digest in used_manifests {
        let manifest = storage
            .read_manifest(manifest_digest)
            .with_context(|| format!("failed to read used manifest {manifest_digest}"))?;
        used_layers.extend(manifest.layers.into_iter().map(|layer| layer.digest));
    }
    Ok(used_layers)
}

fn prune_manifests(
    storage: &StorageMutable,
    used_manifests: &HashSet<String>,
) -> anyhow::Result<usize> {
    storage
        .read_manifest_digests()?
        .try_fold(0, |removed, manifest_digest| {
            let manifest_digest =
                manifest_digest.context("failed to read cached manifest digest")?;
            if used_manifests.contains(&manifest_digest) {
                return Ok(removed);
            }

            storage
                .remove_manifest(&manifest_digest)
                .with_context(|| format!("failed to prune manifest {manifest_digest}"))?;
            Ok(removed + 1)
        })
}

fn prune_layers(storage: &StorageMutable, used_layers: &HashSet<String>) -> anyhow::Result<usize> {
    storage
        .read_layer_digests()?
        .try_fold(0, |removed, layer_digest| {
            let layer_digest = layer_digest.context("failed to read cached layer digest")?;
            if used_layers.contains(&layer_digest) {
                return Ok(removed);
            }

            match storage.lock_layer_for_prune(&layer_digest) {
                LayerLockResult::Acquired(layer_lock) => {
                    if let Err(error) = storage
                        .remove_layer(&layer_digest)
                        .with_context(|| format!("failed to prune layer {layer_digest}"))
                        .and_then(|_| layer_lock.remove_file())
                    {
                        eprintln!("failed to prune layer {layer_digest}: {error:#}");
                        return Ok(removed);
                    }
                    Ok(removed + 1)
                }
                LayerLockResult::Failed => {
                    eprintln!("failed to prune layer {layer_digest}: layer is locked");
                    Ok(removed)
                }
                LayerLockResult::Error(error) => {
                    eprintln!("failed to prune layer {layer_digest}: {error:#}");
                    Ok(removed)
                }
            }
        })
}

fn cleanup_layer_locks(storage: &StorageMutable) -> anyhow::Result<()> {
    for layer_digest in storage.read_layer_lock_digests()? {
        let layer_digest = layer_digest.context("failed to read layer lock digest")?;
        if storage.get_layer_path(&layer_digest).is_some() {
            continue;
        }

        match storage.lock_layer_for_prune(&layer_digest) {
            LayerLockResult::Acquired(layer_lock) => {
                if let Err(error) = layer_lock
                    .remove_file()
                    .with_context(|| format!("failed to remove stale layer lock {layer_digest}"))
                {
                    eprintln!("failed to remove stale layer lock {layer_digest}: {error:#}");
                }
            }
            LayerLockResult::Failed => {}
            LayerLockResult::Error(error) => {
                eprintln!("failed to remove stale layer lock {layer_digest}: {error:#}");
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::tests::storage_mutable_in;
    use std::fs::{self, OpenOptions};
    use std::os::unix::io::AsRawFd;
    use std::path::PathBuf;
    use std::process::Command;
    use temp_dir::TempDir;

    const STORAGE_ENV: &str = "PAKSTAK_PRUNE_LOCK_TEST_STORAGE";

    #[test]
    fn prune_preserves_layers_for_active_container_manifest() {
        const ACTIVE_MANIFEST: &str = "sha256:active-manifest";
        const UNUSED_MANIFEST: &str = "sha256:unused-manifest";
        const ACTIVE_LAYERS: [&str; 2] = ["sha256:active-layer-one", "sha256:active-layer-two"];
        const UNUSED_LAYERS: [&str; 2] = ["sha256:unused-layer-one", "sha256:unused-layer-two"];

        let temp_dir = TempDir::new().unwrap();
        let storage = storage_mutable_in(&temp_dir).unwrap();
        let storage_path = temp_dir.path().join("storage");
        let container_path = storage_path.join("containers").join("active-container");
        let manifests_path = storage_path.join("manifests");
        let layers_path = storage_path.join("layers");

        fs::create_dir_all(&container_path).unwrap();
        fs::write(container_path.join("manifest_digest"), ACTIVE_MANIFEST).unwrap();
        fs::create_dir_all(&manifests_path).unwrap();
        fs::write(
            manifests_path.join(format!("{ACTIVE_MANIFEST}.json")),
            format!(
                "{{\"schemaVersion\":2,\"layers\":[{}]}}",
                ACTIVE_LAYERS
                    .map(|digest| format!(
                        "{{\"digest\":\"{digest}\",\"mediaType\":\"application/vnd.oci.image.layer.v1.tar\"}}"
                    ))
                    .join(",")
            ),
        )
        .unwrap();
        fs::write(
            manifests_path.join(format!("{UNUSED_MANIFEST}.json")),
            b"{\"schemaVersion\":2,\"layers\":[]}",
        )
        .unwrap();
        for digest in ACTIVE_LAYERS.into_iter().chain(UNUSED_LAYERS) {
            fs::create_dir_all(layers_path.join(digest)).unwrap();
        }

        prune(&storage).unwrap();

        assert!(
            manifests_path
                .join(format!("{ACTIVE_MANIFEST}.json"))
                .is_file()
        );
        assert!(
            !manifests_path
                .join(format!("{UNUSED_MANIFEST}.json"))
                .exists()
        );
        for digest in ACTIVE_LAYERS {
            assert!(
                layers_path.join(digest).is_dir(),
                "prune removed layer {digest} referenced by the active container manifest"
            );
        }
        for digest in UNUSED_LAYERS {
            assert!(
                !layers_path.join(digest).exists(),
                "prune retained unused layer {digest}"
            );
        }
    }

    #[test]
    fn prune_ignores_locked_layers() {
        const LOCKED_LAYER_DIGESTS: [&str; 2] =
            ["sha256:locked-layer-one", "sha256:locked-layer-two"];
        const FREE_LAYER_DIGESTS: [&str; 2] = ["sha256:free-layer-one", "sha256:free-layer-two"];

        let temp_dir = TempDir::new().unwrap();
        let storage_path = temp_dir.path().join("storage");
        let layers_path = storage_path.join("layers");
        let layer_locks_path = storage_path.join("locks").join("layers");
        fs::create_dir_all(&layer_locks_path).unwrap();

        for digest in LOCKED_LAYER_DIGESTS.into_iter().chain(FREE_LAYER_DIGESTS) {
            fs::create_dir_all(layers_path.join(digest)).unwrap();
        }

        let lock = libc::flock {
            l_type: libc::F_RDLCK as libc::c_short,
            l_whence: libc::SEEK_SET as libc::c_short,
            l_start: 0,
            l_len: 0,
            l_pid: 0,
        };
        let lock_files = LOCKED_LAYER_DIGESTS.map(|digest| {
            let lock_file = OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .truncate(false)
                .open(layer_locks_path.join(digest))
                .unwrap();
            assert_eq!(
                unsafe { libc::fcntl(lock_file.as_raw_fd(), libc::F_SETLK, &lock) },
                0
            );
            lock_file
        });

        let output = Command::new(std::env::current_exe().unwrap())
            .arg("--exact")
            .arg("prune::tests::prune_locked_layers_child")
            .arg("--ignored")
            .env(STORAGE_ENV, temp_dir.path())
            .output()
            .unwrap();

        assert!(
            output.status.success(),
            "child test failed with status {}\nstdout:\n{}\nstderr:\n{}",
            output.status,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(lock_files.len(), LOCKED_LAYER_DIGESTS.len());
        for digest in LOCKED_LAYER_DIGESTS {
            assert!(
                layers_path.join(digest).is_dir(),
                "prune removed locked layer {digest}"
            );
            assert!(
                layer_locks_path.join(digest).is_file(),
                "prune removed active lock file for {digest}"
            );
        }
        for digest in FREE_LAYER_DIGESTS {
            assert!(
                !layers_path.join(digest).exists(),
                "prune retained unlocked layer {digest}"
            );
            assert!(
                !layer_locks_path.join(digest).exists(),
                "prune retained lock file for unlocked layer {digest}"
            );
        }
    }

    #[test]
    #[ignore = "helper test run by prune_ignores_locked_layers"]
    fn prune_locked_layers_child() {
        let Some(storage_parent) = std::env::var_os(STORAGE_ENV) else {
            return;
        };
        let storage = storage_mutable_in(PathBuf::from(storage_parent)).unwrap();

        prune(&storage).unwrap();
    }
}
