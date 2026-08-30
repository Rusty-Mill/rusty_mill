//! Two-axis error model (RFC v2 §5.5, decision D-8).
//!
//! Every error carries (a) a portable [`ErrorKind`] a caller can match on,
//! and (b) the raw OS code in its own number space via [`OsCode`] — never a
//! bare integer that conflates `errno` with `GetLastError`. Operation and
//! path context ride along so an error is diagnosable without a debugger.

use std::path::PathBuf;

/// Portable classification of a platform error.
///
/// Backends map their OS's native codes into this taxonomy; the mapping
/// tables are parity-tested (RFC v2 §9). `Other` is the escape hatch for
/// codes not yet classified — matching on it should prompt extending the
/// taxonomy, not shipping around it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ErrorKind {
    NotFound,
    PermissionDenied,
    AlreadyExists,
    NotADirectory,
    IsADirectory,
    DirectoryNotEmpty,
    InvalidInput,
    WouldBlock,
    Interrupted,
    BrokenPipe,
    Unsupported,
    /// Net surface (RFC v2 R5+, D16): the peer actively refused the
    /// connection (`ECONNREFUSED`/`WSAECONNREFUSED`) — nothing was
    /// listening, or the listen backlog was full.
    ConnectionRefused,
    /// Net surface: the peer reset the connection
    /// (`ECONNRESET`/`WSAECONNRESET`).
    ConnectionReset,
    /// Net surface: the connection was aborted before it fully
    /// established (`ECONNABORTED`/`WSAECONNABORTED`).
    ConnectionAborted,
    /// Net surface: an operation needing a connected socket was
    /// attempted on one that isn't (`ENOTCONN`/`WSAENOTCONN`).
    NotConnected,
    /// Net surface: the requested local address is already in use
    /// (`EADDRINUSE`/`WSAEADDRINUSE`).
    AddrInUse,
    /// Net surface: the requested local address isn't valid on this
    /// host (`EADDRNOTAVAIL`/`WSAEADDRNOTAVAIL`).
    AddrNotAvailable,
    /// Net surface: the operation exceeded its deadline
    /// (`ETIMEDOUT`/`WSAETIMEDOUT`).
    TimedOut,
    /// R2/R3 resolution containment (Rusty-Mill fs slice): path
    /// resolution encountered a symlink/reparse point it was told to
    /// reject (`openat2` `RESOLVE_NO_SYMLINKS` → `ELOOP`; NT
    /// `OBJ_DONT_REPARSE` → `STATUS_REPARSE_POINT_ENCOUNTERED`) —
    /// distinct from the ordinary POSIX "too many symlinks in a normal
    /// resolution" `ELOOP`, which this taxonomy has never separately
    /// distinguished either, so the two share this variant rather than
    /// growing a third.
    FilesystemLoop,
    /// R2 mount-confinement (Rusty-Mill fs slice): path resolution was
    /// told not to cross a filesystem/volume boundary and would have had
    /// to (`openat2` `RESOLVE_NO_XDEV` → `EXDEV`). Also the existing,
    /// unrelated-to-R2, "rename across filesystems" `EXDEV` — both are
    /// "the kernel refused to let this resolution/operation leave one
    /// filesystem," so one variant covers both rather than forcing a
    /// caller to guess which of two near-identical kinds applies.
    CrossesDevices,
    Other,
}

/// The raw OS error in its own number space.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OsCode {
    /// Unix `errno`.
    Errno(i32),
    /// Windows `GetLastError` / NTSTATUS-derived Win32 code.
    Win32(u32),
    /// No OS code applies (e.g. an error synthesized by `platform-mock`).
    None,
}

/// A platform operation failure with full context.
#[derive(Debug, thiserror::Error)]
#[error("{op} failed{}: {kind:?} ({os:?})", path_display(.path))]
pub struct PlatformError {
    pub kind: ErrorKind,
    pub os: OsCode,
    /// The operation that failed, e.g. `"openat"`, `"CreateProcessW"`.
    pub op: &'static str,
    /// The path involved, when one was.
    pub path: Option<PathBuf>,
}

fn path_display(path: &Option<PathBuf>) -> String {
    match path {
        Some(p) => format!(" on {}", p.display()),
        None => String::new(),
    }
}

impl PlatformError {
    /// Construct an error with no path context.
    pub fn new(kind: ErrorKind, os: OsCode, op: &'static str) -> Self {
        Self {
            kind,
            os,
            op,
            path: None,
        }
    }

    /// Attach path context.
    #[must_use]
    pub fn with_path(mut self, path: impl Into<PathBuf>) -> Self {
        self.path = Some(path.into());
        self
    }
}

/// Convenience alias used across the workspace.
pub type Result<T> = std::result::Result<T, PlatformError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_display_carries_context() {
        let e = PlatformError::new(ErrorKind::NotFound, OsCode::Errno(2), "openat")
            .with_path("/tmp/missing");
        let s = e.to_string();
        assert!(s.contains("openat"), "operation missing from: {s}");
        assert!(s.contains("/tmp/missing"), "path missing from: {s}");
        assert!(s.contains("NotFound"), "kind missing from: {s}");
    }

    #[test]
    fn os_code_spaces_do_not_conflate() {
        // The type system is the test: these are different variants, not
        // the same bare u32. This test exists to pin the regression the
        // v1 scaffold shipped (IoError(u32) mixing errno and Win32 spaces).
        assert_ne!(OsCode::Errno(5), OsCode::Win32(5));
    }
}
