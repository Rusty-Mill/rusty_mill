//! The Device Authorization Grant (RFC 8628), for input-constrained
//! devices (smart TVs, CLIs) that direct the user to authorize on a
//! secondary device.

use crate::client::Client;
use crate::encoding::percent::form_urlencode;
use crate::error::{Error, Result};
use crate::json::{self, Value};
use crate::request::HttpRequest;
use crate::token::{parse_token_response, TokenResponse};

/// Builds the RFC 8628 §3.1 device authorization request.
pub fn device_authorization_request(
    device_authorization_endpoint: &str,
    client: &Client,
    scope: Option<&str>,
) -> HttpRequest {
    let mut params = vec![("client_id", client.client_id.as_str())];
    if let Some(scope) = scope {
        params.push(("scope", scope));
    }
    let body = form_urlencode(params);
    HttpRequest::form_post(device_authorization_endpoint, body)
}

/// The RFC 8628 §3.2 device authorization response.
#[derive(Debug, Clone)]
pub struct DeviceAuthorizationResponse {
    pub device_code: String,
    pub user_code: String,
    pub verification_uri: String,
    /// `verification_uri_complete`, when the server provides it: a
    /// single URI that pre-fills `user_code` (RFC 8628 §3.3.1).
    pub verification_uri_complete: Option<String>,
    pub expires_in: u64,
    /// Minimum seconds to wait between polls (RFC 8628 §3.2); defaults
    /// to 5 when the server omits it, per §3.5.
    pub interval: u64,
}

/// Parses a device authorization response body.
pub fn parse_device_authorization_response(
    status: u16,
    body: &str,
) -> Result<DeviceAuthorizationResponse> {
    if status != 200 {
        return Err(Error::Protocol(format!(
            "device authorization endpoint returned HTTP {status}"
        )));
    }
    let value = json::parse(body)?;
    let field = |key: &str| -> Result<String> {
        value
            .get(key)
            .and_then(Value::as_str)
            .map(str::to_string)
            .ok_or_else(|| {
                Error::Protocol(format!("device authorization response missing `{key}`"))
            })
    };

    Ok(DeviceAuthorizationResponse {
        device_code: field("device_code")?,
        user_code: field("user_code")?,
        verification_uri: field("verification_uri")?,
        verification_uri_complete: value
            .get("verification_uri_complete")
            .and_then(Value::as_str)
            .map(str::to_string),
        expires_in: value
            .get("expires_in")
            .and_then(Value::as_u64)
            .ok_or_else(|| {
                Error::Protocol("device authorization response missing `expires_in`".to_string())
            })?,
        interval: value.get("interval").and_then(Value::as_u64).unwrap_or(5),
    })
}

/// Builds the RFC 8628 §3.4 polling request against the token endpoint.
pub fn poll_request(
    token_endpoint: &str,
    client: &Client,
    device_code: &str,
) -> Result<HttpRequest> {
    crate::token::device_code_request(token_endpoint, client, device_code)
}

/// The result of one poll of the token endpoint during a device flow.
#[derive(Debug, Clone)]
pub enum PollOutcome {
    /// The user has authorized; here is the token.
    Success(TokenResponse),
    /// RFC 8628 §3.5: keep polling, no user action yet.
    AuthorizationPending,
    /// RFC 8628 §3.5: the client is polling too fast -- increase the
    /// interval by 5 seconds and continue.
    SlowDown,
    /// RFC 8628 §3.5: `device_code` expired; the flow must be restarted.
    ExpiredToken,
    /// RFC 8628 §3.5: the user denied the request.
    AccessDenied,
}

/// Parses one poll response, distinguishing the device-flow-specific
/// pending/slow-down/expired states from a genuine success or fatal
/// error (RFC 8628 §3.5).
pub fn parse_poll_response(status: u16, body: &str) -> Result<PollOutcome> {
    if status == 200 {
        return Ok(PollOutcome::Success(parse_token_response(status, body)?));
    }

    match parse_token_response(status, body) {
        Err(Error::OAuth(e)) => match e.error {
            crate::error::ErrorCode::AuthorizationPending => Ok(PollOutcome::AuthorizationPending),
            crate::error::ErrorCode::SlowDown => Ok(PollOutcome::SlowDown),
            crate::error::ErrorCode::ExpiredToken => Ok(PollOutcome::ExpiredToken),
            crate::error::ErrorCode::AccessDenied => Ok(PollOutcome::AccessDenied),
            _ => Err(Error::OAuth(e)),
        },
        Err(other) => Err(other),
        Ok(resp) => Ok(PollOutcome::Success(resp)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::{Client, ClientId};

    #[test]
    fn parses_device_authorization_response() {
        let json = r#"{
            "device_code": "GmRhmhcxhwAzkoEqiMEg_DnyEysNkuNhszIySk9eS",
            "user_code": "WDJB-MJHT",
            "verification_uri": "https://example.com/device",
            "verification_uri_complete": "https://example.com/device?user_code=WDJB-MJHT",
            "expires_in": 1800,
            "interval": 5
        }"#;
        let resp = parse_device_authorization_response(200, json).unwrap();
        assert_eq!(resp.user_code, "WDJB-MJHT");
        assert_eq!(resp.interval, 5);
    }

    #[test]
    fn defaults_interval_when_absent() {
        let json = r#"{
            "device_code": "x",
            "user_code": "y",
            "verification_uri": "https://example.com/device",
            "expires_in": 1800
        }"#;
        let resp = parse_device_authorization_response(200, json).unwrap();
        assert_eq!(resp.interval, 5);
    }

    #[test]
    fn poll_pending_and_slow_down_are_not_errors() {
        let pending = parse_poll_response(400, r#"{"error":"authorization_pending"}"#).unwrap();
        assert!(matches!(pending, PollOutcome::AuthorizationPending));

        let slow = parse_poll_response(400, r#"{"error":"slow_down"}"#).unwrap();
        assert!(matches!(slow, PollOutcome::SlowDown));

        let expired = parse_poll_response(400, r#"{"error":"expired_token"}"#).unwrap();
        assert!(matches!(expired, PollOutcome::ExpiredToken));
    }

    #[test]
    fn poll_success() {
        let json = r#"{"access_token":"tok","token_type":"Bearer"}"#;
        let outcome = parse_poll_response(200, json).unwrap();
        assert!(matches!(outcome, PollOutcome::Success(_)));
    }

    #[test]
    fn poll_fatal_error_propagates() {
        let err = parse_poll_response(400, r#"{"error":"invalid_grant"}"#).unwrap_err();
        assert!(matches!(err, Error::OAuth(_)));
    }

    #[test]
    fn device_authorization_request_includes_scope() {
        let client = Client::public(ClientId::new("device-client"));
        let req = device_authorization_request(
            "https://auth.example.com/device_authorization",
            &client,
            Some("offline_access"),
        );
        let body = String::from_utf8(req.body).unwrap();
        assert!(body.contains("client_id=device-client"));
        assert!(body.contains("scope=offline_access"));
    }
}
