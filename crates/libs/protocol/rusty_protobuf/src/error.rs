use std::fmt;

use crate::WireType;

/// Why a buffer could not be read or built.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    /// The buffer ends inside a tag, a value or a length-delimited run.
    Truncated,
    /// A varint is longer than 10 bytes or does not fit 64 bits.
    VarintOverflow,
    /// A field number is 0 or above [`MAX_FIELD_NUMBER`](crate::MAX_FIELD_NUMBER).
    BadTag,
    /// A wire type other than 0, 1, 2, 3, 4 or 5.
    BadWireType(u8),
    /// A group (wire type 3 or 4), which this crate does not support.
    Group,
    /// A field was read as a type its wire type cannot be.
    WrongWireType {
        /// What the accessor needed.
        expected: WireType,
        /// What the buffer holds.
        found: WireType,
    },
    /// A string is not valid UTF-8.
    InvalidUtf8,
    /// Messages are nested deeper than the reader's limit.
    DepthExceeded,
    /// A packed run's length is not a whole number of elements.
    BadPackedLength,
    /// The written message is larger than the format allows (2 GiB), or a field number was
    /// invalid.
    TooLarge,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Truncated => f.write_str("buffer ends inside a field"),
            Error::VarintOverflow => f.write_str("varint longer than 10 bytes or over 64 bits"),
            Error::BadTag => f.write_str("field number is 0 or too large"),
            Error::BadWireType(w) => write!(f, "invalid wire type {w}"),
            Error::Group => f.write_str("groups are not supported"),
            Error::WrongWireType { expected, found } => {
                write!(f, "field is {found:?}, expected {expected:?}")
            }
            Error::InvalidUtf8 => f.write_str("string is not valid UTF-8"),
            Error::DepthExceeded => f.write_str("messages nested too deeply"),
            Error::BadPackedLength => f.write_str("packed run is not a whole number of elements"),
            Error::TooLarge => f.write_str("message too large, or invalid field number"),
        }
    }
}

impl std::error::Error for Error {}
