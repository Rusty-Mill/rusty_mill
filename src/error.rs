use std::fmt;

/// Errors produced by this crate's engine and API layer.
#[derive(Debug)]
pub enum Error {
    /// The connection has already been closed.
    ConnectionClosed,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::ConnectionClosed => write!(f, "connection is already closed"),
        }
    }
}

impl std::error::Error for Error {}

pub type Result<T> = std::result::Result<T, Error>;
