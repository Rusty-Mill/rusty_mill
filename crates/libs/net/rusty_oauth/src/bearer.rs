//! Bearer token usage (RFC 6750): attaching an access token to a resource
//! request, and parsing `WWW-Authenticate` challenges.

use std::fmt;

/// Builds the `Authorization: Bearer <token>` header value for
/// authenticating a resource request (RFC 6750 §2.1). This is the
/// recommended way to send a bearer token; avoid the form-encoded body
/// parameter (§2.2) and URI query parameter (§2.3) methods where possible,
/// since both are more prone to leaking the token (into logs, proxies,
/// browser history).
pub fn authorization_header(access_token: &str) -> String {
    format!("Bearer {access_token}")
}

/// The parsed fields of a `WWW-Authenticate: Bearer ...` challenge
/// returned by a resource server on a failed request (RFC 6750 §3).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BearerChallenge {
    pub realm: Option<String>,
    pub error: Option<String>,
    pub error_description: Option<String>,
    pub error_uri: Option<String>,
    pub scope: Option<String>,
}

impl BearerChallenge {
    /// Maps the challenge's `error` field to the standard error codes
    /// defined in RFC 6750 §3.1 (`invalid_request`, `invalid_token`,
    /// `insufficient_scope`).
    pub fn error_code(&self) -> Option<crate::error::ErrorCode> {
        self.error.as_deref().map(crate::error::ErrorCode::from)
    }
}

/// Parses a `WWW-Authenticate` header value. Only the `Bearer` scheme is
/// recognized; other schemes (or a missing header) yield `None`.
///
/// Handles the `auth-param` list format of RFC 6750 §3, e.g.:
/// `Bearer realm="example", error="invalid_token", error_description="The access token expired"`
pub fn parse_www_authenticate(header_value: &str) -> Option<BearerChallenge> {
    let rest = header_value.trim();
    let rest = rest.strip_prefix("Bearer")?;
    // The scheme name must be followed by whitespace or end-of-string,
    // not run directly into another token (rules out "Bearerish ...").
    if !rest.is_empty() && !rest.starts_with(' ') && !rest.starts_with('\t') {
        return None;
    }
    let rest = rest.trim_start();

    let mut challenge = BearerChallenge::default();
    for param in split_auth_params(rest) {
        let Some((key, value)) = param.split_once('=') else {
            continue;
        };
        let key = key.trim();
        let value = value.trim().trim_matches('"');
        match key {
            "realm" => challenge.realm = Some(value.to_string()),
            "error" => challenge.error = Some(value.to_string()),
            "error_description" => challenge.error_description = Some(value.to_string()),
            "error_uri" => challenge.error_uri = Some(value.to_string()),
            "scope" => challenge.scope = Some(value.to_string()),
            _ => {}
        }
    }
    Some(challenge)
}

/// Splits a comma-separated `auth-param` list, respecting commas that
/// appear inside quoted string values.
fn split_auth_params(input: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut in_quotes = false;
    let mut start = 0;
    for (i, c) in input.char_indices() {
        match c {
            '"' => in_quotes = !in_quotes,
            ',' if !in_quotes => {
                parts.push(input[start..i].trim());
                start = i + 1;
            }
            _ => {}
        }
    }
    let tail = input[start..].trim();
    if !tail.is_empty() {
        parts.push(tail);
    }
    parts
}

impl fmt::Display for BearerChallenge {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Bearer")?;
        let mut first = true;
        let mut push =
            |f: &mut fmt::Formatter<'_>, key: &str, value: &Option<String>| -> fmt::Result {
                if let Some(v) = value {
                    write!(f, "{}{}=\"{}\"", if first { " " } else { ", " }, key, v)?;
                    first = false;
                }
                Ok(())
            };
        push(f, "realm", &self.realm)?;
        push(f, "error", &self.error)?;
        push(f, "error_description", &self.error_description)?;
        push(f, "error_uri", &self.error_uri)?;
        push(f, "scope", &self.scope)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_authorization_header() {
        assert_eq!(
            authorization_header("mF_9.B5f-4.1JqM"),
            "Bearer mF_9.B5f-4.1JqM"
        );
    }

    #[test]
    fn parses_full_challenge() {
        let header = r#"Bearer realm="example", error="invalid_token", error_description="The access token expired""#;
        let challenge = parse_www_authenticate(header).unwrap();
        assert_eq!(challenge.realm.as_deref(), Some("example"));
        assert_eq!(challenge.error.as_deref(), Some("invalid_token"));
        assert_eq!(
            challenge.error_description.as_deref(),
            Some("The access token expired")
        );
        assert_eq!(
            challenge.error_code(),
            Some(crate::error::ErrorCode::from("invalid_token"))
        );
    }

    #[test]
    fn parses_bare_bearer() {
        let challenge = parse_www_authenticate("Bearer").unwrap();
        assert_eq!(challenge, BearerChallenge::default());
    }

    #[test]
    fn rejects_non_bearer_scheme() {
        assert!(parse_www_authenticate("Basic realm=\"example\"").is_none());
        assert!(parse_www_authenticate("Bearerish realm=\"x\"").is_none());
    }

    #[test]
    fn display_roundtrips_through_parse() {
        let challenge = BearerChallenge {
            realm: Some("example".to_string()),
            error: Some("insufficient_scope".to_string()),
            error_description: None,
            error_uri: None,
            scope: Some("read write".to_string()),
        };
        let rendered = challenge.to_string();
        let reparsed = parse_www_authenticate(&rendered).unwrap();
        assert_eq!(reparsed, challenge);
    }
}
