//! A minimal, transport-agnostic HTTP request description.
//!
//! This crate never performs I/O itself (see the crate-level docs for why);
//! every endpoint-calling function instead returns a [`HttpRequest`]
//! describing exactly what to send. Hand it to whatever HTTP client you
//! trust and feed the response body to the matching `parse_response`
//! function.

use crate::client::Client;
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Method {
    Get,
    Post,
}

impl Method {
    pub fn as_str(&self) -> &'static str {
        match self {
            Method::Get => "GET",
            Method::Post => "POST",
        }
    }
}

/// Body fields that carry a raw credential or one-time secret and must
/// never appear verbatim in `Debug` output.
const SENSITIVE_BODY_FIELDS: &[&str] =
    &["client_secret", "client_assertion", "refresh_token", "code"];

/// A fully-formed HTTP request: send it verbatim with your HTTP client of
/// choice.
///
/// `Debug` is hand-written (not derived): every request in this crate that
/// authenticates a client folds its secret into either the `Authorization`
/// header (`client_secret_basic`) or the body (`client_secret_post`,
/// `client_assertion`, refresh/authorization-code grants), which would
/// otherwise defeat [`ClientSecret`](crate::client::ClientSecret)'s own
/// redacting `Debug` impl the moment it's logged as part of a request.
#[derive(Clone)]
pub struct HttpRequest {
    pub method: Method,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl fmt::Debug for HttpRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let headers: Vec<(String, String)> = self
            .headers
            .iter()
            .map(|(name, value)| {
                if name.eq_ignore_ascii_case("authorization") {
                    (name.clone(), "[redacted]".to_string())
                } else {
                    (name.clone(), value.clone())
                }
            })
            .collect();

        f.debug_struct("HttpRequest")
            .field("method", &self.method)
            .field("url", &self.url)
            .field("headers", &headers)
            .field("body", &self.redacted_body())
            .finish()
    }
}

impl HttpRequest {
    /// Best-effort redaction of [`SENSITIVE_BODY_FIELDS`] from the body for
    /// `Debug` output, using the request's own `Content-Type` header to
    /// decide whether the body is structured enough to redact by key.
    ///
    /// Falls back to a byte-count placeholder whenever the body isn't
    /// recognizable UTF-8 form/JSON data, so a body we can't cheaply and
    /// reliably parse never gets echoed verbatim.
    fn redacted_body(&self) -> String {
        if self.body.is_empty() {
            return String::new();
        }
        let Ok(text) = std::str::from_utf8(&self.body) else {
            return format!("<body present, {} bytes>", self.body.len());
        };
        let content_type = self
            .headers
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case("content-type"))
            .map(|(_, value)| value.as_str())
            .unwrap_or("");

        if content_type.contains("application/x-www-form-urlencoded") {
            if let Ok(pairs) = crate::encoding::percent::form_urldecode(text) {
                let redacted: Vec<(String, String)> = pairs
                    .into_iter()
                    .map(|(k, v)| {
                        if SENSITIVE_BODY_FIELDS.contains(&k.as_str()) {
                            (k, "[redacted]".to_string())
                        } else {
                            (k, v)
                        }
                    })
                    .collect();
                return crate::encoding::percent::form_urlencode(
                    redacted.iter().map(|(k, v)| (k.as_str(), v.as_str())),
                );
            }
        } else if content_type.contains("application/json") {
            if let Ok(mut value) = rusty_json::Value::from_json_str(text) {
                if let Some(map) = value.as_object_mut() {
                    for field in SENSITIVE_BODY_FIELDS {
                        if map.contains_key(*field) {
                            map.insert((*field).to_string(), "[redacted]".into());
                        }
                    }
                }
                return value.to_json_string();
            }
        }

        format!("<body present, {} bytes>", self.body.len())
    }

    pub(crate) fn form_post(url: impl Into<String>, body: String) -> Self {
        HttpRequest {
            method: Method::Post,
            url: url.into(),
            headers: vec![(
                "Content-Type".to_string(),
                "application/x-www-form-urlencoded".to_string(),
            )],
            body: body.into_bytes(),
        }
    }

    /// Adds client authentication to this request per RFC 6749 §2.3.1:
    /// `client_secret_basic` sets the `Authorization` header, everything
    /// else is expected to already be present in the form body.
    pub(crate) fn with_basic_auth_if_applicable(mut self, client: &Client) -> Self {
        if client.auth_method == crate::client::AuthMethod::ClientSecretBasic {
            if let Some(header) = client.basic_auth_header() {
                self.headers.push(("Authorization".to_string(), header));
            }
        }
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_redacts_authorization_header_and_form_body_secret() {
        let req = HttpRequest {
            method: Method::Post,
            url: "https://example.com/token".to_string(),
            headers: vec![
                (
                    "Authorization".to_string(),
                    "Basic Y2xpZW50X2lkOnN1cGVyc2VjcmV0".to_string(),
                ),
                (
                    "Content-Type".to_string(),
                    "application/x-www-form-urlencoded".to_string(),
                ),
            ],
            body: b"grant_type=client_credentials&client_secret=supersecret".to_vec(),
        };

        let debug = format!("{req:?}");

        assert!(!debug.contains("supersecret"));
        assert!(!debug.contains("Y2xpZW50X2lkOnN1cGVyc2VjcmV0"));
        assert!(debug.contains("[redacted]"));
        // Non-sensitive fields remain visible for debugging.
        assert!(debug.contains("grant_type=client_credentials"));
    }

    #[test]
    fn debug_redacts_json_body_secret() {
        let req = HttpRequest {
            method: Method::Post,
            url: "https://example.com/register".to_string(),
            headers: vec![("Content-Type".to_string(), "application/json".to_string())],
            body: br#"{"client_name":"demo","client_secret":"supersecret"}"#.to_vec(),
        };

        let debug = format!("{req:?}");

        assert!(!debug.contains("supersecret"));
        assert!(debug.contains("demo"));
    }

    #[test]
    fn debug_falls_back_to_placeholder_for_unrecognized_body() {
        let req = HttpRequest {
            method: Method::Post,
            url: "https://example.com/x".to_string(),
            headers: vec![],
            body: vec![0xff, 0xfe, 0x00, 0x01],
        };

        let debug = format!("{req:?}");

        assert!(debug.contains("<body present, 4 bytes>"));
    }
}
