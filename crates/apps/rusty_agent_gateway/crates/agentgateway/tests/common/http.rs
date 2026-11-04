//! The HTTP client the suites drive the gateway with: `rusty_request`, behind
//! the few calls the suites make, with `serde_json` bodies.
//!
//! The defaults are set to what a test wants from a plain client rather than
//! what `rusty_request` picks for a real one: no `User-Agent`, no cookie jar,
//! no proxy, no timeout and no cap on a response body. A suite asserting on the headers an upstream saw
//! then sees only the ones it set, a loopback request never goes to an ambient
//! `HTTP_PROXY`, and a slow test fails on its own deadline rather than on a
//! client's.

#![allow(dead_code, reason = "each suite uses the part of this client it needs")]

use std::fmt;
use std::time::Duration;

pub use rusty_request::{HeaderMap, Method};

/// A client with test defaults (see the module docs).
#[derive(Clone)]
pub struct Client {
    inner: rusty_request::Client,
}

impl Default for Client {
    fn default() -> Self {
        Self::new()
    }
}

impl Client {
    /// A client with test defaults, trusting the system's certificates.
    pub fn new() -> Self {
        Self::trusting(rusty_request::TrustPolicy::default())
    }

    /// A client with test defaults, trusting what `policy` says.
    pub fn trusting(policy: rusty_request::TrustPolicy) -> Self {
        let inner = rusty_request::Client::builder()
            .no_user_agent()
            .no_cookie_store()
            .no_proxy()
            .no_timeout()
            .max_response_body_len(u64::MAX)
            .trust_policy(policy)
            .build();
        Self { inner }
    }

    /// A `GET` to `url`.
    pub fn get(&self, url: impl AsRef<str>) -> RequestBuilder {
        self.request(Method::Get, url)
    }

    /// A `POST` to `url`.
    pub fn post(&self, url: impl AsRef<str>) -> RequestBuilder {
        self.request(Method::Post, url)
    }

    /// A request with any method.
    pub fn request(&self, method: Method, url: impl AsRef<str>) -> RequestBuilder {
        RequestBuilder(self.inner.request(method, url.as_ref()))
    }
}

/// `GET url` on a fresh client.
pub async fn get(url: impl AsRef<str>) -> rusty_request::Result<Response> {
    Client::new().get(url).send().await
}

/// A request being built. A step that fails (a bad URL or header) is kept and
/// reported by [`RequestBuilder::send`], so a chain reads as one expression.
pub struct RequestBuilder(rusty_request::Result<rusty_request::RequestBuilder>);

impl RequestBuilder {
    fn map(
        self,
        step: impl FnOnce(
            rusty_request::RequestBuilder,
        ) -> rusty_request::Result<rusty_request::RequestBuilder>,
    ) -> Self {
        Self(self.0.and_then(step))
    }

    /// Sets `name: value`, replacing any earlier value.
    pub fn header(self, name: impl AsRef<str>, value: impl AsRef<str>) -> Self {
        self.map(|r| r.header(name.as_ref(), value.as_ref()))
    }

    /// Adds `name: value`, keeping any earlier value.
    pub fn header_append(self, name: impl AsRef<str>, value: impl AsRef<str>) -> Self {
        self.map(|r| r.header_append(name.as_ref(), value.as_ref()))
    }

    /// Sets `Authorization: Bearer <token>`.
    pub fn bearer_auth(self, token: impl AsRef<str>) -> Self {
        self.map(|r| r.bearer_auth(token.as_ref()))
    }

    /// `value` as a JSON body, with `Content-Type: application/json`.
    pub fn json(self, value: &serde_json::Value) -> Self {
        let body = value.to_string();
        self.map(|r| {
            r.header("Content-Type", "application/json")
                .map(|r| r.body(body))
        })
    }

    /// A raw body.
    pub fn body(self, body: impl Into<rusty_request::Body>) -> Self {
        self.map(|r| Ok(r.body(body)))
    }

    /// Appends `pairs` to the URL's query.
    pub fn query(self, pairs: &[(&str, &str)]) -> Self {
        self.map(|r| Ok(r.query(pairs.iter().copied())))
    }

    /// Fails the request if it takes longer than `timeout`.
    pub fn timeout(self, timeout: Duration) -> Self {
        self.map(|r| Ok(r.timeout(timeout)))
    }

    /// Sends it and reads the whole response.
    pub async fn send(self) -> rusty_request::Result<Response> {
        Ok(Response(self.0?.send().await?))
    }
}

/// A response, read to its end.
pub struct Response(rusty_request::Response);

impl Response {
    /// The status, comparable with a plain number.
    pub fn status(&self) -> Status {
        Status(self.0.status().as_u16())
    }

    /// The headers, looked up by name in any case.
    pub fn headers(&self) -> &HeaderMap {
        self.0.headers()
    }

    /// The headers as `(name, value)` pairs, names lowercased.
    pub fn header_pairs(&self) -> Vec<(String, String)> {
        self.0
            .headers()
            .iter()
            .map(|(name, value)| (name.to_ascii_lowercase(), value.to_string()))
            .collect()
    }

    /// The body.
    pub fn bytes(&self) -> &[u8] {
        self.0.bytes()
    }

    /// The body as UTF-8.
    pub fn text(&self) -> rusty_request::Result<String> {
        self.0.text()
    }

    /// The body as JSON.
    pub fn json(&self) -> serde_json::Result<serde_json::Value> {
        serde_json::from_slice(self.0.bytes())
    }
}

/// A status code that compares with a plain number, as in
/// `assert_eq!(response.status(), 200)`.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Status(u16);

impl Status {
    /// The code.
    pub fn as_u16(self) -> u16 {
        self.0
    }

    /// Whether it is a `2xx`.
    pub fn is_success(self) -> bool {
        (200..300).contains(&self.0)
    }
}

impl PartialEq<u16> for Status {
    fn eq(&self, other: &u16) -> bool {
        self.0 == *other
    }
}

impl fmt::Display for Status {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.0, f)
    }
}

impl fmt::Debug for Status {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.0, f)
    }
}
