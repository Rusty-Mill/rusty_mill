//! The protocol-level [`Error`] object, shared by every endpoint.

use serde::{Deserialize, Serialize};

/// Machine-readable error classification defined by the ACP specification.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    /// The server failed to fulfil an otherwise valid request.
    ServerError,
    /// The request was malformed or semantically invalid.
    InvalidInput,
    /// The referenced agent, run or session does not exist.
    NotFound,
}

impl ErrorCode {
    /// The HTTP status code conventionally paired with this error code.
    pub const fn http_status(self) -> u16 {
        match self {
            ErrorCode::ServerError => 500,
            ErrorCode::InvalidInput => 422,
            ErrorCode::NotFound => 404,
        }
    }
}

/// The error payload returned by ACP endpoints.
///
/// This is the `Error` schema of the specification: a [`code`](ErrorCode), a
/// human-readable `message`, and optional structured `data`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Error {
    /// Machine-readable classification.
    pub code: ErrorCode,
    /// Human-readable description of what went wrong.
    pub message: String,
    /// Optional structured detail.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
}

impl Error {
    /// Construct an error with the given code and message.
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self { code, message: message.into(), data: None }
    }

    /// Construct a [`ErrorCode::ServerError`].
    pub fn server_error(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::ServerError, message)
    }

    /// Construct an [`ErrorCode::InvalidInput`] error.
    pub fn invalid_input(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::InvalidInput, message)
    }

    /// Construct an [`ErrorCode::NotFound`] error.
    pub fn not_found(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::NotFound, message)
    }

    /// Attach structured detail to the error.
    pub fn with_data(mut self, data: serde_json::Value) -> Self {
        self.data = Some(data);
        self
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}: {}", self.code, self.message)
    }
}

impl std::error::Error for Error {}
