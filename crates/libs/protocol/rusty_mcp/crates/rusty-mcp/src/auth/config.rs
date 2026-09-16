//! Resource-server configuration.

use std::{collections::BTreeSet, fmt, sync::Arc};

use rusty_url::Url;

use super::token::TokenValidator;

/// The canonical resource URI was unusable.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum AuthConfigError {
    /// Not a parseable absolute URI.
    #[error("`{uri}` is not a valid absolute URI: {source}")]
    Malformed {
        /// What was supplied.
        uri: String,
        /// Underlying parse failure.
        #[source]
        source: rusty_url::ParseError,
    },

    /// Carried a fragment, which RFC 8707 forbids on a resource identifier.
    #[error("resource URI `{0}` must not contain a fragment")]
    HasFragment(String),

    /// Carried a query string, which is not part of a resource identifier.
    #[error("resource URI `{0}` must not contain a query string")]
    HasQuery(String),
}

/// What the MCP endpoint requires of incoming requests, and what it publishes
/// about itself.
///
/// Authorization applies to Streamable HTTP only. The spec is explicit that
/// stdio servers **SHOULD NOT** use it and should take credentials from the
/// environment instead, so [`crate::config::Transport::Stdio`] has no hook for
/// this.
#[derive(Clone)]
pub struct AuthConfig {
    resource: Url,
    /// Authorization servers that can mint tokens for this resource,
    /// advertised via Protected Resource Metadata.
    pub authorization_servers: Vec<String>,
    /// Scopes published in `scopes_supported`.
    ///
    /// Per the spec this should be the *minimal* set needed for basic
    /// functionality, not everything the server can do.
    pub scopes_supported: Vec<String>,
    /// Scopes every request to the MCP endpoint must carry.
    ///
    /// Empty means any validly-audienced token gets in. For finer control,
    /// leave this empty and check scopes per tool by reading the
    /// [`crate::auth::VerifiedToken`] out of the request extensions.
    pub required_scopes: BTreeSet<String>,
    /// Optional human-facing documentation URL for the metadata document.
    pub resource_documentation: Option<String>,
    /// How presented tokens are validated.
    pub validator: Arc<dyn TokenValidator>,
}

impl fmt::Debug for AuthConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AuthConfig")
            .field("resource", &self.resource.as_str())
            .field("authorization_servers", &self.authorization_servers)
            .field("scopes_supported", &self.scopes_supported)
            .field("required_scopes", &self.required_scopes)
            .field("resource_documentation", &self.resource_documentation)
            .field("validator", &"<dyn TokenValidator>")
            .finish()
    }
}

impl AuthConfig {
    /// Protect the endpoint identified by `resource`, validating with `validator`.
    ///
    /// `resource` is the **canonical URI** clients name in the RFC 8707
    /// `resource` parameter, and the audience their tokens must carry — for
    /// example `https://mcp.example.com/mcp`. It must be absolute and carry
    /// neither fragment nor query. A trailing slash is stripped, matching the
    /// spec's guidance to prefer the form without one.
    pub fn new(
        resource: &str,
        validator: Arc<dyn TokenValidator>,
    ) -> Result<Self, AuthConfigError> {
        let parsed = Url::parse(resource).map_err(|source| AuthConfigError::Malformed {
            uri: resource.to_string(),
            source,
        })?;

        if parsed.fragment().is_some() {
            return Err(AuthConfigError::HasFragment(resource.to_string()));
        }
        if parsed.query().is_some() {
            return Err(AuthConfigError::HasQuery(resource.to_string()));
        }

        Ok(Self {
            resource: normalize(parsed),
            authorization_servers: Vec::new(),
            scopes_supported: Vec::new(),
            required_scopes: BTreeSet::new(),
            resource_documentation: None,
            validator,
        })
    }

    /// Advertise the authorization servers that mint tokens for this resource.
    pub fn with_authorization_servers(
        mut self,
        servers: impl IntoIterator<Item = impl Into<String>>,
    ) -> Self {
        self.authorization_servers = servers.into_iter().map(Into::into).collect();
        self
    }

    /// Publish `scopes_supported` in the metadata document.
    pub fn with_scopes_supported(
        mut self,
        scopes: impl IntoIterator<Item = impl Into<String>>,
    ) -> Self {
        self.scopes_supported = scopes.into_iter().map(Into::into).collect();
        self
    }

    /// Require these scopes on every request to the MCP endpoint.
    pub fn with_required_scopes(
        mut self,
        scopes: impl IntoIterator<Item = impl Into<String>>,
    ) -> Self {
        self.required_scopes = scopes.into_iter().map(Into::into).collect();
        self
    }

