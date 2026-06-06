use anyhow::{Context as _, bail};
use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Reference {
    pub registry: String,
    pub repository: String,
    pub tag: String,
    pub digest: Option<String>,
}

impl Reference {
    pub fn parse(input: &str) -> anyhow::Result<Self> {
        let name = input.split_once('@').map_or(input, |(name, _digest)| name);
        let name = image_name_without_tag(name);
        let digest = input
            .split_once('@')
            .map(|(_name, digest)| digest.to_string());

        let tag = tag_from_input(input).unwrap_or("latest").to_string();
        let mut parts = name.split('/');
        let first = parts.next().context("image name is empty")?;

        let (registry, repository) =
            if first.contains('.') || first.contains(':') || first == "localhost" {
                let rest = parts.collect::<Vec<_>>().join("/");
                if rest.is_empty() {
                    bail!("image repository is missing");
                }
                (first.to_string(), rest)
            } else {
                bail!("image reference must include an explicit registry")
            };

        Ok(Self {
            registry,
            repository,
            tag,
            digest,
        })
    }

    pub fn manifest_reference(&self) -> &str {
        self.digest.as_deref().unwrap_or(&self.tag)
    }
}

impl fmt::Display for Reference {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{}/{}:{}",
            self.registry, self.repository, self.tag
        )?;
        if let Some(digest) = &self.digest {
            write!(formatter, "@{digest}")?;
        }
        Ok(())
    }
}

fn image_name_without_tag(input: &str) -> &str {
    let last_slash = input.rfind('/');
    let last_colon = input.rfind(':');
    if let Some(colon) = last_colon
        && last_slash.is_none_or(|slash| colon > slash)
    {
        return &input[..colon];
    }
    input
}

fn tag_from_input(input: &str) -> Option<&str> {
    let name = input.split_once('@').map_or(input, |(name, _digest)| name);
    let last_slash = name.rfind('/');
    let last_colon = name.rfind(':');
    let colon = last_colon.filter(|colon| last_slash.is_none_or(|slash| *colon > slash))?;
    Some(&name[colon + 1..])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_reference_without_registry() {
        let error = Reference::parse("image").unwrap_err();
        assert!(
            error
                .to_string()
                .contains("image reference must include an explicit registry")
        );
    }

    #[test]
    fn rejects_namespace_reference_without_registry() {
        let error = Reference::parse("username/image:latest").unwrap_err();
        assert!(
            error
                .to_string()
                .contains("image reference must include an explicit registry")
        );
    }

    #[test]
    fn parses_explicit_registry_tag() {
        let image = Reference::parse("ghcr.io/org/container:1.2.3").unwrap();
        assert_eq!(image.registry, "ghcr.io");
        assert_eq!(image.repository, "org/container");
        assert_eq!(image.tag, "1.2.3");
        assert_eq!(image.digest, None);
        assert_eq!(image.manifest_reference(), "1.2.3");
    }

    #[test]
    fn parses_digest_reference() {
        let image = Reference::parse("example.com/org/container@sha256:abc").unwrap();
        assert_eq!(image.registry, "example.com");
        assert_eq!(image.repository, "org/container");
        assert_eq!(image.tag, "latest");
        assert_eq!(image.digest, Some("sha256:abc".to_string()));
        assert_eq!(image.manifest_reference(), "sha256:abc");
    }

    #[test]
    fn parses_tagged_digest_reference() {
        let image = Reference::parse("example.com/org/container:1.2.3@sha256:abc").unwrap();
        assert_eq!(image.registry, "example.com");
        assert_eq!(image.repository, "org/container");
        assert_eq!(image.tag, "1.2.3");
        assert_eq!(image.digest, Some("sha256:abc".to_string()));
        assert_eq!(image.manifest_reference(), "sha256:abc");
    }

    #[test]
    fn formats_tag_reference() {
        let image = Reference::parse("ghcr.io/org/container:1.2.3").unwrap();
        assert_eq!(image.to_string(), "ghcr.io/org/container:1.2.3");
    }

    #[test]
    fn formats_tagged_digest_reference() {
        let image = Reference::parse("example.com/org/container:1.2.3@sha256:abc").unwrap();
        assert_eq!(
            image.to_string(),
            "example.com/org/container:1.2.3@sha256:abc"
        );
    }
}
