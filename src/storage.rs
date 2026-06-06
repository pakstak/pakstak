use crate::digest::DigestVerifier;
use crate::manifest::ImageManifest;
use crate::reference::Reference;
use anyhow::Context as _;
use flate2::read::GzDecoder;
use std::env;
use std::fs::{self, File};
use std::io::{self, BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use tar::Archive;

#[derive(Debug)]
pub struct Storage {
    storage_path: PathBuf,
    temporary_path: PathBuf,
    _lock: StorageLock,
}

#[derive(Debug)]
pub struct StorageMutable {
    storage: Storage,
}

impl Storage {
    pub fn new() -> anyhow::Result<Self> {
        Self::new_with_lock(LockMode::Shared)
    }

    fn new_with_lock(lock_mode: LockMode) -> anyhow::Result<Self> {
        let home_dir = env::var_os("HOME")
            .map(PathBuf::from)
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "HOME is not set"))?;
        let storage_path = home_dir.join(".var").join("pakstak");
        let temporary_path = storage_path.join("temporary");

        let _lock = acquire_lock(&storage_path, lock_mode)?;

        Ok(Self {
            temporary_path,
            storage_path,
            _lock,
        })
    }

    pub fn read_container_manifest_digest(&self, container: &str) -> anyhow::Result<String> {
        let manifest_digest_path = self.container_path(container).join("manifest_digest");
        let manifest_digest = fs::read_to_string(&manifest_digest_path).with_context(|| {
            format!(
                "failed to read container manifest digest {}",
                manifest_digest_path.display()
            )
        })?;
        let manifest_digest = manifest_digest.trim().to_string();
        if manifest_digest.is_empty() {
            anyhow::bail!(
                "container manifest digest {} is empty",
                manifest_digest_path.display()
            );
        }
        Ok(manifest_digest)
    }

    pub fn read_manifest(&self, digest: &str) -> anyhow::Result<ImageManifest> {
        let manifest_path = self.manifest_path(digest);
        let manifest_bytes = fs::read(&manifest_path)
            .with_context(|| format!("failed to read manifest {}", manifest_path.display()))?;
        serde_json::from_slice(&manifest_bytes)
            .with_context(|| format!("failed to parse manifest {}", manifest_path.display()))
    }

    pub fn read_container_reference(&self, container: &str) -> anyhow::Result<Reference> {
        let reference_path = self.container_path(container).join("reference.json");
        let reference = fs::read(&reference_path)
            .with_context(|| format!("failed to read reference {}", reference_path.display()))?;
        serde_json::from_slice(&reference)
            .with_context(|| format!("failed to parse reference {}", reference_path.display()))
    }

    pub fn read_containers(&self) -> anyhow::Result<Vec<String>> {
        let containers_path = self.containers_path();
        let mut containers = Vec::new();
        for entry in fs::read_dir(&containers_path).with_context(|| {
            format!(
                "failed to read containers directory {}",
                containers_path.display()
            )
        })? {
            let entry = entry.with_context(|| {
                format!(
                    "failed to read containers directory entry under {}",
                    containers_path.display()
                )
            })?;
            let container = entry
                .file_name()
                .into_string()
                .map_err(|name| anyhow::anyhow!("container name {:?} is not valid UTF-8", name))?;
            containers.push(container);
        }
        Ok(containers)
    }

    pub fn get_layer_path(&self, digest: &str) -> Option<PathBuf> {
        let layer_path = self.layer_path(digest);
        layer_path.is_dir().then_some(layer_path)
    }

    pub fn is_container_taken(&self, container: &str) -> bool {
        self.container_path(container).is_dir()
    }

    pub fn is_manifest_saved(&self, digest: &str) -> bool {
        self.manifest_path(digest).is_file()
    }

    fn containers_path(&self) -> PathBuf {
        self.storage_path.join("containers")
    }

    fn container_path(&self, container: &str) -> PathBuf {
        self.containers_path().join(container)
    }

    fn manifest_path(&self, digest: &str) -> PathBuf {
        self.storage_path
            .join("manifests")
            .join(format!("{digest}.json"))
    }

    fn layer_path(&self, digest: &str) -> PathBuf {
        self.storage_path.join("layers").join(digest)
    }

    fn temporary_path_for(&self, output_path: &Path) -> anyhow::Result<PathBuf> {
        Ok(self
            .temporary_path
            .join(output_path.file_name().with_context(|| {
                format!(
                    "output path {} does not have a file name",
                    output_path.display()
                )
            })?))
    }
}

