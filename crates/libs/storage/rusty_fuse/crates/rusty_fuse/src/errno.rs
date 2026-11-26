//! The port's own error enum. Each adapter maps it to its OS (errno on Linux
//! and macOS, HRESULT on Windows); backends map their own errors into it.

use std::fmt;

/// Why a port call failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Errno {
    /// No such file or directory.
    NoEnt,
    /// A path step is not a directory.
    NotDir,
    /// The node is a directory where a file was needed.
    IsDir,
    /// The caller may not access the node.
    Access,
    /// The operation is not permitted.
    Perm,
    /// An argument is invalid (for example `readlink` on a file).
    Inval,
    /// A name is longer than [`crate::NAME_MAX`].
    NameTooLong,
    /// The node id is unknown, or a directory changed under a readdir cookie.
    Stale,
    /// The handle is not one this backend returned.
    BadHandle,
    /// The backend is read-only.
    ReadOnly,
    /// The backend does not implement the operation.
    NotSupported,
    /// An I/O error the backend could not classify.
    Io,
}

impl fmt::Display for Errno {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let text = match self {
            Self::NoEnt => "no such file or directory",
            Self::NotDir => "not a directory",
            Self::IsDir => "is a directory",
            Self::Access => "access denied",
            Self::Perm => "operation not permitted",
            Self::Inval => "invalid argument",
            Self::NameTooLong => "name too long",
            Self::Stale => "stale node or directory cookie",
            Self::BadHandle => "bad file handle",
            Self::ReadOnly => "read-only filesystem",
            Self::NotSupported => "operation not supported",
            Self::Io => "input/output error",
        };
        f.write_str(text)
    }
}

impl std::error::Error for Errno {}
