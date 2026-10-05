use alloc::string::String;
use core::fmt;

/// Why a pointer, a patch document, or an application failed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    /// The pointer text is not RFC 6901: non-empty without a leading `/`,
    /// or a `~` not followed by `0` or `1`.
    InvalidPointer(String),
    /// The patch document is not RFC 6902: not an array, an operation
    /// without `op`/`path`, an unknown `op`, or a missing `value`/`from`.
    InvalidPatch(String),
    /// A path (or `from`) does not resolve in the document.
    PathNotFound(String),
    /// An array step is not a valid index: non-numeric, leading zero, or
    /// out of bounds (`-` is only valid for `add`).
    InvalidIndex(String),
    /// A `test` operation's value differs from the document's.
    TestFailed(String),
    /// A `move` whose `from` is a proper prefix of its `path`.
    MoveIntoSelf(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::InvalidPointer(p) => write!(f, "invalid JSON pointer: {p:?}"),
            Error::InvalidPatch(why) => write!(f, "invalid JSON patch: {why}"),
            Error::PathNotFound(p) => write!(f, "path not found: {p}"),
            Error::InvalidIndex(p) => write!(f, "invalid array index at {p}"),
            Error::TestFailed(p) => write!(f, "test failed at {p}"),
            Error::MoveIntoSelf(p) => write!(f, "cannot move a value into itself: {p}"),
        }
    }
}

#[cfg(feature = "std")]
impl std::error::Error for Error {}
