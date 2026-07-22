//! The error type shared by both transport adapters ([`crate::sync`],
//! and [`crate::async_tokio`] behind the `rusty-tokio` feature): unlike
//! the sans-IO core's own [`crate::Error`], an adapter genuinely touches
//! a socket, so its errors are either an I/O failure or one from the
//! core it's driving.

use std::fmt;

/// Everything that can go wrong driving the sans-IO core over a real
/// transport.
#[derive(Debug)]
pub enum Error {
    /// The transport itself failed, or closed before a head/body
    /// completed.
    Io(std::io::Error),
    /// The core rejected what came off (or went onto) the wire.
    Http(crate::Error),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Io(e) => write!(f, "io error: {e}"),
            Error::Http(e) => write!(f, "http error: {e}"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Io(e) => Some(e),
            Error::Http(e) => Some(e),
        }
    }
}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::Io(e)
    }
}

impl From<crate::Error> for Error {
    fn from(e: crate::Error) -> Self {
        Error::Http(e)
    }
}

/// A transport adapter's `Result` alias.
pub type Result<T> = std::result::Result<T, Error>;

/// A connection closed before a complete head/body arrived -- distinct
/// from a clean "nothing left to read" EOF at a message boundary, which
/// callers see as a `0`-length final chunk instead.
pub(crate) fn unexpected_eof(context: &str) -> Error {
    Error::Io(std::io::Error::new(
        std::io::ErrorKind::UnexpectedEof,
        context.to_string(),
    ))
}
