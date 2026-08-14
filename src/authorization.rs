//! The authorization request/response (RFC 6749 §4.1.1 / §4.1.2 / §4.1.2.1),
//! extended with PKCE (RFC 7636).
//!
//! Only the `code` response type is supported: the Implicit grant
//! (`response_type=token`) is removed under OAuth 2.1 because it returns
//! access tokens directly in a URL fragment with no way to bind them to
//! the requesting client, and this crate does not implement it.

use crate::client::Client;
use crate::encoding::base64::encode_url_safe_no_pad;
use crate::encoding::percent::{encode as percent_encode, form_urldecode};
use crate::error::OAuthErrorResponse;
use crate::pkce::Pkce;
use crate::rand::random_bytes;

/// Builds an authorization request URL.
pub struct AuthorizationRequest<'a> {
    authorization_endpoint: &'a str,
    client_id: &'a str,
    redirect_uri: &'a str,
    scope: Option<String>,
    state: Option<String>,
    pkce: Option<(&'a str, &'a str)>, // (code_challenge, method)
    extra_params: Vec<(String, String)>,
}

impl<'a> AuthorizationRequest<'a> {
    /// Starts building an authorization request against `authorization_endpoint`
    /// for `client`, redirecting back to `redirect_uri` (RFC 6749 §3.1.2).
    pub fn new(authorization_endpoint: &'a str, client: &'a Client, redirect_uri: &'a str) -> Self {
        AuthorizationRequest {
            authorization_endpoint,
            client_id: client.client_id.as_str(),
            redirect_uri,
            scope: None,
            state: None,
            pkce: None,
            extra_params: Vec::new(),
        }
    }

    /// Sets the requested scope (RFC 6749 §3.3), a space-delimited list.
    pub fn scope(mut self, scope: impl Into<String>) -> Self {
        self.scope = Some(scope.into());
        self
    }

    /// Sets an explicit `state` value (RFC 6749 §4.1.1 / §10.12). If not
    /// called, [`build`](Self::build) generates a fresh random one.
    pub fn state(mut self, state: impl Into<String>) -> Self {
        self.state = Some(state.into());
        self
    }

    /// Attaches a PKCE code challenge (RFC 7636 §4.3).
    pub fn pkce(mut self, pkce: &'a Pkce) -> Self {
        self.pkce = Some((&pkce.code_challenge, pkce.code_challenge_method.as_str()));
        self
    }

    /// Adds a non-standard/extension query parameter (e.g. `audience`,
    /// `prompt`, `login_hint`, `resource`).
    pub fn extra_param(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.extra_params.push((key.into(), value.into()));
        self
    }

    /// Builds the final request, generating a random `state` if one
    /// wasn't set explicitly. The caller must persist `state` (and the
    /// PKCE `code_verifier`, if used) until the callback arrives.
    pub fn build(self) -> crate::error::Result<BuiltAuthorizationRequest> {
        let state = match self.state {
            Some(s) => s,
            None => encode_url_safe_no_pad(&random_bytes(24)?),
        };

        let mut url = String::new();
        url.push_str(self.authorization_endpoint);
        url.push(if self.authorization_endpoint.contains('?') {
            '&'
        } else {
            '?'
        });
        url.push_str("response_type=code");
        url.push_str("&client_id=");
        url.push_str(&percent_encode(self.client_id));
        url.push_str("&redirect_uri=");
        url.push_str(&percent_encode(self.redirect_uri));
        url.push_str("&state=");
        url.push_str(&percent_encode(&state));

        if let Some(scope) = &self.scope {
            url.push_str("&scope=");
            url.push_str(&percent_encode(scope));
        }

        if let Some((challenge, method)) = self.pkce {
            url.push_str("&code_challenge=");
            url.push_str(&percent_encode(challenge));
            url.push_str("&code_challenge_method=");
            url.push_str(&percent_encode(method));
        }

        for (k, v) in &self.extra_params {
            url.push('&');
            url.push_str(&percent_encode(k));
            url.push('=');
            url.push_str(&percent_encode(v));
        }

        Ok(BuiltAuthorizationRequest { url, state })
    }
}

