//! Error types for the runtime, plus a convenience error for tool bodies.

use rmcp::model::ErrorData;

/// Something went wrong starting or running the server.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ServeError {
    /// Binding the HTTP listener failed.
    #[error("failed to bind {bind}: {source}")]
    Bind {
        /// The address we tried to bind.
        bind: String,
        /// Underlying I/O error.
        #[source]
        source: std::io::Error,
    },

    /// The handler factory refused to produce a handler.
    #[error("failed to construct server handler: {0}")]
    Handler(#[source] std::io::Error),

    /// The transport failed while serving.
    #[error("transport error: {0}")]
    Transport(#[source] Box<dyn std::error::Error + Send + Sync>),

    /// Generic I/O failure.
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

/// A failure inside a tool body, as a shorthand for building [`ErrorData`].
///
/// Declare tools as `Result<T, ErrorData>` and use `?` — the [`From`] impl
/// below picks the right JSON-RPC code, so you never spell one out:
///
/// ```
/// use rmcp::model::ErrorData;
/// use rusty_mcp::ToolError;
///
/// fn halve(n: i64) -> Result<i64, ErrorData> {
///     if n % 2 != 0 {
///         return Err(ToolError::invalid("expected an even number").into());
///     }
///     Ok(n / 2)
/// }
///
/// assert!(halve(3).is_err());
/// assert_eq!(halve(4).unwrap(), 2);
/// ```
///
/// `Invalid` maps to `InvalidParams`, `Failed` to `InternalError`.
///
/// Mind the distinction the spec draws. A *protocol* error — this type — says
/// the call could not be processed at all. A tool that ran fine but produced a
/// domain-level failure ("no such user") should instead return a normal result
/// with `isError: true`, so the model can see the failure and react to it.
#[derive(Debug, thiserror::Error)]
pub enum ToolError {
    /// The arguments were structurally fine but semantically unusable.
    #[error("{0}")]
    Invalid(String),

    /// The tool failed to complete.
    #[error("{0}")]
    Failed(String),
}

impl ToolError {
    /// Build an [`ToolError::Invalid`].
    pub fn invalid(msg: impl Into<String>) -> Self {
        Self::Invalid(msg.into())
    }

    /// Build a [`ToolError::Failed`].
    pub fn failed(msg: impl Into<String>) -> Self {
        Self::Failed(msg.into())
    }
}

impl From<ToolError> for ErrorData {
    fn from(err: ToolError) -> Self {
        match err {
            ToolError::Invalid(msg) => ErrorData::invalid_params(msg, None),
            ToolError::Failed(msg) => ErrorData::internal_error(msg, None),
        }
    }
}
