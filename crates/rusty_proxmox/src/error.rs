//! Everything that can go wrong talking to a Proxmox VE API.

/// Errors returned by [`crate::ProxmoxClient`].
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The request never got a response: DNS, connect, TLS, or I/O failure,
    /// or the response wasn't valid HTTP.
    #[error("proxmox request failed: {0}")]
    Request(#[from] rusty_request::Error),

    /// Proxmox answered with a 4xx/5xx status. The body is included as-is --
    /// Proxmox puts the human-readable reason in an `errors` field there.
    #[error("proxmox api returned {status}: {body}")]
    Api {
        /// HTTP status code.
        status: u16,
        /// Raw response body.
        body: String,
    },

    /// The response body wasn't valid JSON.
    #[error("proxmox response was not valid json: {0}")]
    Decode(#[from] serde_json::Error),

    /// The response was valid JSON but didn't have the shape every Proxmox
    /// API response is documented to have (a top-level `data` field).
    #[error("proxmox response is missing the `data` field: {0}")]
    MissingData(String),
}

/// Shorthand for `Result<T, Error>`.
pub type Result<T> = std::result::Result<T, Error>;
