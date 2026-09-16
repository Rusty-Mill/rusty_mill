#![cfg_attr(not(feature = "std"), no_std)]
#![deny(missing_docs)]

//! # `rusty_wiremock`
//!
//! A `#![no_std]` + `alloc` sovereign HTTP mock server, request matcher,
//! and response generator for test harnesses in the **Rusty Mill** ecosystem.
//!
//! The matcher/template API below is still a scaffold (`MockServer::start`
//! binds nothing yet). The [`canned`] module, behind the `std` feature, is
//! the working part: a blocking `std::net` server that answers a fixed
//! sequence of canned responses, which is what `rusty_proxmox`,
//! `rusty_opnsense`, `rusty_fedora`, and `rusty_homelab_mcp`'s client tests
//! all drive their clients against (each used to carry its own identical
//! copy under `tests/support/`).

extern crate alloc;

#[cfg(feature = "std")]
pub mod canned;

use alloc::string::String;
use alloc::vec::Vec;

/// Request matcher rule.
pub struct RequestMatcher {
    method: String,
    path: String,
}

impl RequestMatcher {
    /// Matches GET requests to path.
    pub fn get(path: &str) -> Self {
        Self {
            method: String::from("GET"),
            path: String::from(path),
        }
    }

    /// Matches POST requests to path.
    pub fn post(path: &str) -> Self {
        Self {
            method: String::from("POST"),
            path: String::from(path),
        }
    }

    /// Checks if a request method and path match this rule.
    pub fn matches(&self, method: &str, path: &str) -> bool {
        self.method == method && self.path == path
    }
}

/// Mock response template.
pub struct ResponseTemplate {
    status: u16,
    body: Vec<u8>,
}

impl ResponseTemplate {
    /// Creates a new HTTP response template with status code.
    pub fn new(status: u16) -> Self {
        Self {
            status,
            body: Vec::new(),
        }
    }

    /// Sets response body to JSON string.
    pub fn set_body_json(mut self, json_str: &str) -> Self {
        self.body = json_str.as_bytes().to_vec();
        self
    }

    /// Returns HTTP status code.
    pub fn status(&self) -> u16 {
        self.status
    }

    /// Returns response body slice.
    pub fn body(&self) -> &[u8] {
        &self.body
    }
}

/// Sovereign HTTP Mock Server handle.
pub struct MockServer {
    uri: String,
}

impl MockServer {
    /// Starts a sovereign mock server instance.
    pub fn start() -> Self {
        Self {
            uri: String::from("http://127.0.0.1:18080"),
        }
    }

    /// Returns the mock server URI base string.
    pub fn uri(&self) -> &str {
        &self.uri
    }

    /// Registers a mock expectation matcher and response template.
    pub fn register(&mut self, _matcher: RequestMatcher, _response: ResponseTemplate) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mock_server_expectation() {
        let mut server = MockServer::start();
        let matcher = RequestMatcher::get("/api/v1/health");
        let response = ResponseTemplate::new(200).set_body_json("{\"status\":\"ok\"}");

        server.register(matcher, response);
        assert!(server.uri().starts_with("http://127.0.0.1"));
    }
}
