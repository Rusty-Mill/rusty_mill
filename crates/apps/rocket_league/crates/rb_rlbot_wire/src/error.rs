use std::fmt;

/// Why a message could not be encoded or decoded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    /// The FlatBuffers layer rejected the bytes (or the value did not fit the format).
    Flatbuffers(rusty_flatbuffers::Error),
    /// An enum field holds a value this version of the protocol does not define.
    UnknownEnum { name: &'static str, value: u8 },
    /// A union holds a member this crate does not model, where one is required.
    UnknownUnion { name: &'static str, tag: u8 },
    /// A field the protocol requires is absent.
    Missing(&'static str),
    /// A payload is longer than a `u16` length prefix can describe.
    FrameTooLarge(usize),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Flatbuffers(e) => write!(f, "flatbuffers: {e}"),
            Error::UnknownEnum { name, value } => write!(f, "unknown {name} value {value}"),
            Error::UnknownUnion { name, tag } => write!(f, "unsupported {name} member {tag}"),
            Error::Missing(field) => write!(f, "missing required field `{field}`"),
            Error::FrameTooLarge(len) => write!(f, "payload of {len} bytes exceeds a u16 frame"),
        }
    }
}

impl std::error::Error for Error {}

impl From<rusty_flatbuffers::Error> for Error {
    fn from(e: rusty_flatbuffers::Error) -> Error {
        Error::Flatbuffers(e)
    }
}
