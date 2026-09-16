//! Error types for the RDP codec.

use std::fmt;

/// Result alias used throughout the crate.
pub type Result<T> = std::result::Result<T, Error>;

/// Errors produced while encoding or decoding RDP wire structures.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// The buffer ended before a full field could be read.
    ///
    /// Carries how many bytes were needed and how many remained.
    UnexpectedEof {
        /// Bytes the read operation required.
        needed: usize,
        /// Bytes actually available in the buffer.
        available: usize,
    },
    /// A field held a value outside the range the protocol permits.
    InvalidValue {
        /// Human-readable name of the field.
        field: &'static str,
        /// The offending value, formatted for display.
        value: String,
    },
    /// A structure declared a length that is inconsistent with the protocol
    /// (for example a TPKT length shorter than its own header).
    InvalidLength {
        /// Name of the length field.
        field: &'static str,
        /// The declared length.
        length: usize,
    },
    /// A writer was asked to emit more bytes than the wire format allows in
    /// the relevant length field.
    Overflow {
        /// Name of the field that overflowed.
        field: &'static str,
    },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::UnexpectedEof { needed, available } => write!(
                f,
                "unexpected end of buffer: needed {needed} byte(s), {available} available"
            ),
            Error::InvalidValue { field, value } => {
                write!(f, "invalid value for {field}: {value}")
            }
            Error::InvalidLength { field, length } => {
                write!(f, "invalid length for {field}: {length}")
            }
            Error::Overflow { field } => write!(f, "value too large for {field}"),
        }
    }
}

impl std::error::Error for Error {}

/// Converts a `rusty_wire` cursor error into this crate's error type,
/// preserving the variant shape (`UnexpectedEof`/`InvalidValue` line up
/// field-for-field) so every `?` call site on [`crate::cursor::Reader`] /
/// [`crate::cursor::Writer`] keeps working unchanged.
impl From<rusty_wire::Error> for Error {
    fn from(e: rusty_wire::Error) -> Self {
        match e {
            rusty_wire::Error::UnexpectedEof { needed, available } => {
                Error::UnexpectedEof { needed, available }
            }
            rusty_wire::Error::InvalidValue { field, value } => {
                Error::InvalidValue { field, value }
            }
        }
    }
}