impl StorageMutable {
    pub fn new() -> anyhow::Result<Self> {
        let storage = Storage::new_with_lock(LockMode::Exclusive)?;

        // Clear temporary directory
        if storage.temporary_path.exists() {
            fs::remove_dir_all(&storage.temporary_path).with_context(|| {
                format!(
                    "failed to remove temporary directory {}",
                    &storage.temporary_path.display()
                )
            })?;
        }
        fs::create_dir_all(&storage.temporary_path).with_context(|| {
            format!(
                "failed to create temporary directory {}",
                &storage.temporary_path.display()
            )
        })?;

        Ok(Self { storage })
    }

    pub fn read_container_manifest_digest(&self, container: &str) -> anyhow::Result<String> {
        self.storage.read_container_manifest_digest(container)
    }

    pub fn read_container_reference(&self, container: &str) -> anyhow::Result<Reference> {
        self.storage.read_container_reference(container)
    }

    pub fn read_containers(&self) -> anyhow::Result<Vec<String>> {
        self.storage.read_containers()
    }

    pub fn get_layer_path(&self, digest: &str) -> Option<PathBuf> {
        self.storage.get_layer_path(digest)
    }

    pub fn is_container_taken(&self, container: &str) -> bool {
        self.storage.is_container_taken(container)
    }

    pub fn is_manifest_saved(&self, digest: &str) -> bool {
        self.storage.is_manifest_saved(digest)
    }

    pub fn write_manifest(&self, digest: &str, contents: &[u8]) -> anyhow::Result<()> {
        let path = self.storage.manifest_path(digest);
        self.atomic_write(&path, contents)
            .with_context(|| format!("failed to write manifest to {}", path.display()))
    }

    pub fn write_container_manifest_digest(
        &self,
        container: &str,
        digest: &str,
    ) -> anyhow::Result<()> {
        let path = self
            .storage
            .container_path(container)
            .join("manifest_digest");
        self.atomic_write(&path, digest.as_bytes())
            .with_context(|| format!("failed to write manifest digest to {}", path.display()))
    }

    pub fn write_container(
        &self,
        container: &str,
        manifest_digest: &str,
        reference: &Reference,
    ) -> anyhow::Result<()> {
        let container_path = self.storage.container_path(container);
        let temporary_container_path = self.storage.temporary_path_for(&container_path)?;
        fs::create_dir_all(&temporary_container_path).with_context(|| {
            format!(
                "failed to create temporary container directory {}",
                temporary_container_path.display()
            )
        })?;

        fs::write(
            temporary_container_path.join("manifest_digest"),
            manifest_digest,
        )
        .with_context(|| {
            format!(
                "failed to write temporary container manifest digest file in {}",
                temporary_container_path.display()
            )
        })?;

        let reference = serde_json::to_vec_pretty(reference)
            .context("failed to serialize container reference")?;
        fs::write(temporary_container_path.join("reference.json"), reference).with_context(
            || {
                format!(
                    "failed to write temporary container reference file in {}",
                    temporary_container_path.display()
                )
            },
        )?;

        self.publish_directory(&temporary_container_path, &container_path)
    }

    pub fn remove_container(&self, container: &str) -> anyhow::Result<()> {
        let container_path = self.storage.container_path(container);
        if !container_path.is_dir() {
            anyhow::bail!("container `{container}` is not installed");
        }

        let temporary_container_path = self.storage.temporary_path_for(&container_path)?;
        fs::rename(&container_path, &temporary_container_path).with_context(|| {
            format!(
                "failed to move container directory {} to temporary path {}",
                container_path.display(),
                temporary_container_path.display()
            )
        })?;
        fs::remove_dir_all(&temporary_container_path).with_context(|| {
            format!(
                "failed to remove temporary container directory {}",
                temporary_container_path.display()
            )
        })
    }

