//! Endpoint hardening for remote (HTTP/SSE) MCP servers, independent of the
//! `rmcp` transport so it is exercised in offline CI. Two concerns:
//!
//! - **TLS for non-loopback** (threat-model): plaintext `http://` is only
//!   tolerated to a loopback host; any other host must use `https://`, so a
//!   bearer token is never sent in the clear over the network.
//! - **Bearer-token resolution**: the token is read from the env var named by
//!   `auth_token_env`, never stored in `mcp.toml`.

use crate::McpError;

/// Reject a remote MCP endpoint that would send traffic (and its bearer token)
/// over plaintext to a non-loopback host. `https` is always allowed; `http` is
/// allowed only to loopback.
pub fn require_tls_for_non_loopback(url: &str) -> Result<(), McpError> {
    let (scheme, rest) = url
        .split_once("://")
        .ok_or_else(|| McpError::Connect(format!("invalid MCP endpoint URL: {url}")))?;
    match scheme.to_ascii_lowercase().as_str() {
        "https" => Ok(()),
        "http" => {
            let host = host_of(rest);
            if is_loopback_host(&host) {
                Ok(())
            } else {
                Err(McpError::Connect(format!(
                    "refusing plaintext http to non-loopback MCP host '{host}'; \
                     use https for remote endpoints"
                )))
            }
        }
        other => Err(McpError::Connect(format!(
            "unsupported MCP endpoint scheme '{other}' in {url}"
        ))),
    }
}

/// Resolve the bearer token for a server from its `auth_token_env` var name
/// using `get` (the process env in production). A missing/empty value yields
/// `None` — the transport then sends no `Authorization` header.
pub fn resolve_bearer_token(
    auth_token_env: Option<&str>,
    get: impl Fn(&str) -> Option<String>,
) -> Option<String> {
    auth_token_env
        .and_then(get)
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// The host portion of a URL `authority[/path][?query][#frag]`, lowercased,
/// with any userinfo and port stripped (IPv6 literals in `[..]` handled).
fn host_of(authority_and_path: &str) -> String {
    let authority = authority_and_path
        .split(['/', '?', '#'])
        .next()
        .unwrap_or("");
    // Drop any `userinfo@` prefix.
    let authority = authority
        .rsplit_once('@')
        .map(|(_, h)| h)
        .unwrap_or(authority);
    // IPv6 literal: `[::1]:port`.
    if let Some(rest) = authority.strip_prefix('[') {
        return rest.split(']').next().unwrap_or("").to_ascii_lowercase();
    }
    authority.split(':').next().unwrap_or("").to_ascii_lowercase()
}

fn is_loopback_host(host: &str) -> bool {
    host == "localhost"
        || host
            .parse::<std::net::IpAddr>()
            .map(|ip| ip.is_loopback())
            .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn https_always_allowed() {
        assert!(require_tls_for_non_loopback("https://mcp.example.com/sse").is_ok());
        assert!(require_tls_for_non_loopback("https://10.0.0.5:8443/mcp").is_ok());
    }

    #[test]
    fn http_allowed_only_to_loopback() {
        assert!(require_tls_for_non_loopback("http://localhost:8000/mcp").is_ok());
        assert!(require_tls_for_non_loopback("http://127.0.0.1:8000/sse").is_ok());
        assert!(require_tls_for_non_loopback("http://[::1]:8000/mcp").is_ok());
        assert!(require_tls_for_non_loopback("http://127.5.4.3/mcp").is_ok());
    }

    #[test]
    fn http_to_remote_is_rejected() {
        let err = require_tls_for_non_loopback("http://mcp.example.com/sse").unwrap_err();
        assert!(matches!(err, McpError::Connect(_)));
        assert!(require_tls_for_non_loopback("http://10.0.0.5:8000/mcp").is_err());
        // userinfo must not smuggle a loopback host past the check.
        assert!(require_tls_for_non_loopback("http://localhost@evil.com/mcp").is_err());
    }

    #[test]
    fn malformed_and_unknown_schemes_rejected() {
        assert!(require_tls_for_non_loopback("mcp.example.com/sse").is_err());
        assert!(require_tls_for_non_loopback("ftp://mcp.example.com").is_err());
    }

    #[test]
    fn token_resolution() {
        let env = |k: &str| match k {
            "TOK" => Some("  secret  ".to_string()),
            "BLANK" => Some("   ".to_string()),
            _ => None,
        };
        assert_eq!(
            resolve_bearer_token(Some("TOK"), env),
            Some("secret".to_string())
        );
        assert_eq!(resolve_bearer_token(Some("BLANK"), env), None);
        assert_eq!(resolve_bearer_token(Some("MISSING"), env), None);
        assert_eq!(resolve_bearer_token(None, env), None);
    }
}
