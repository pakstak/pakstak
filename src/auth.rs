use anyhow::{Context as _, anyhow, bail};
use serde::Deserialize;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthChallenge {
    pub realm: String,
    pub service: Option<String>,
    pub scopes: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct TokenResponse {
    token: Option<String>,
    access_token: Option<String>,
}

pub fn parse_bearer_challenge(header: &str) -> anyhow::Result<AuthChallenge> {
    let challenge = http_auth::parse_challenges(header)
        .map_err(|error| anyhow!("failed to parse registry auth challenge `{header}`: {error:?}"))?
        .into_iter()
        .find(|challenge| challenge.scheme.eq_ignore_ascii_case("Bearer"))
        .ok_or_else(|| anyhow!("unsupported registry auth challenge `{header}`"))?;

    let mut realm = None;
    let mut service = None;
    let mut scopes = Vec::new();

    for (key, value) in challenge.params {
        match key {
            "realm" => realm = Some(value.to_unescaped()),
            "service" => service = Some(value.to_unescaped()),
            "scope" => scopes.push(value.to_unescaped()),
            _ => {}
        }
    }

    let realm = realm.context("registry auth challenge did not include a realm")?;
    Ok(AuthChallenge {
        realm,
        service,
        scopes,
    })
}

pub fn fetch_bearer_token(
    agent: &ureq::Agent,
    challenge: &AuthChallenge,
) -> anyhow::Result<String> {
    let mut request = agent.get(&challenge.realm).query("client_id", "pakstak");
    if let Some(service) = &challenge.service {
        request = request.query("service", service);
    }
    for scope in &challenge.scopes {
        request = request.query("scope", scope);
    }

    let token: TokenResponse = request
        .call()
        .with_context(|| {
            format!(
                "registry auth token request failed: GET {}",
                challenge.realm
            )
        })?
        .into_json()
        .context("failed to parse registry auth token response")?;

    token
        .token
        .or(token.access_token)
        .ok_or_else(|| anyhow!("registry auth token response did not include a token"))
}

pub fn auth_header_from_unauthorized(response: &ureq::Response) -> anyhow::Result<&str> {
    response
        .header("WWW-Authenticate")
        .or_else(|| response.header("Www-Authenticate"))
        .context("registry returned 401 without a WWW-Authenticate header")
}

pub fn token_from_unauthorized(
    agent: &ureq::Agent,
    response: &ureq::Response,
) -> anyhow::Result<String> {
    let header = auth_header_from_unauthorized(response)?;
    let challenge = parse_bearer_challenge(header)?;
    if challenge.realm.is_empty() {
        bail!("registry auth challenge included an empty realm");
    }
    fetch_bearer_token(agent, &challenge)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_bearer_challenge() {
        let challenge = parse_bearer_challenge(
            "Bearer realm=\"https://auth.example.test/token\",service=\"registry.example.test\",scope=\"repository:namespace/app:pull,push\"",
        )
        .unwrap();

        assert_eq!(challenge.realm, "https://auth.example.test/token");
        assert_eq!(challenge.service.as_deref(), Some("registry.example.test"));
        assert_eq!(
            challenge.scopes,
            vec!["repository:namespace/app:pull,push".to_string()]
        );
    }
}
