use std::fmt;

/// Why a buffer could not be read or built.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    /// A read of `len` bytes at `pos` falls outside a buffer of `buf_len` bytes.
    Truncated {
        pos: usize,
        len: usize,
        buf_len: usize,
    },
    /// An offset points before the start of the buffer or overflows.
    BadOffset,
    /// A vtable is shorter than its header, odd-sized, or outside the buffer.
    BadVtable,
    /// A string is not valid UTF-8.
    InvalidUtf8,
    /// A vector index is past its length.
    IndexOutOfRange { index: usize, len: usize },
    /// The built data does not fit the format's 16- or 32-bit fields.
    TooLarge,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Truncated { pos, len, buf_len } => {
                write!(
                    f,
                    "read of {len} bytes at {pos} is outside a {buf_len}-byte buffer"
                )
            }
            Error::BadOffset => f.write_str("offset points outside the buffer"),
            Error::BadVtable => f.write_str("malformed vtable"),
            Error::InvalidUtf8 => f.write_str("string is not valid UTF-8"),
            Error::IndexOutOfRange { index, len } => {
                write!(f, "index {index} is past a vector of {len}")
            }
            Error::TooLarge => f.write_str("buffer too large for the format's offsets"),
        }
    }
}

impl std::error::Error for Error {}
