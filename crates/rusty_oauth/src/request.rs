//! A minimal, transport-agnostic HTTP request description.
//!
//! This crate never performs I/O itself (see the crate-level docs for why);
//! every endpoint-calling function instead returns a [`HttpRequest`]
//! describing exactly what to send. Hand it to whatever HTTP client you
//! trust and feed the response body to the matching `parse_response`
//! function.

use crate::client::Client;

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

/// A fully-formed HTTP request: send it verbatim with your HTTP client of
/// choice.
#[derive(Debug, Clone)]
pub struct HttpRequest {
    pub method: Method,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl HttpRequest {
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
