//! Crate-wide error types, including the standard OAuth error responses
//! defined by RFC 6749 §4.1.2.1 / §5.2, RFC 8628 §3.5, and RFC 7009 §2.2.1.

use crate::json::{self, Value};
use std::fmt;

/// The standard `error` codes used across the OAuth authorization and
/// token endpoints, plus the device authorization grant's polling codes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ErrorCode {
    /// RFC 6749 §4.1.2.1 / §5.2
    InvalidRequest,
    InvalidClient,
    InvalidGrant,
    UnauthorizedClient,
    UnsupportedGrantType,
    UnsupportedResponseType,
    InvalidScope,
    AccessDenied,
    ServerError,
    TemporarilyUnavailable,
    /// RFC 7636 §4.6 (PKCE-specific extension of `invalid_grant` semantics
    /// is reused; `invalid_request` is used when a required PKCE
    /// parameter is missing).
    /// RFC 8628 §3.5 device authorization grant polling responses.
    AuthorizationPending,
    SlowDown,
    ExpiredToken,
    /// RFC 9449 §8: the server wants a fresh DPoP proof carrying a
    /// `nonce` claim, provided via the accompanying `DPoP-Nonce` response
    /// header. Retry with that nonce attached to the next proof.
    UseDpopNonce,
    /// Any error code not covered above (extension grants, vendor codes).
    Other(String),
}

impl ErrorCode {
    pub fn as_str(&self) -> &str {
        match self {
            ErrorCode::InvalidRequest => "invalid_request",
            ErrorCode::InvalidClient => "invalid_client",
            ErrorCode::InvalidGrant => "invalid_grant",
            ErrorCode::UnauthorizedClient => "unauthorized_client",
            ErrorCode::UnsupportedGrantType => "unsupported_grant_type",
            ErrorCode::UnsupportedResponseType => "unsupported_response_type",
            ErrorCode::InvalidScope => "invalid_scope",
            ErrorCode::AccessDenied => "access_denied",
            ErrorCode::ServerError => "server_error",
            ErrorCode::TemporarilyUnavailable => "temporarily_unavailable",
            ErrorCode::AuthorizationPending => "authorization_pending",
            ErrorCode::SlowDown => "slow_down",
            ErrorCode::ExpiredToken => "expired_token",
            ErrorCode::UseDpopNonce => "use_dpop_nonce",
            ErrorCode::Other(s) => s,
        }
    }
}

impl From<&str> for ErrorCode {
    fn from(s: &str) -> Self {
        match s {
            "invalid_request" => ErrorCode::InvalidRequest,
            "invalid_client" => ErrorCode::InvalidClient,
            "invalid_grant" => ErrorCode::InvalidGrant,
            "unauthorized_client" => ErrorCode::UnauthorizedClient,
            "unsupported_grant_type" => ErrorCode::UnsupportedGrantType,
            "unsupported_response_type" => ErrorCode::UnsupportedResponseType,
            "invalid_scope" => ErrorCode::InvalidScope,
            "access_denied" => ErrorCode::AccessDenied,
            "server_error" => ErrorCode::ServerError,
            "temporarily_unavailable" => ErrorCode::TemporarilyUnavailable,
            "authorization_pending" => ErrorCode::AuthorizationPending,
            "slow_down" => ErrorCode::SlowDown,
            "expired_token" => ErrorCode::ExpiredToken,
            "use_dpop_nonce" => ErrorCode::UseDpopNonce,
            other => ErrorCode::Other(other.to_string()),
        }
    }
}

impl fmt::Display for ErrorCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// A standard OAuth error response, as returned from the authorization
/// endpoint (via redirect query parameters) or the token endpoint (as a
/// JSON body), per RFC 6749 §4.1.2.1 / §5.2.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OAuthErrorResponse {
    pub error: ErrorCode,
    pub error_description: Option<String>,
    pub error_uri: Option<String>,
    /// Echoes the `state` value, present on authorization-endpoint errors.
    pub state: Option<String>,
}

impl fmt::Display for OAuthErrorResponse {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.error)?;
        if let Some(desc) = &self.error_description {
            write!(f, ": {desc}")?;
        }
        Ok(())
    }
}

impl std::error::Error for OAuthErrorResponse {}

