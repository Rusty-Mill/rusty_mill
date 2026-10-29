//! The outbound HTTP client shared by the router's side calls
//! (`[moderation]`, `[web_search]`, `[webhook]`), on `rusty_request`.
//!
//! `rusty_request` runs on `rusty_tokio`. A host that drives the router
//! from a tokio runtime (rp-server) must call
//! `rusty_tokio::runtime::install_fallback()` once at startup.

use std::time::Duration;

use serde::de::DeserializeOwned;

/// A client with `timeout` per request and no cookie store -- each side
/// call is a stateless API request, as it was under reqwest's defaults.
pub(crate) fn client(timeout: Duration) -> rusty_request::Client {
    rusty_request::Client::builder()
        .timeout(timeout)
        .no_cookie_store()
        .build()
}

/// Decodes `resp`'s body as JSON into `T`.
pub(crate) fn json<T: DeserializeOwned>(resp: &rusty_request::Response) -> Result<T, String> {
    serde_json::from_slice(resp.bytes()).map_err(|e| e.to_string())
}

/// Installs `rusty_tokio`'s fallback runtime so a `#[tokio::test]` can
/// drive `rusty_request` (wiremock needs tokio, so these tests stay on it).
#[cfg(test)]
pub(crate) fn install_fallback_for_test() {
    rusty_tokio::runtime::install_fallback().expect("rusty_tokio fallback runtime should start");
}
