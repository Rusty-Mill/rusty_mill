//! Error types and Result alias for rusty_std.

use alloc::string::String;
use core::fmt;

/// Standard Result type for rusty_std operations.
pub type Result<T> = core::result::Result<T, Error>;

/// Represents errors that occur across the sovereign Rusty Mill stack.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// Input/Output failure with numeric OS code and descriptive message.
    Io(i32, String),
    /// Invalid argument supplied to function.
    InvalidArgument(String),
    /// Requested resource was not found.
    NotFound(String),
    /// Operation timed out.
    TimedOut,
    /// Permission denied by underlying substrate.
    PermissionDenied,
    /// Generic operational failure.
    Custom(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Io(code, msg) => write!(f, "IO Error (code {}): {}", code, msg),
            Error::InvalidArgument(msg) => write!(f, "Invalid Argument: {}", msg),
            Error::NotFound(msg) => write!(f, "Not Found: {}", msg),
            Error::TimedOut => write!(f, "Operation Timed Out"),
            Error::PermissionDenied => write!(f, "Permission Denied"),
            Error::Custom(msg) => write!(f, "Error: {}", msg),
        }
    }
}

impl core::error::Error for Error {}
