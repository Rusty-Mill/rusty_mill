//! Pushed Authorization Requests (RFC 9126).
//!
//! Instead of appending authorization parameters to a redirect URL (where
//! they can be tampered with, logged by intermediaries, or leak via
//! referrer headers), the client POSTs them directly to the authorization
//! server first and gets back a short-lived `request_uri` to redirect the
//! user-agent to instead. Considered current OAuth 2.1-era best practice.

use crate::authorization::AuthorizationRequest;
use crate::client::Client;
use crate::encoding::percent::{encode as percent_encode, form_urlencode};
use crate::error::{Error, OAuthErrorResponse, Result};
use crate::json::{self, Value};
use crate::request::HttpRequest;

/// A pushed authorization request, ready to send, plus the `state` it
/// carries -- save `state` exactly as you would with a normal
/// [`AuthorizationRequest::build`] flow, to verify it against the
/// eventual callback.
#[derive(Debug, Clone)]
pub struct PushedAuthorizationRequest {
    pub http_request: HttpRequest,
    pub state: String,
}

/// Builds the RFC 9126 §2.1 pushed authorization request: the same
/// parameters an [`AuthorizationRequest`] would put in a redirect URL
/// query string, instead POSTed as a form body to
/// `pushed_authorization_request_endpoint`, with client authentication
/// (RFC 9126 §2.1 requires the same auth as the token endpoint).
pub fn pushed_authorization_request(
    par_endpoint: &str,
    client: &Client,
    request: AuthorizationRequest,
) -> Result<PushedAuthorizationRequest> {
    let (owned_params, state) = request.resolve()?;
    let mut params: Vec<(&str, &str)> = owned_params
        .iter()
        .map(|(k, v)| (k.as_str(), v.as_str()))
        .collect();

    let uses_body_auth = client.auth_method != crate::client::AuthMethod::ClientSecretBasic;
    let secret_str;
    if uses_body_auth && client.auth_method == crate::client::AuthMethod::ClientSecretPost {
        if let Some(secret) = &client.client_secret {
            secret_str = secret.as_str().to_string();
            params.push(("client_secret", &secret_str));
        }
    }
    let assertion;
    if let Some((assertion_type, assertion_value)) = client.build_client_assertion(par_endpoint)? {
        assertion = assertion_value;
        params.push(("client_assertion_type", assertion_type));
        params.push(("client_assertion", &assertion));
    }

    let body = form_urlencode(params);
    let http_request =
        HttpRequest::form_post(par_endpoint, body).with_basic_auth_if_applicable(client);

    Ok(PushedAuthorizationRequest {
        http_request,
        state,
    })
}

/// The RFC 9126 §2.2 success response: a `request_uri` referencing the
/// pushed parameters, valid for `expires_in` seconds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PushedAuthorizationResponse {
    pub request_uri: String,
    pub expires_in: u64,
}

/// Parses a pushed authorization request response. Per RFC 9126 §2.2 a
/// successful push returns `201 Created`; this accepts any `2xx` to be
/// lenient with servers that return a plain `200 OK`. Anything else is
/// parsed as an RFC 6749 §5.2-style error body (RFC 9126 §2.3).
pub fn parse_pushed_authorization_response(
    status: u16,
    body: &str,
) -> Result<PushedAuthorizationResponse> {
    let value = json::parse(body)?;

    if !(200..300).contains(&status) {
        if let Some(err) = OAuthErrorResponse::from_json(&value) {
            return Err(err.into());
        }
        return Err(Error::Protocol(format!(
            "pushed authorization request endpoint returned HTTP {status} with an unrecognized error body"
        )));
    }

    let request_uri = value
        .get("request_uri")
        .and_then(Value::as_str)
        .ok_or_else(|| Error::Protocol("PAR response missing `request_uri`".to_string()))?
        .to_string();
    let expires_in = value
        .get("expires_in")
        .and_then(Value::as_u64)
        .ok_or_else(|| Error::Protocol("PAR response missing `expires_in`".to_string()))?;

    Ok(PushedAuthorizationResponse {
        request_uri,
        expires_in,
    })
}

