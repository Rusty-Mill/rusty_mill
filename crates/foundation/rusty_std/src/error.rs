//! Error types and Result alias for rusty_std.

use alloc::string::String;
use core::fmt;

/// Standard Result type for rusty_std operations.
pub type Result<T> = core::result::Result<T, Error>;

/// Represents errors that occur across the sovereign Rusty Mill stack.
///
/// Non-exhaustive: match with a wildcard arm, since more kinds may be added.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
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
    /// The operation has no implementation on this target. Carries the
    /// operation's name, e.g. `"fs::File::open"`.
    Unsupported(&'static str),
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
            Error::Unsupported(op) => write!(f, "Unsupported on this target: {}", op),
        }
    }
}

impl core::error::Error for Error {}
