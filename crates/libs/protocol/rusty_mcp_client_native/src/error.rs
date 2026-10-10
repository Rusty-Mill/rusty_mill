//! What can go wrong in a client call.

use rusty_mcp_proto::ErrorData;
use std::fmt;
use std::io;

/// A failed client operation.
#[derive(Debug)]
pub enum ClientError {
    /// The transport failed.
    Io(io::Error),
    /// The server closed the connection.
    Closed,
    /// No answer within the call timeout.
    Timeout,
    /// The server answered with a JSON-RPC error.
    Rpc(ErrorData),
    /// The server's answer did not fit the protocol.
    Protocol(String),
    /// No protocol revision both sides speak.
    NoCommonVersion,
    /// A call kept asking for input beyond the round limit.
    TooManyRounds(u32),
    /// A task ended without a result.
    TaskEnded(String),
}

impl fmt::Display for ClientError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(e) => write!(f, "transport error: {e}"),
            Self::Closed => f.write_str("the server closed the connection"),
            Self::Timeout => f.write_str("timed out waiting for the server"),
            Self::Rpc(e) => write!(f, "server error {}: {}", e.code.0, e.message),
            Self::Protocol(why) => write!(f, "protocol error: {why}"),
            Self::NoCommonVersion => f.write_str("no protocol revision in common with the server"),
            Self::TooManyRounds(n) => write!(f, "the server asked for input more than {n} times"),
            Self::TaskEnded(why) => write!(f, "the task ended without a result: {why}"),
        }
    }
}

impl std::error::Error for ClientError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(e) => Some(e),
            _ => None,
        }
    }
}

impl From<io::Error> for ClientError {
    fn from(e: io::Error) -> Self {
        Self::Io(e)
    }
}

impl From<rusty_mcp_proto::Error> for ClientError {
    fn from(e: rusty_mcp_proto::Error) -> Self {
        Self::Protocol(e.to_string())
    }
}
