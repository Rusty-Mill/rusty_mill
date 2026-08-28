//! OAuth 2.0 Protected Resource Metadata (RFC 9728).
//!
//! The spec makes this a **MUST** for MCP servers: it is how a client that got
//! a `401` finds out which authorization server to talk to. The document is
//! served unauthenticated — requiring a token to discover how to get a token
//! would be a deadlock.

use serde::Serialize;

use super::config::AuthConfig;

/// The RFC 9728 metadata document.
#[derive(Debug, Clone, Serialize)]
pub struct ProtectedResourceMetadata {
    /// Canonical URI of this resource. Must equal the audience clients request.
    pub resource: String,

    /// Issuer identifiers of authorization servers that can mint tokens here.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub authorization_servers: Vec<String>,

    /// Scopes a client may request. Minimal set for basic functionality.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub scopes_supported: Vec<String>,

    /// How tokens may be presented. This server accepts the `Authorization`
    /// header only — RFC 6750 also allows form and query, but the MCP spec
    /// forbids tokens in the URI.
    pub bearer_methods_supported: Vec<String>,

    /// Human-facing documentation.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resource_documentation: Option<String>,
}

impl ProtectedResourceMetadata {
    /// Build the document `config` describes.
    pub fn from_config(config: &AuthConfig) -> Self {
        Self {
            resource: config.resource().to_string(),
            authorization_servers: config.authorization_servers.clone(),
            scopes_supported: config.scopes_supported.clone(),
            bearer_methods_supported: vec!["header".to_string()],
            resource_documentation: config.resource_documentation.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::auth::token::StaticTokenValidator;

    #[test]
    fn serializes_the_required_shape() {
        let config = AuthConfig::new(
            "https://mcp.example.com/mcp",
            Arc::new(StaticTokenValidator::new()),
        )
        .expect("valid resource")
        .with_authorization_servers(["https://auth.example.com"])
        .with_scopes_supported(["mcp:read"]);

        let json = serde_json::to_value(ProtectedResourceMetadata::from_config(&config))
            .expect("serializes");

        assert_eq!(json["resource"], "https://mcp.example.com/mcp");
        assert_eq!(json["authorization_servers"][0], "https://auth.example.com");
        assert_eq!(json["scopes_supported"][0], "mcp:read");
        assert_eq!(json["bearer_methods_supported"][0], "header");
        // Absent rather than null, so clients don't see an empty doc field.
        assert!(json.get("resource_documentation").is_none());
    }

    #[test]
    fn omits_empty_optional_lists() {
        let config = AuthConfig::new(
            "https://mcp.example.com/mcp",
            Arc::new(StaticTokenValidator::new()),
        )
        .expect("valid resource");

        let json =
            serde_json::to_value(ProtectedResourceMetadata::from_config(&config)).expect("ok");

        assert!(json.get("authorization_servers").is_none());
        assert!(json.get("scopes_supported").is_none());
    }
}
