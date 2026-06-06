use anyhow::Context as _;
use std::env;
use std::fs::{self, File};
use std::io;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct Context {
    pub storage_path: PathBuf,
    temporary_path: PathBuf,
    lock_path: PathBuf,
}

#[derive(Debug, Clone, Copy)]
pub enum LockMode {
    Shared,
    Exclusive,
}

impl Context {
    pub fn new() -> io::Result<Self> {
        let home_dir = env::var_os("HOME")
            .map(PathBuf::from)
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "HOME is not set"))?;
        let storage_path = home_dir.join(".var").join("pakstak");

        Ok(Self {
            temporary_path: storage_path.join("temporary"),
            lock_path: storage_path.join(".lock"),
            storage_path,
        })
    }

    pub fn acquire_lock(&self, mode: LockMode) -> anyhow::Result<StorageLock> {
        fs::create_dir_all(&self.storage_path).with_context(|| {
            format!(
                "failed to create storage directory {}",
                self.storage_path.display()
            )
        })?;

        // `.lock` serializes access to the storage directory. Readers hold a
        // shared lock, while mutating commands hold an exclusive lock so they
        // cannot publish partial state while another process is reading.
        let file = File::create(&self.lock_path)
            .with_context(|| format!("failed to open lock file {}", self.lock_path.display()))?;
        match mode {
            LockMode::Shared => file.try_lock_shared(),
            LockMode::Exclusive => file.try_lock(),
        }
        .with_context(|| format!("failed to acquire lock {}", self.lock_path.display()))?;

        // `temporary` is the staging area for writes. Mutating commands clean it
        // immediately after taking the exclusive lock, then write new content
        // there and publish completed files/directories with atomic renames.
        if matches!(mode, LockMode::Exclusive) {
            self.clear_temporary_dir()?;
        }

        Ok(StorageLock { _file: file })
    }

    pub fn atomic_write(&self, path: &Path, contents: &[u8]) -> anyhow::Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).with_context(|| {
                format!("failed to create output directory {}", parent.display())
            })?;
        }

        fs::create_dir_all(&self.temporary_path).with_context(|| {
            format!(
                "failed to create temporary directory {}",
                self.temporary_path.display()
            )
        })?;

        let temporary_path = self.temporary_path.join(path.file_name().with_context(|| {
            format!("output path {} does not have a file name", path.display())
        })?);
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

    pub fn temporary_directory_for(&self, output_path: &Path) -> anyhow::Result<PathBuf> {
        Ok(self
            .temporary_path
            .join(output_path.file_name().with_context(|| {
                format!(
                    "output path {} does not have a directory name",
                    output_path.display()
                )
            })?))
    }

    pub fn publish_directory(
        &self,
        temporary_path: &Path,
        output_path: &Path,
    ) -> anyhow::Result<()> {
        if let Some(parent) = output_path.parent() {
            fs::create_dir_all(parent).with_context(|| {
                format!("failed to create output directory {}", parent.display())
            })?;
        }

        fs::rename(temporary_path, output_path).with_context(|| {
            format!(
                "failed to rename temporary directory {} to {}",
                temporary_path.display(),
                output_path.display()
            )
        })
    }

    fn clear_temporary_dir(&self) -> anyhow::Result<()> {
        if self.temporary_path.exists() {
            fs::remove_dir_all(&self.temporary_path).with_context(|| {
                format!(
                    "failed to remove temporary directory {}",
                    self.temporary_path.display()
                )
            })?;
        }
        fs::create_dir_all(&self.temporary_path).with_context(|| {
            format!(
                "failed to create temporary directory {}",
                self.temporary_path.display()
            )
        })
    }
}

pub struct StorageLock {
    _file: File,
}
