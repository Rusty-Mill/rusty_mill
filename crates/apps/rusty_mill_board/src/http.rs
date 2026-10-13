//! A blocking JSON-over-HTTP helper on `rusty_request`, for adapters that run
//! on plain threads (GitHub sync, the runner's board client).
//!
//! The async client runs on a private current-thread runtime, so call it from
//! a plain thread, never from inside another runtime. One connection per
//! request: calls are seconds or minutes apart, so a pooled connection would
//! be stale (closed by the server) by the next one, and a request landing on
//! it fails.

use std::time::Duration;

use rusty_request::Client;
use tokio::runtime::{Builder, Runtime};

/// A blocking HTTP client.
pub struct Http {
    client: Client,
    rt: Runtime,
}

fn text(e: impl std::fmt::Display) -> String {
    e.to_string()
}

impl Http {
    /// A client with a per-request timeout.
    ///
    /// # Errors
    /// If the private runtime cannot start.
    pub fn new(timeout: Duration) -> Result<Self, String> {
        let rt = Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(text)?;
        Ok(Self {
            client: Client::builder().timeout(timeout).no_pool().build(),
            rt,
        })
    }

    /// One request; returns `(status, body)` whatever the status.
    ///
    /// # Errors
    /// On a transport failure or an invalid header.
    pub fn send(
        &self,
        method: &str,
        url: &str,
        headers: &[(&str, &str)],
        body: Option<String>,
    ) -> Result<(u16, String), String> {
        self.rt.block_on(async {
            let mut req = match method {
                "POST" => self.client.post(url),
                "PATCH" => self.client.patch(url),
                _ => self.client.get(url),
            }
            .map_err(text)?;
            for (k, v) in headers {
                req = req.header(k, v).map_err(text)?;
            }
            if let Some(b) = body {
                req = req
                    .header("Content-Type", "application/json")
                    .map_err(text)?
                    .body(b);
            }
            let response = req.send().await.map_err(text)?;
            let status = response.status().as_u16();
            Ok((status, response.text().map_err(text)?))
        })
    }
}
