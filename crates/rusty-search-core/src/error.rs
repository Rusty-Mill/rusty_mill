use rusty_err::{BoxError, Error};

/// The result type returned by every [`crate::SearchBackend`] operation.
pub type Result<T> = std::result::Result<T, SearchError>;

/// Errors that can occur while talking to a search backend.
///
/// Backends map their own error types onto these variants so callers can
/// handle failures generically, regardless of which engine is plugged in.
#[derive(Debug, Error)]
pub enum SearchError {
    #[error("index `{0}` not found")]
    IndexNotFound(String),

    #[error("index `{0}` already exists")]
    IndexAlreadyExists(String),

    #[error("document `{0}` not found in index `{1}`")]
    DocumentNotFound(String, String),

    #[error("invalid schema: {0}")]
    InvalidSchema(String),

    #[error("invalid query: {0}")]
    InvalidQuery(String),

    #[error("serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    /// Catch-all for backend-specific failures (I/O, network, the engine's
    /// own error type, etc). Backends should prefer the typed variants above
    /// when the failure maps cleanly onto one.
    ///
    /// Deliberately not `#[from]`/`#[source]`: [`BoxError`] doesn't
    /// implement [`Error`] itself (by design - see its own docs), so it
    /// can't be used as a source field. Build one with
    /// `SearchError::Backend(BoxError::new(e))`, or
    /// [`SearchError::backend_msg`] for a formatted message with no
    /// concrete error to wrap.
    #[error("backend error: {0}")]
    Backend(BoxError),
}

impl SearchError {
    /// Builds a [`SearchError::Backend`] from a formatted message, for
    /// failures with no underlying concrete error to wrap (a backend's own
    /// malformed-response or unexpected-status-code cases) - the
    /// `rusty_err`-shaped equivalent of `anyhow!("...")`.
    pub fn backend_msg(msg: impl Into<String>) -> Self {
        SearchError::Backend(BoxError::new(BackendMessage(msg.into())))
    }
}

/// A minimal `Display`+`Debug`-only error wrapping a plain message, so
/// [`SearchError::backend_msg`] has a concrete type to box - `rusty_err`
/// has no `anyhow!`-style ad-hoc-message macro.
#[derive(Debug)]
struct BackendMessage(String);

impl std::fmt::Display for BackendMessage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for BackendMessage {}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_send_sync<T: Send + Sync>() {}

    #[test]
    fn search_error_is_send_sync() {
        assert_send_sync::<SearchError>();
    }

    #[test]
    fn backend_msg_formats_the_message() {
        let err = SearchError::backend_msg("something went wrong");
        assert_eq!(err.to_string(), "backend error: something went wrong");
    }

    #[test]
    fn serialization_from_serde_json_error_chains_source() {
        let json_err = serde_json::from_str::<serde_json::Value>("not json").unwrap_err();
        let err: SearchError = json_err.into();
        assert!(matches!(err, SearchError::Serialization(_)));
    }
}
