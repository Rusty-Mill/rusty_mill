//! OAuth 2.0 Protected Resource Metadata (RFC 9728).
//!
//! The spec makes this a **MUST** for MCP servers: it is how a client that got
//! a `401` finds out which authorization server to talk to. The document is
//! served unauthenticated — requiring a token to discover how to get a token
//! would be a deadlock.

use rusty_json::{Map, Value};

use super::config::AuthConfig;

/// The RFC 9728 metadata document.
#[derive(Debug, Clone)]
pub struct ProtectedResourceMetadata {
    /// Canonical URI of this resource. Must equal the audience clients request.
    pub resource: String,

    /// Issuer identifiers of authorization servers that can mint tokens here.
    pub authorization_servers: Vec<String>,

    /// Scopes a client may request. Minimal set for basic functionality.
    pub scopes_supported: Vec<String>,

    /// How tokens may be presented. This server accepts the `Authorization`
    /// header only — RFC 6750 also allows form and query, but the MCP spec
    /// forbids tokens in the URI.
    pub bearer_methods_supported: Vec<String>,

    /// Human-facing documentation.
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

    /// The document as a JSON value. Empty lists and an unset
    /// `resource_documentation` are left out rather than written as `[]` or
    /// `null`; keys come out in alphabetical order.
    pub fn to_value(&self) -> Value {
        let mut doc = Map::new();
        doc.insert("resource".into(), self.resource.clone().into());
        insert_list(
            &mut doc,
            "authorization_servers",
            &self.authorization_servers,
        );
        insert_list(&mut doc, "scopes_supported", &self.scopes_supported);
        doc.insert(
            "bearer_methods_supported".into(),
            strings(&self.bearer_methods_supported),
        );
        if let Some(url) = &self.resource_documentation {
            doc.insert("resource_documentation".into(), url.clone().into());
        }
        doc.into()
    }
}

fn strings(items: &[String]) -> Value {
    items.iter().cloned().map(Value::from).collect()
}

fn insert_list(doc: &mut Map, key: &str, items: &[String]) {
    if !items.is_empty() {
        doc.insert(key.into(), strings(items));
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

        let json = ProtectedResourceMetadata::from_config(&config).to_value();

        assert_eq!(json["resource"], "https://mcp.example.com/mcp");
        assert_eq!(json["authorization_servers"][0], "https://auth.example.com");
        assert_eq!(json["scopes_supported"][0], "mcp:read");
        assert_eq!(json["bearer_methods_supported"][0], "header");
        // Absent rather than null, so clients don't see an empty doc field.
        assert!(json.get("resource_documentation").is_none());
    }

    #[test]
    fn writes_the_whole_document_with_sorted_keys() {
        let config = AuthConfig::new(
            "https://mcp.example.com/mcp",
            Arc::new(StaticTokenValidator::new()),
        )
        .expect("valid resource")
        .with_authorization_servers(["https://auth.example.com"])
        .with_scopes_supported(["mcp:read", "mcp:write"])
        .with_resource_documentation("https://docs.example.com");

        let text = ProtectedResourceMetadata::from_config(&config)
            .to_value()
            .to_json_string();

        assert_eq!(
            text,
            concat!(
                r#"{"authorization_servers":["https://auth.example.com"],"#,
                r#""bearer_methods_supported":["header"],"#,
                r#""resource":"https://mcp.example.com/mcp","#,
                r#""resource_documentation":"https://docs.example.com","#,
                r#""scopes_supported":["mcp:read","mcp:write"]}"#,
            )
        );
    }

    #[test]
    fn omits_empty_optional_lists() {
        let config = AuthConfig::new(
            "https://mcp.example.com/mcp",
            Arc::new(StaticTokenValidator::new()),
        )
        .expect("valid resource");

        let json = ProtectedResourceMetadata::from_config(&config).to_value();

        assert!(json.get("authorization_servers").is_none());
        assert!(json.get("scopes_supported").is_none());
    }
}