/// A completed authorization request, ready to redirect the user-agent to.
#[derive(Debug, Clone)]
pub struct BuiltAuthorizationRequest {
    pub url: String,
    /// The CSRF `state` value; compare against the callback's `state`
    /// with [`verify_state`] before trusting the returned `code`.
    pub state: String,
}

/// A successful authorization response (RFC 6749 §4.1.2): the
/// authorization `code` and echoed `state`, extracted from the redirect
/// callback's query string.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthorizationResponse {
    pub code: String,
    pub state: Option<String>,
}

/// Parses the callback redirect's query string (everything after `?`,
/// without the leading `?`) into either a successful [`AuthorizationResponse`]
/// or an [`OAuthErrorResponse`] (RFC 6749 §4.1.2.1).
pub fn parse_callback_query(query: &str) -> crate::error::Result<AuthorizationResponse> {
    let pairs = form_urldecode(query)?;

    if let Some(err) = OAuthErrorResponse::from_query_pairs(&pairs) {
        return Err(err.into());
    }

    let code = pairs
        .iter()
        .find(|(k, _)| k == "code")
        .map(|(_, v)| v.clone())
        .ok_or_else(|| crate::error::Error::Protocol("callback is missing `code`".to_string()))?;
    let state = pairs
        .iter()
        .find(|(k, _)| k == "state")
        .map(|(_, v)| v.clone());

    Ok(AuthorizationResponse { code, state })
}

/// Verifies the `state` returned in the callback matches the one
/// generated at authorization time, in constant time (RFC 6749 §10.12
/// CSRF protection). Always call this before using the returned `code`.
pub fn verify_state(expected: &str, received: Option<&str>) -> bool {
    match received {
        Some(r) => crate::crypto::hmac::constant_time_eq(expected.as_bytes(), r.as_bytes()),
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::{Client, ClientId};

    #[test]
    fn builds_url_with_pkce_and_scope() {
        let client = Client::public(ClientId::new("abc123"));
        let pkce = Pkce::generate().unwrap();
        let req = AuthorizationRequest::new(
            "https://auth.example.com/authorize",
            &client,
            "https://app.example.com/cb",
        )
        .scope("openid profile")
        .pkce(&pkce)
        .state("fixed-state")
        .build()
        .unwrap();

        assert!(req
            .url
            .starts_with("https://auth.example.com/authorize?response_type=code"));
        assert!(req.url.contains("client_id=abc123"));
        assert!(req
            .url
            .contains("redirect_uri=https%3A%2F%2Fapp.example.com%2Fcb"));
        assert!(req.url.contains("scope=openid%20profile"));
        assert!(req.url.contains(&format!(
            "code_challenge={}",
            crate::encoding::percent::encode(&pkce.code_challenge)
        )));
        assert!(req.url.contains("code_challenge_method=S256"));
        assert_eq!(req.state, "fixed-state");
    }

    #[test]
    fn generates_random_state_when_unset() {
        let client = Client::public(ClientId::new("abc"));
        let a = AuthorizationRequest::new(
            "https://auth.example.com/authorize",
            &client,
            "https://app/cb",
        )
        .build()
        .unwrap();
        let b = AuthorizationRequest::new(
            "https://auth.example.com/authorize",
            &client,
            "https://app/cb",
        )
        .build()
        .unwrap();
        assert_ne!(a.state, b.state);
        assert!(a.state.len() >= 32);
    }

    #[test]
    fn parses_success_callback() {
        let resp = parse_callback_query("code=SplxlOBeZQQYbYS6WxSbIA&state=xyz").unwrap();
        assert_eq!(resp.code, "SplxlOBeZQQYbYS6WxSbIA");
        assert_eq!(resp.state.as_deref(), Some("xyz"));
    }

    #[test]
    fn parses_error_callback() {
        let err = parse_callback_query("error=access_denied&state=xyz").unwrap_err();
        match err {
            crate::error::Error::OAuth(e) => {
                assert_eq!(e.error, crate::error::ErrorCode::AccessDenied);
                assert_eq!(e.state.as_deref(), Some("xyz"));
            }
            other => panic!("expected OAuth error, got {other:?}"),
        }
    }

    #[test]
    fn state_verification() {
        assert!(verify_state("abc", Some("abc")));
        assert!(!verify_state("abc", Some("xyz")));
        assert!(!verify_state("abc", None));
    }
}