/// Builds the short follow-up authorization URL (RFC 9126 §4): just
/// `client_id` and the `request_uri` obtained from a successful push,
/// instead of the full parameter set.
pub fn build_par_authorization_url(
    authorization_endpoint: &str,
    client: &Client,
    request_uri: &str,
) -> String {
    format!(
        "{}{}client_id={}&request_uri={}",
        authorization_endpoint,
        if authorization_endpoint.contains('?') {
            '&'
        } else {
            '?'
        },
        percent_encode(client.client_id.as_str()),
        percent_encode(request_uri)
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::{AuthMethod, Client, ClientId, ClientSecret};
    use crate::pkce::Pkce;

    #[test]
    fn builds_pushed_request_with_basic_auth() {
        let client = Client::confidential(ClientId::new("s6BhdRkqt3"), ClientSecret::new("secret"));
        let pkce = Pkce::generate().unwrap();
        let request = AuthorizationRequest::new(
            "https://auth.example.com/authorize",
            &client,
            "https://app.example.com/cb",
        )
        .scope("openid")
        .pkce(&pkce)
        .state("fixed-state");

        let pushed =
            pushed_authorization_request("https://auth.example.com/as/par", &client, request)
                .unwrap();
        assert_eq!(pushed.state, "fixed-state");

        let body = String::from_utf8(pushed.http_request.body).unwrap();
        assert!(body.contains("response_type=code"));
        assert!(body.contains("client_id=s6BhdRkqt3"));
        assert!(body.contains("state=fixed-state"));
        assert!(body.contains("scope=openid"));
        assert!(!body.contains("client_secret")); // sent via Basic header, not body
        assert!(pushed
            .http_request
            .headers
            .iter()
            .any(|(k, v)| k == "Authorization" && v.starts_with("Basic ")));
    }

    #[test]
    fn client_secret_jwt_attaches_assertion() {
        let client = Client::confidential(ClientId::new("id"), ClientSecret::new("s3cr3t"))
            .with_auth_method(AuthMethod::ClientSecretJwt);
        let request = AuthorizationRequest::new(
            "https://auth.example.com/authorize",
            &client,
            "https://app.example.com/cb",
        );
        let pushed =
            pushed_authorization_request("https://auth.example.com/as/par", &client, request)
                .unwrap();
        let body = String::from_utf8(pushed.http_request.body).unwrap();
        assert!(body.contains("client_assertion="));
    }

    #[test]
    fn parses_success_response() {
        let resp = parse_pushed_authorization_response(
            201,
            r#"{"request_uri":"urn:ietf:params:oauth:request_uri:6esc_11ACC5bwc014ltc14eY22c","expires_in":90}"#,
        )
        .unwrap();
        assert_eq!(
            resp.request_uri,
            "urn:ietf:params:oauth:request_uri:6esc_11ACC5bwc014ltc14eY22c"
        );
        assert_eq!(resp.expires_in, 90);
    }

    #[test]
    fn parses_error_response() {
        let err = parse_pushed_authorization_response(
            400,
            r#"{"error":"invalid_request","error_description":"redirect_uri missing"}"#,
        )
        .unwrap_err();
        match err {
            Error::OAuth(e) => assert_eq!(e.error, crate::error::ErrorCode::InvalidRequest),
            other => panic!("expected OAuth error, got {other:?}"),
        }
    }

    #[test]
    fn builds_short_authorization_url() {
        let client = Client::public(ClientId::new("s6BhdRkqt3"));
        let url = build_par_authorization_url(
            "https://as.example.com/authorize",
            &client,
            "urn:ietf:params:oauth:request_uri:6esc_11ACC5bwc014ltc14eY22c",
        );
        assert_eq!(
            url,
            "https://as.example.com/authorize?client_id=s6BhdRkqt3&request_uri=urn%3Aietf%3Aparams%3Aoauth%3Arequest_uri%3A6esc_11ACC5bwc014ltc14eY22c"
        );
    }
}
