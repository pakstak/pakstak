use anyhow::{Context as _, bail};
use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(PartialEq, Eq, Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Specifier {
    Digest(String),
    Tag(String),
}

impl Specifier {
    pub fn as_typeless_str(&self) -> &str {
        match self {
            Specifier::Digest(s) => s,
            Specifier::Tag(s) => s,
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Reference {
    pub registry: String,
    pub repository: String,
    pub specifier: Specifier,
}

impl Reference {
    pub fn parse(input: &str) -> anyhow::Result<Self> {
        let (name, digest) = input
            .rsplit_once('@')
            .map(|(n, d)| (n, Some(d)))
            .unwrap_or((input, None));

        let (name, tag) = name
            .rsplit_once(':')
            .map(|(n, d)| (n, Some(d)))
            .unwrap_or((name, None));

        let mut parts = name.split('/');
        let registry = parts.next().context("image name is empty")?.to_owned();

        if !registry.contains('.') && !registry.contains(':') && registry != "localhost" {
            bail!("image reference must include an explicit registry")
        }

        let repository = parts.collect::<Vec<_>>().join("/").to_string();

        if repository.is_empty() {
            bail!("image repository is missing");
        }

        Ok(Self {
            registry,
            repository,
            specifier: digest
                .map(|d| Specifier::Digest(d.to_owned()))
                .unwrap_or_else(|| Specifier::Tag(tag.unwrap_or("latest").to_owned())),
        })
    }
}

impl fmt::Display for Reference {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}/{}", self.registry, self.repository)?;

        match &self.specifier {
            Specifier::Digest(digest) => {
                write!(formatter, "@{digest}")
            }
            Specifier::Tag(tag) => {
                write!(formatter, ":{tag}")
            }
        }
    }
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
        assert_eq!(image.specifier, Specifier::Tag("1.2.3".to_owned()));
    }

    #[test]
    fn parses_digest_reference() {
        let image = Reference::parse("example.com/org/container@sha256:abc").unwrap();
        assert_eq!(image.registry, "example.com");
        assert_eq!(image.repository, "org/container");
        assert_eq!(image.specifier, Specifier::Digest("sha256:abc".to_owned()));
    }

    #[test]
    fn parses_tagged_digest_reference() {
        let image = Reference::parse("example.com/org/container:1.2.3@sha256:abc").unwrap();
        assert_eq!(image.registry, "example.com");
        assert_eq!(image.repository, "org/container");
        assert_eq!(image.specifier, Specifier::Digest("sha256:abc".to_owned()));
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
