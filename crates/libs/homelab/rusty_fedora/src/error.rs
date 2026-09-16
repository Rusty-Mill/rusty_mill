//! Everything that can go wrong talking to a `rusty_fedora_agent`.

/// Errors returned by [`crate::FedoraAgentClient`].
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The request never got a response: DNS, connect, or I/O failure, or
    /// the response wasn't valid HTTP.
    #[error("rusty_fedora_agent request failed: {0}")]
    Request(#[from] rusty_request::Error),

    /// The agent answered with a 4xx/5xx status. `body` is the agent's
    /// own `{"error": "..."}` JSON, passed through as raw text.
    #[error("rusty_fedora_agent returned {status}: {body}")]
    Api {
        /// HTTP status code.
        status: u16,
        /// Raw response body.
        body: String,
    },

    /// The response body wasn't valid JSON.
    #[error("rusty_fedora_agent response was not valid json: {0}")]
    Decode(#[from] serde_json::Error),
}

/// Shorthand for `Result<T, Error>`.
pub type Result<T> = std::result::Result<T, Error>;
