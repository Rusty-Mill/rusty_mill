//! Error types for the framework and the JSON-RPC / LSP error code constants.
//!
//! Handlers return [`Result<T>`]. The common case is to construct a
//! [`Error`] with one of the helper constructors ([`Error::invalid_params`],
//! [`Error::method_not_found`], …); when a request handler returns such an
//! error the framework turns it into a JSON-RPC error response automatically.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fmt;

/// Standard JSON-RPC and LSP error codes.
///
/// JSON-RPC reserves `-32768..=-32000`. LSP carves additional codes out of
/// that range; the protocol-specific ones live just below `-32800`.
pub mod codes {
    /// Invalid JSON was received by the server.
    pub const PARSE_ERROR: i64 = -32700;
    /// The JSON sent is not a valid Request object.
    pub const INVALID_REQUEST: i64 = -32600;
    /// The method does not exist or is not available.
    pub const METHOD_NOT_FOUND: i64 = -32601;
    /// Invalid method parameters.
    pub const INVALID_PARAMS: i64 = -32602;
    /// Internal JSON-RPC error.
    pub const INTERNAL_ERROR: i64 = -32603;

    /// Server received a request before the `initialize` request.
    pub const SERVER_NOT_INITIALIZED: i64 = -32002;
    /// Unknown error (reserved by LSP).
    pub const UNKNOWN_ERROR: i64 = -32001;

    /// A request failed but the server is otherwise healthy (LSP 3.17).
    pub const REQUEST_FAILED: i64 = -32803;
    /// The server cancelled the request (LSP 3.17).
    pub const SERVER_CANCELLED: i64 = -32802;
    /// Content was modified out from under an in-flight request (LSP 3.16).
    pub const CONTENT_MODIFIED: i64 = -32801;
    /// The client cancelled the request via `$/cancelRequest` (LSP 3.16).
    pub const REQUEST_CANCELLED: i64 = -32800;
}

/// A JSON-RPC error object, as it appears on the wire inside an error response.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResponseError {
    /// A number indicating the error type that occurred.
    pub code: i64,
    /// A short, human-readable description of the error.
    pub message: String,
    /// Optional structured detail about the error.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

/// The crate-wide error type.
///
/// Transport and serialization failures are kept distinct from
/// [`ResponseError`] (errors meant to travel back to the client) so the server
/// loop can decide what to surface versus what to treat as fatal.
#[derive(Debug)]
pub enum Error {
    /// An I/O error from the underlying transport.
    Io(std::io::Error),
    /// A (de)serialization error.
    Serde(serde_json::Error),
    /// A protocol-level violation (bad framing, malformed message, …).
    Protocol(String),
    /// An error destined to be returned to the client as a JSON-RPC error.
    Response(ResponseError),
}

/// Crate result alias. The error type defaults to [`Error`] but stays
/// overridable for call sites that want a more precise error.
pub type Result<T, E = Error> = std::result::Result<T, E>;

impl Error {
    /// Construct a [`Error::Response`] from a raw code and message.
    pub fn response(code: i64, message: impl Into<String>) -> Self {
        Error::Response(ResponseError {
            code,
            message: message.into(),
            data: None,
        })
    }

    /// A protocol-level violation that is not safe to attribute to one request.
    pub fn protocol(message: impl Into<String>) -> Self {
        Error::Protocol(message.into())
    }

    /// `-32601` the requested method has no handler.
    pub fn method_not_found(message: impl Into<String>) -> Self {
        Error::response(codes::METHOD_NOT_FOUND, message)
    }

    /// `-32602` the params could not be understood by the handler.
    pub fn invalid_params(message: impl Into<String>) -> Self {
        Error::response(codes::INVALID_PARAMS, message)
    }

    /// `-32600` the request was structurally invalid for the current state.
    pub fn invalid_request(message: impl Into<String>) -> Self {
        Error::response(codes::INVALID_REQUEST, message)
    }

    /// `-32603` an unexpected internal failure.
    pub fn internal(message: impl Into<String>) -> Self {
        Error::response(codes::INTERNAL_ERROR, message)
    }

    /// `-32002` a request arrived before `initialize` completed.
    pub fn server_not_initialized() -> Self {
        Error::response(codes::SERVER_NOT_INITIALIZED, "server not initialized")
    }

    /// `-32800` the request was cancelled.
    pub fn request_cancelled() -> Self {
        Error::response(codes::REQUEST_CANCELLED, "request cancelled")
    }

    /// `-32801` the document changed out from under this request; the
    /// client should re-issue it against the new content. The standard
    /// reply when a handler detects it raced a `didChange` (e.g. via the
    /// version guard in [`crate::Documents`]).
    pub fn content_modified() -> Self {
        Error::response(codes::CONTENT_MODIFIED, "content modified")
    }

    /// `-32802` the server cancelled the request itself (LSP 3.17), e.g.
    /// because it is too busy; the client may re-send it later.
    pub fn server_cancelled() -> Self {
        Error::response(codes::SERVER_CANCELLED, "server cancelled the request")
    }

    /// `-32803` the request failed for a reason that is not the client's
    /// fault and not a crash — the server is otherwise healthy (LSP 3.17).
    pub fn request_failed(message: impl Into<String>) -> Self {
        Error::response(codes::REQUEST_FAILED, message)
    }

    /// Attach structured `data` to a response error. No-op for non-response
    /// variants.
    #[must_use]
    pub fn with_data(self, data: Value) -> Self {
        match self {
            Error::Response(mut e) => {
                e.data = Some(data);
                Error::Response(e)
            }
            other => other,
        }
    }

    /// Convert any error into the [`ResponseError`] that should be sent back to
    /// the client. Transport/serialization failures collapse to
    /// `INTERNAL_ERROR` since they cannot be meaningfully actioned by the peer.
    pub fn into_response_error(self) -> ResponseError {
        match self {
            Error::Response(e) => e,
            Error::Serde(e) => ResponseError {
                code: codes::INVALID_PARAMS,
                message: e.to_string(),
                data: None,
            },
            Error::Io(e) => ResponseError {
                code: codes::INTERNAL_ERROR,
                message: e.to_string(),
                data: None,
            },
            Error::Protocol(message) => ResponseError {
                code: codes::INTERNAL_ERROR,
                message,
                data: None,
            },
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Io(e) => write!(f, "io error: {e}"),
            Error::Serde(e) => write!(f, "serialization error: {e}"),
            Error::Protocol(m) => write!(f, "protocol error: {m}"),
            Error::Response(e) => write!(f, "response error {}: {}", e.code, e.message),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Io(e) => Some(e),
            Error::Serde(e) => Some(e),
            _ => None,
        }
    }
}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::Io(e)
    }
}

impl From<serde_json::Error> for Error {
    fn from(e: serde_json::Error) -> Self {
        Error::Serde(e)
    }
}

impl From<ResponseError> for Error {
    fn from(e: ResponseError) -> Self {
        Error::Response(e)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn helper_constructors_map_to_their_codes() {
        for (error, code) in [
            (Error::content_modified(), codes::CONTENT_MODIFIED),
            (Error::server_cancelled(), codes::SERVER_CANCELLED),
            (Error::request_failed("x"), codes::REQUEST_FAILED),
            (Error::request_cancelled(), codes::REQUEST_CANCELLED),
        ] {
            assert_eq!(error.into_response_error().code, code);
        }
    }
}
