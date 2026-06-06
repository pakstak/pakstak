use std::env;
use std::io;
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct Context {
    pub storage_path: PathBuf,
}

impl Context {
    pub fn new() -> io::Result<Self> {
        let home_dir = env::var_os("HOME")
            .map(PathBuf::from)
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "HOME is not set"))?;

        Ok(Self {
            storage_path: home_dir.join(".var").join("pakstak"),
        })
    }
}
