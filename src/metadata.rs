//! Authorization Server Metadata discovery (RFC 8414).

use crate::error::{Error, Result};
use crate::json::{self, Value};
use crate::request::{HttpRequest, Method};

/// Builds the well-known metadata document request for `issuer`
/// (RFC 8414 §3): `GET {issuer}/.well-known/oauth-authorization-server`,
/// with any path component on `issuer` inserted per §3.1.
pub fn discovery_request(issuer: &str) -> HttpRequest {
    let issuer = issuer.trim_end_matches('/');
    let url = if let Some(idx) = issuer.find("://").and_then(|scheme_end| {
        issuer[scheme_end + 3..]
            .find('/')
            .map(|i| scheme_end + 3 + i)
    }) {
        // issuer has a path component, e.g. https://example.com/tenant1
        let (origin, path) = issuer.split_at(idx);
        format!("{origin}/.well-known/oauth-authorization-server{path}")
    } else {
        format!("{issuer}/.well-known/oauth-authorization-server")
    };

    HttpRequest {
        method: Method::Get,
        url,
        headers: vec![("Accept".to_string(), "application/json".to_string())],
        body: Vec::new(),
    }
}

/// A parsed subset of the RFC 8414 §2 metadata document -- the fields
/// most clients need to drive a flow. `raw` retains the full document for
/// anything else (e.g. `registration_endpoint`, `jwks_uri`).
#[derive(Debug, Clone)]
pub struct AuthorizationServerMetadata {
    pub issuer: String,
    pub authorization_endpoint: Option<String>,
    pub token_endpoint: Option<String>,
    pub introspection_endpoint: Option<String>,
    pub revocation_endpoint: Option<String>,
    pub device_authorization_endpoint: Option<String>,
    pub jwks_uri: Option<String>,
    pub scopes_supported: Vec<String>,
    pub response_types_supported: Vec<String>,
    pub grant_types_supported: Vec<String>,
    pub token_endpoint_auth_methods_supported: Vec<String>,
    pub code_challenge_methods_supported: Vec<String>,
    pub raw: Value,
}

fn string_array(value: &Value, key: &str) -> Vec<String> {
    value
        .get(key)
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

fn string_field(value: &Value, key: &str) -> Option<String> {
    value.get(key).and_then(Value::as_str).map(str::to_string)
}

/// Parses a metadata document body (RFC 8414 §3.2).
pub fn parse_metadata(body: &str) -> Result<AuthorizationServerMetadata> {
    let value = json::parse(body)?;
    let issuer = string_field(&value, "issuer")
        .ok_or_else(|| Error::Protocol("metadata document missing `issuer`".to_string()))?;

    Ok(AuthorizationServerMetadata {
        issuer,
        authorization_endpoint: string_field(&value, "authorization_endpoint"),
        token_endpoint: string_field(&value, "token_endpoint"),
        introspection_endpoint: string_field(&value, "introspection_endpoint"),
        revocation_endpoint: string_field(&value, "revocation_endpoint"),
        device_authorization_endpoint: string_field(&value, "device_authorization_endpoint"),
        jwks_uri: string_field(&value, "jwks_uri"),
        scopes_supported: string_array(&value, "scopes_supported"),
        response_types_supported: string_array(&value, "response_types_supported"),
        grant_types_supported: string_array(&value, "grant_types_supported"),
        token_endpoint_auth_methods_supported: string_array(
            &value,
            "token_endpoint_auth_methods_supported",
        ),
        code_challenge_methods_supported: string_array(&value, "code_challenge_methods_supported"),
        raw: value,
    })
}

/// Validates that `issuer` in a fetched metadata document matches the
/// issuer identifier used to locate it, per RFC 8414 §3.3 -- a required
/// check to defend against mix-up attacks.
pub fn verify_issuer(metadata: &AuthorizationServerMetadata, expected_issuer: &str) -> bool {
    metadata.issuer.trim_end_matches('/') == expected_issuer.trim_end_matches('/')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discovery_request_simple_issuer() {
        let req = discovery_request("https://example.com");
        assert_eq!(
            req.url,
            "https://example.com/.well-known/oauth-authorization-server"
        );
    }

    #[test]
    fn discovery_request_issuer_with_path() {
        let req = discovery_request("https://example.com/tenant1");
        assert_eq!(
            req.url,
            "https://example.com/.well-known/oauth-authorization-server/tenant1"
        );
    }

    #[test]
    fn parses_metadata_document() {
        let json = r#"{
            "issuer": "https://example.com",
            "authorization_endpoint": "https://example.com/authorize",
            "token_endpoint": "https://example.com/token",
            "response_types_supported": ["code"],
            "grant_types_supported": ["authorization_code", "refresh_token"],
            "code_challenge_methods_supported": ["S256"]
        }"#;
        let metadata = parse_metadata(json).unwrap();
        assert_eq!(metadata.issuer, "https://example.com");
        assert_eq!(
            metadata.token_endpoint.as_deref(),
            Some("https://example.com/token")
        );
        assert_eq!(
            metadata.grant_types_supported,
            vec!["authorization_code", "refresh_token"]
        );
        assert!(verify_issuer(&metadata, "https://example.com"));
        assert!(!verify_issuer(&metadata, "https://evil.example.com"));
    }
}
