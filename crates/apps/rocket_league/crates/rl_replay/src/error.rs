//! Parse errors. Each one names what was being read and the byte offset into
//! the file, so an unknown or corrupt replay says exactly where it stopped.

use std::fmt;

/// Why a replay could not be parsed. `offset` is in bytes from the start of
/// the file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// The data ended before `what` could be read.
    Truncated {
        /// What was being read.
        what: &'static str,
        /// Where the read started.
        offset: usize,
        /// Bytes needed.
        needed: usize,
        /// Bytes left.
        available: usize,
    },
    /// A section's stored CRC does not match its data: a corrupt or edited file.
    Crc {
        /// `"header"` or `"content"`.
        section: &'static str,
        /// The CRC stored in the file.
        stored: u32,
        /// The CRC of the data as read.
        actual: u32,
    },
    /// A length or count field is negative or implausibly large.
    BadLength {
        /// What the length was for.
        what: &'static str,
        /// Where the length field starts.
        offset: usize,
        /// The value read.
        value: i64,
    },
    /// A UTF-8 name is not valid UTF-8.
    InvalidUtf8 {
        /// Where the string's bytes start.
        offset: usize,
    },
    /// A header property kind this parser does not know.
    UnknownProperty {
        /// Where the property's kind string starts.
        offset: usize,
        /// The kind as written.
        kind: String,
    },
    /// A nested array or struct would exceed [`crate::MAX_PROPERTY_DEPTH`].
    NestingLimit {
        /// Absolute offset of the rejected property's kind string length.
        offset: usize,
        /// Maximum number of array/struct levels allowed on one path.
        limit: usize,
    },
    /// The replay predates the versions this parser supports
    /// ([`crate::MIN_VERSION`]).
    UnsupportedVersion {
        /// Engine version.
        engine: i32,
        /// Licensee version.
        licensee: i32,
        /// Net version, absent in older replays.
        net: Option<i32>,
    },
    /// Bytes are left over after the header's property map.
    TrailingHeaderData {
        /// Bytes left.
        bytes: usize,
    },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Truncated {
                what,
                offset,
                needed,
                available,
            } => write!(
                f,
                "truncated at byte {offset}: {what} needs {needed} bytes, {available} left"
            ),
            Error::Crc {
                section,
                stored,
                actual,
            } => write!(
                f,
                "{section} CRC mismatch: stored {stored:#010x}, computed {actual:#010x}"
            ),
            Error::BadLength {
                what,
                offset,
                value,
            } => write!(f, "bad {what} {value} at byte {offset}"),
            Error::InvalidUtf8 { offset } => write!(f, "invalid UTF-8 string at byte {offset}"),
            Error::UnknownProperty { offset, kind } => {
                write!(f, "unknown header property kind {kind:?} at byte {offset}")
            }
            Error::NestingLimit { offset, limit } => {
                write!(
                    f,
                    "header property nesting exceeds {limit} levels at byte {offset}"
                )
            }
            Error::UnsupportedVersion {
                engine,
                licensee,
                net,
            } => write!(
                f,
                "unsupported replay version {engine}.{licensee}.{}: {}.{}.{} or later is required",
                net.map_or_else(|| "-".to_string(), |n| n.to_string()),
                crate::MIN_VERSION.0,
                crate::MIN_VERSION.1,
                crate::MIN_VERSION.2
            ),
            Error::TrailingHeaderData { bytes } => {
                write!(f, "{bytes} bytes left after the header properties")
            }
        }
    }
}

impl std::error::Error for Error {}