impl OAuthErrorResponse {
    /// Parses an OAuth error response from a JSON token-endpoint body.
    pub fn from_json(value: &Value) -> Option<Self> {
        let error = value.get("error")?.as_str()?;
        Some(OAuthErrorResponse {
            error: ErrorCode::from(error),
            error_description: value
                .get("error_description")
                .and_then(Value::as_str)
                .map(str::to_string),
            error_uri: value
                .get("error_uri")
                .and_then(Value::as_str)
                .map(str::to_string),
            state: value
                .get("state")
                .and_then(Value::as_str)
                .map(str::to_string),
        })
    }

    /// Parses an OAuth error response from decoded redirect query
    /// parameters (authorization endpoint error redirects).
    pub fn from_query_pairs(pairs: &[(String, String)]) -> Option<Self> {
        let find = |key: &str| pairs.iter().find(|(k, _)| k == key).map(|(_, v)| v.clone());
        let error = find("error")?;
        Some(OAuthErrorResponse {
            error: ErrorCode::from(error.as_str()),
            error_description: find("error_description"),
            error_uri: find("error_uri"),
            state: find("state"),
        })
    }
}

/// The crate's top-level error type.
#[derive(Debug)]
pub enum Error {
    /// A standard OAuth error response from the authorization or token
    /// endpoint.
    OAuth(OAuthErrorResponse),
    /// The server returned malformed or unexpected data.
    Protocol(String),
    /// A JSON document could not be parsed.
    Json(json::ParseError),
    /// Base64 decoding failed.
    Base64(crate::encoding::base64::DecodeError),
    /// Percent-decoding failed.
    Percent(crate::encoding::percent::DecodeError),
    /// The OS CSPRNG could not be read.
    Rand(crate::rand::RandError),
    /// A caller-supplied value failed local validation (e.g. `state`
    /// mismatch, expired JWT, unsupported algorithm).
    Validation(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::OAuth(e) => write!(f, "OAuth error: {e}"),
            Error::Protocol(msg) => write!(f, "protocol error: {msg}"),
            Error::Json(e) => write!(f, "{e}"),
            Error::Base64(e) => write!(f, "{e}"),
            Error::Percent(e) => write!(f, "{e}"),
            Error::Rand(e) => write!(f, "{e}"),
            Error::Validation(msg) => write!(f, "validation error: {msg}"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::OAuth(e) => Some(e),
            Error::Json(e) => Some(e),
            Error::Base64(e) => Some(e),
            Error::Percent(e) => Some(e),
            Error::Rand(e) => Some(e),
            _ => None,
        }
    }
}

impl From<OAuthErrorResponse> for Error {
    fn from(e: OAuthErrorResponse) -> Self {
        Error::OAuth(e)
    }
}

impl From<json::ParseError> for Error {
    fn from(e: json::ParseError) -> Self {
        Error::Json(e)
    }
}

impl From<crate::encoding::base64::DecodeError> for Error {
    fn from(e: crate::encoding::base64::DecodeError) -> Self {
        Error::Base64(e)
    }
}

impl From<crate::encoding::percent::DecodeError> for Error {
    fn from(e: crate::encoding::percent::DecodeError) -> Self {
        Error::Percent(e)
    }
}

impl From<crate::rand::RandError> for Error {
    fn from(e: crate::rand::RandError) -> Self {
        Error::Rand(e)
    }
}

pub type Result<T> = std::result::Result<T, Error>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_json_error_response() {
        let json = json::parse(
            r#"{"error":"invalid_grant","error_description":"The authorization code has expired"}"#,
        )
        .unwrap();
        let err = OAuthErrorResponse::from_json(&json).unwrap();
        assert_eq!(err.error, ErrorCode::InvalidGrant);
        assert_eq!(
            err.error_description.as_deref(),
            Some("The authorization code has expired")
        );
    }

    #[test]
    fn parses_query_error_response() {
        let pairs = vec![
            ("error".to_string(), "access_denied".to_string()),
            ("state".to_string(), "xyz".to_string()),
        ];
        let err = OAuthErrorResponse::from_query_pairs(&pairs).unwrap();
        assert_eq!(err.error, ErrorCode::AccessDenied);
        assert_eq!(err.state.as_deref(), Some("xyz"));
    }

    #[test]
    fn unknown_error_code_preserved() {
        assert_eq!(
            ErrorCode::from("some_vendor_code").as_str(),
            "some_vendor_code"
        );
    }
}
