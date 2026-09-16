//! Everything that can go wrong talking to an OPNsense API.

/// Errors returned by [`crate::OpnsenseClient`].
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The request never got a response: DNS, connect, TLS, or I/O failure,
    /// or the response wasn't valid HTTP.
    #[error("opnsense request failed: {0}")]
    Request(#[from] rusty_request::Error),

    /// OPNsense answered with a 4xx/5xx status.
    #[error("opnsense api returned {status}: {body}")]
    Api {
        /// HTTP status code.
        status: u16,
        /// Raw response body.
        body: String,
    },

    /// The response body wasn't valid JSON.
    #[error("opnsense response was not valid json: {0}")]
    Decode(#[from] serde_json::Error),
}

/// Shorthand for `Result<T, Error>`.
pub type Result<T> = std::result::Result<T, Error>;