    /// Point the metadata document at human-facing docs.
    pub fn with_resource_documentation(mut self, url: impl Into<String>) -> Self {
        self.resource_documentation = Some(url.into());
        self
    }

    /// The canonical resource URI, without a trailing slash.
    pub fn resource(&self) -> &str {
        self.resource.as_str().trim_end_matches('/')
    }

    /// Whether `audience` names this resource.
    ///
    /// Compared as strings after trailing-slash normalization. Deliberately not
    /// a looser match: a prefix or host-only comparison would let a token
    /// minted for a sibling resource through.
    pub fn matches_audience(&self, audience: &str) -> bool {
        audience.trim_end_matches('/') == self.resource()
    }

    /// Absolute URL of this resource's Protected Resource Metadata document.
    ///
    /// RFC 9728 §3.1 inserts `/.well-known/oauth-protected-resource` *before*
    /// the resource's path, so `https://host/mcp` publishes at
    /// `https://host/.well-known/oauth-protected-resource/mcp` — not at the
    /// path-suffixed location people usually guess.
    pub fn metadata_url(&self) -> String {
        let mut url = self.resource.clone();
        let path = url.path().trim_end_matches('/').to_string();
        url.set_path(&format!("/.well-known/oauth-protected-resource{path}"));
        url.to_string()
    }

    /// Path component of [`AuthConfig::metadata_url`], for mounting a route.
    pub fn metadata_path(&self) -> String {
        let path = self.resource.path().trim_end_matches('/');
        format!("/.well-known/oauth-protected-resource{path}")
    }
}

/// Drop a trailing slash from the path, per the spec's interoperability note.
fn normalize(mut url: Url) -> Url {
    let path = url.path().trim_end_matches('/').to_string();
    url.set_path(&path);
    url
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::token::StaticTokenValidator;

    fn config(resource: &str) -> Result<AuthConfig, AuthConfigError> {
        AuthConfig::new(resource, Arc::new(StaticTokenValidator::new()))
    }

    #[test]
    fn derives_metadata_location_with_path_insertion() {
        let cfg = config("https://mcp.example.com/mcp").expect("valid");
        assert_eq!(cfg.resource(), "https://mcp.example.com/mcp");
        assert_eq!(
            cfg.metadata_url(),
            "https://mcp.example.com/.well-known/oauth-protected-resource/mcp"
        );
        assert_eq!(
            cfg.metadata_path(),
            "/.well-known/oauth-protected-resource/mcp"
        );
    }

    #[test]
    fn derives_metadata_location_for_a_root_resource() {
        let cfg = config("https://mcp.example.com").expect("valid");
        assert_eq!(cfg.resource(), "https://mcp.example.com");
        assert_eq!(
            cfg.metadata_url(),
            "https://mcp.example.com/.well-known/oauth-protected-resource"
        );
        assert_eq!(cfg.metadata_path(), "/.well-known/oauth-protected-resource");
    }

    #[test]
    fn normalizes_a_trailing_slash() {
        let cfg = config("https://mcp.example.com/mcp/").expect("valid");
        assert_eq!(cfg.resource(), "https://mcp.example.com/mcp");
        assert!(cfg.matches_audience("https://mcp.example.com/mcp"));
        assert!(cfg.matches_audience("https://mcp.example.com/mcp/"));
    }

    #[test]
    fn rejects_uris_the_spec_calls_invalid() {
        assert!(matches!(
            config("mcp.example.com"),
            Err(AuthConfigError::Malformed { .. })
        ));
        assert!(matches!(
            config("https://mcp.example.com#fragment"),
            Err(AuthConfigError::HasFragment(_))
        ));
        assert!(matches!(
            config("https://mcp.example.com?a=b"),
            Err(AuthConfigError::HasQuery(_))
        ));
    }

    #[test]
    fn audience_matching_is_exact_not_prefix() {
        let cfg = config("https://mcp.example.com/mcp").expect("valid");

        assert!(cfg.matches_audience("https://mcp.example.com/mcp"));
        // A token minted for a sibling resource must not be accepted.
        assert!(!cfg.matches_audience("https://mcp.example.com/mcp-admin"));
        assert!(!cfg.matches_audience("https://mcp.example.com"));
        assert!(!cfg.matches_audience("https://evil.example.com/mcp"));
    }

    #[test]
    fn preserves_a_non_default_port() {
        let cfg = config("https://mcp.example.com:8443/mcp").expect("valid");
        assert_eq!(cfg.resource(), "https://mcp.example.com:8443/mcp");
        assert!(cfg.matches_audience("https://mcp.example.com:8443/mcp"));
        assert!(!cfg.matches_audience("https://mcp.example.com/mcp"));
    }
}
