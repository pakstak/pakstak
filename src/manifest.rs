use serde::Deserialize;
use std::env;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageManifest {
    pub schema_version: u32,
    pub layers: Vec<Descriptor>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageIndex {
    pub manifests: Vec<Descriptor>,
}

#[derive(Debug, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Descriptor {
    pub digest: String,
    pub platform: Option<Platform>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct Platform {
    pub architecture: String,
    pub os: String,
}

pub struct FetchedManifest {
    pub manifest: ImageManifest,
    pub bytes: Vec<u8>,
    pub digest: String,
}

impl Descriptor {
    pub fn matches_current_platform(&self) -> bool {
        match &self.platform {
            Some(platform) => {
                platform.os == env::consts::OS && platform.architecture == normalized_arch()
            }
            None => false,
        }
    }
}

fn normalized_arch() -> &'static str {
    match env::consts::ARCH {
        "x86_64" => "amd64",
        "aarch64" => "arm64",
        arch => arch,
    }
}