    pub fn write_layer(&self, digest: &str, reader: impl Read) -> anyhow::Result<()> {
        let output_path = self.storage.layer_path(digest);
        let temporary_output_path = self.storage.temporary_path_for(&output_path)?;
        fs::create_dir_all(&temporary_output_path).with_context(|| {
            format!(
                "failed to create temporary layer output directory {}",
                temporary_output_path.display()
            )
        })?;

        if let Err(err) = extract_layer(reader, digest, &temporary_output_path) {
            let _ = fs::remove_dir_all(&temporary_output_path);
            return Err(err);
        }

        self.publish_directory(&temporary_output_path, &output_path)
            .with_context(|| {
                format!(
                    "failed to publish layer {} to {}",
                    digest,
                    output_path.display()
                )
            })
    }

    fn atomic_write(&self, path: &Path, contents: &[u8]) -> anyhow::Result<()> {
        ensure_parent_dir(path)?;

        fs::create_dir_all(&self.storage.temporary_path).with_context(|| {
            format!(
                "failed to create temporary directory {}",
                self.storage.temporary_path.display()
            )
        })?;

        let temporary_path = self.storage.temporary_path_for(path)?;
        fs::write(&temporary_path, contents).with_context(|| {
            format!(
                "failed to write temporary file {}",
                temporary_path.display()
            )
        })?;
        fs::rename(&temporary_path, path).with_context(|| {
            format!(
                "failed to rename temporary file {} to {}",
                temporary_path.display(),
                path.display()
            )
        })
    }

    fn publish_directory(&self, temporary_path: &Path, output_path: &Path) -> anyhow::Result<()> {
        ensure_parent_dir(output_path)?;

        fs::rename(temporary_path, output_path).with_context(|| {
            format!(
                "failed to rename temporary directory {} to {}",
                temporary_path.display(),
                output_path.display()
            )
        })
    }
}

fn ensure_parent_dir(path: &Path) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create output directory {}", parent.display()))?;
    }
    Ok(())
}

#[derive(Debug, Clone, Copy)]
enum LockMode {
    Shared,
    Exclusive,
}

fn acquire_lock(storage_path: &Path, mode: LockMode) -> anyhow::Result<StorageLock> {
    fs::create_dir_all(storage_path).with_context(|| {
        format!(
            "failed to create storage directory {}",
            storage_path.display()
        )
    })?;

    let lock_path = storage_path.join(".lock");

    // `.lock` serializes access to the storage directory. Readers hold a shared
    // lock, while mutating commands hold an exclusive lock so they cannot publish
    // partial state while another process is reading.
    let file = File::create(&lock_path)
        .with_context(|| format!("failed to open lock file {}", lock_path.display()))?;
    match mode {
        LockMode::Shared => file.try_lock_shared(),
        LockMode::Exclusive => file.try_lock(),
    }
    .with_context(|| format!("failed to acquire lock {}", lock_path.display()))?;

    Ok(StorageLock { _file: file })
}

fn extract_layer(
    reader: impl Read,
    expected_digest: &str,
    output_dir: &Path,
) -> anyhow::Result<()> {
    let mut verifier = DigestVerifier::new(reader, expected_digest).with_context(|| {
        format!("failed to initialize digest verifier for layer {expected_digest}")
    })?;

    {
        let mut peekable = BufReader::new(&mut verifier);
        let buffer = peekable
            .fill_buf()
            .context("failed to inspect layer bytes")?;
        let is_gzip = buffer.starts_with(&[0x1f, 0x8b]);

        if is_gzip {
            let decoder = GzDecoder::new(&mut peekable);
            Archive::new(decoder).unpack(output_dir).with_context(|| {
                format!("failed to unpack gzip layer into {}", output_dir.display())
            })?;
        } else {
            Archive::new(&mut peekable)
                .unpack(output_dir)
                .with_context(|| {
                    format!("failed to unpack tar layer into {}", output_dir.display())
                })?;
        }

        io::copy(&mut peekable, &mut io::sink())
            .context("failed to finish reading layer bytes for digest verification")?;
    }

    verifier
        .verify()
        .with_context(|| format!("failed to verify layer digest {expected_digest}"))?;

    Ok(())
}

#[derive(Debug)]
struct StorageLock {
    _file: File,
}
