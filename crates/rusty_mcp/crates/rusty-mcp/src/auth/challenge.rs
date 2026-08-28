//! `WWW-Authenticate` challenge construction (RFC 6750 §3).
//!
//! Getting this header right is what makes a client able to recover on its own:
//! `resource_metadata` tells it where to look up the authorization server, and
//! `scope` tells it what to ask for. A `401` without it leaves the client with
//! nowhere to go.

use std::collections::BTreeSet;

/// A `Bearer` challenge, rendered as a header value.
#[derive(Debug, Clone)]
pub struct Challenge {
    error: Option<&'static str>,
    error_description: Option<String>,
    resource_metadata: Option<String>,
    scope: Option<String>,
}

impl Challenge {
    /// A bare challenge: no token was presented.
    ///
    /// RFC 6750 §3.1 is explicit that a request without credentials should not
    /// carry an `error` code, which is why this differs from
    /// [`Challenge::invalid_token`].
    pub fn unauthorized() -> Self {
        Self {
            error: None,
            error_description: None,
            resource_metadata: None,
            scope: None,
        }
    }

    /// `error="invalid_token"` — a token was presented but did not validate.
    pub fn invalid_token(description: impl Into<String>) -> Self {
        Self {
            error: Some("invalid_token"),
            error_description: Some(description.into()),
            resource_metadata: None,
            scope: None,
        }
    }

    /// `error="insufficient_scope"` — the token is valid but underprivileged.
    ///
    /// `missing` should list every scope the operation needs, in one challenge.
    /// Drip-feeding them one at a time forces a fresh authorization round trip
    /// per scope, which the spec calls out as a user-experience problem.
    pub fn insufficient_scope(missing: &BTreeSet<String>) -> Self {
        Self {
            error: Some("insufficient_scope"),
            error_description: Some(format!(
                "the access token is missing required scopes: {}",
                missing.iter().cloned().collect::<Vec<_>>().join(", ")
            )),
            resource_metadata: None,
            scope: Some(space_delimited(missing)),
        }
    }

    /// `error="invalid_request"` — the `Authorization` header was malformed.
    pub fn invalid_request(description: impl Into<String>) -> Self {
        Self {
            error: Some("invalid_request"),
            error_description: Some(description.into()),
            resource_metadata: None,
            scope: None,
        }
    }

    /// Point the client at the Protected Resource Metadata document.
    pub fn with_resource_metadata(mut self, url: impl Into<String>) -> Self {
        self.resource_metadata = Some(url.into());
        self
    }

    /// Advertise the scopes needed, unless a more specific set is already set.
    pub fn with_scope(mut self, scopes: &BTreeSet<String>) -> Self {
        if self.scope.is_none() && !scopes.is_empty() {
            self.scope = Some(space_delimited(scopes));
        }
        self
    }

    /// The `error` code, if any.
    pub fn error(&self) -> Option<&'static str> {
        self.error
    }

    /// Render as a `WWW-Authenticate` header value.
    pub fn to_header_value(&self) -> String {
        let mut params: Vec<String> = Vec::new();

        if let Some(error) = self.error {
            params.push(format!("error={}", quote(error)));
        }
        if let Some(description) = &self.error_description {
            params.push(format!("error_description={}", quote(description)));
        }
        if let Some(scope) = &self.scope {
            params.push(format!("scope={}", quote(scope)));
        }
        if let Some(url) = &self.resource_metadata {
            params.push(format!("resource_metadata={}", quote(url)));
        }

        if params.is_empty() {
            "Bearer".to_string()
        } else {
            format!("Bearer {}", params.join(", "))
        }
    }
}

fn space_delimited(scopes: &BTreeSet<String>) -> String {
    scopes.iter().cloned().collect::<Vec<_>>().join(" ")
}

/// Render as an RFC 7230 quoted-string.
///
/// Backslashes and quotes are escaped; control characters are dropped, since
/// they cannot appear in a header value and a rejected header would cost the
/// client the challenge entirely.
fn quote(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for ch in value.chars() {
        match ch {
            '"' | '\\' => {
                out.push('\\');
                out.push(ch);
            }
            c if c.is_control() => {}
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scopes(items: &[&str]) -> BTreeSet<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn bare_challenge_has_no_error_code() {
        assert_eq!(Challenge::unauthorized().to_header_value(), "Bearer");
    }

    #[test]
    fn unauthorized_matches_the_spec_example() {
        let header = Challenge::unauthorized()
            .with_resource_metadata("https://mcp.example.com/.well-known/oauth-protected-resource")
            .with_scope(&scopes(&["files:read"]))
            .to_header_value();

        assert_eq!(
            header,
            "Bearer scope=\"files:read\", \
             resource_metadata=\"https://mcp.example.com/.well-known/oauth-protected-resource\""
        );
    }

    #[test]
    fn insufficient_scope_carries_all_missing_scopes() {
        let header = Challenge::insufficient_scope(&scopes(&["files:write", "files:read"]))
            .with_resource_metadata("https://mcp.example.com/.well-known/oauth-protected-resource")
            .to_header_value();

        assert!(header.contains("error=\"insufficient_scope\""));
        // Both in a single challenge, not one per round trip.
        assert!(header.contains("scope=\"files:read files:write\""));
        assert!(header.contains("resource_metadata="));
    }

    #[test]
    fn explicit_scope_is_not_overwritten_by_the_default() {
        let header = Challenge::insufficient_scope(&scopes(&["files:write"]))
            .with_scope(&scopes(&["mcp:read"]))
            .to_header_value();

        assert!(header.contains("scope=\"files:write\""));
        assert!(!header.contains("mcp:read"));
    }

    #[test]
    fn escapes_quotes_and_strips_control_characters() {
        let header =
            Challenge::invalid_token("he said \"no\" \\ here\r\nInjected: yes").to_header_value();

        assert!(header.contains(r#"\"no\""#));
        assert!(header.contains(r"\\"));
        // A header injection attempt must not survive into the value.
        assert!(!header.contains('\r'));
        assert!(!header.contains('\n'));
    }

    #[test]
    fn empty_scope_set_adds_no_scope_parameter() {
        let header = Challenge::unauthorized()
            .with_scope(&scopes(&[]))
            .to_header_value();
        assert_eq!(header, "Bearer");
    }
}
