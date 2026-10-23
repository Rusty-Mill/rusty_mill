//! Why bytes from a peer were rejected.

use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// The length prefix exceeds [`MAX_FRAME`](crate::MAX_FRAME).
    FrameTooLarge(u32),
    /// The length prefix is zero, so there is no message type.
    EmptyFrame,
    UnknownType(u8),
    /// A field or varint runs past the end of its frame.
    Truncated,
    /// A varint is longer than ten bytes or overflows `u64`.
    VarintOverflow,
    /// A field key's id does not fit in `u32`.
    FieldIdTooLarge(u64),
    MissingField {
        message: u8,
        field: u32,
    },
    /// A field holds the wrong kind, or a value out of range.
    InvalidField {
        message: u8,
        field: u32,
    },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::FrameTooLarge(len) => write!(f, "frame of {len} bytes exceeds the limit"),
            Self::EmptyFrame => f.write_str("empty frame"),
            Self::UnknownType(ty) => write!(f, "unknown message type {ty:#04x}"),
            Self::Truncated => f.write_str("field runs past the end of its frame"),
            Self::VarintOverflow => f.write_str("varint overflows u64"),
            Self::FieldIdTooLarge(key) => write!(f, "field key {key} has an id over u32"),
            Self::MissingField { message, field } => {
                write!(f, "message {message:#04x} is missing field {field}")
            }
            Self::InvalidField { message, field } => {
                write!(f, "message {message:#04x} has an invalid field {field}")
            }
        }
    }
}

impl std::error::Error for Error {}

impl From<rusty_wire::Error> for Error {
    fn from(_: rusty_wire::Error) -> Self {
        Self::Truncated
    }
}
