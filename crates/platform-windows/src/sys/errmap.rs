//! Win32 / NTSTATUS → `PlatformError` mapping (RFC v2 §5.5).
//!
//! Two native number spaces feed this backend: `GetLastError` codes from
//! Win32 calls and `NTSTATUS` from `NtCreateFile`. Classification into the
//! portable [`ErrorKind`] happens on the code actually returned; an
//! NTSTATUS is additionally converted with `RtlNtStatusToDosError` so the
//! recorded [`OsCode::Win32`] is always in the one documented Win32 space
//! rather than leaking raw NTSTATUS values into diagnostics.

#![allow(unsafe_code)]

use std::ffi::OsStr;

use platform::error::{ErrorKind, OsCode, PlatformError};

use crate::ffi::win32_surface as w;

pub(crate) fn kind_of_win32(code: u32) -> ErrorKind {
    match code {
        w::ERROR_FILE_NOT_FOUND | w::ERROR_PATH_NOT_FOUND | w::ERROR_NOT_FOUND => {
            ErrorKind::NotFound
        }
        w::ERROR_ACCESS_DENIED | w::ERROR_SHARING_VIOLATION => ErrorKind::PermissionDenied,
        w::ERROR_FILE_EXISTS | w::ERROR_ALREADY_EXISTS => ErrorKind::AlreadyExists,
        w::ERROR_DIR_NOT_EMPTY => ErrorKind::DirectoryNotEmpty,
        w::ERROR_DIRECTORY => ErrorKind::NotADirectory,
        // `ERROR_NOT_A_REPARSE_POINT`: `read_link` on an entry that
        // isn't a symlink — mirrors Linux `readlinkat` refusing with
        // `EINVAL`/`InvalidInput` for the same case.
        w::ERROR_INVALID_PARAMETER | w::ERROR_NOT_A_REPARSE_POINT => ErrorKind::InvalidInput,
        _ => ErrorKind::Other,
    }
}

fn kind_of_ntstatus(status: w::NTSTATUS) -> ErrorKind {
    match status {
        w::STATUS_OBJECT_NAME_NOT_FOUND | w::STATUS_OBJECT_PATH_NOT_FOUND => ErrorKind::NotFound,
        w::STATUS_ACCESS_DENIED | w::STATUS_SHARING_VIOLATION | w::STATUS_DELETE_PENDING => {
            ErrorKind::PermissionDenied
        }
        w::STATUS_OBJECT_NAME_COLLISION => ErrorKind::AlreadyExists,
        w::STATUS_FILE_IS_A_DIRECTORY => ErrorKind::IsADirectory,
        w::STATUS_NOT_A_DIRECTORY => ErrorKind::NotADirectory,
        w::STATUS_DIRECTORY_NOT_EMPTY => ErrorKind::DirectoryNotEmpty,
        w::STATUS_OBJECT_NAME_INVALID => ErrorKind::InvalidInput,
        _ => ErrorKind::Other,
    }
}

/// Error from the calling thread's last Win32 error code.
pub fn last_win32_err(op: &'static str, path: &OsStr) -> PlatformError {
    // SAFETY: `GetLastError` takes no arguments and has no preconditions.
    let code = unsafe { w::GetLastError() };
    let e = PlatformError::new(kind_of_win32(code), OsCode::Win32(code), op);
    if path.is_empty() {
        e
    } else {
        e.with_path(path)
    }
}

/// Track W error path (D-15): a `rusty_win32` wrapper reports failure by
/// returning a `Win32Error` it captured itself, immediately after the
/// failing call. Classification is identical to [`last_win32_err`] — same
/// `GetLastError` number space, same [`kind_of_win32`] table — but the code
/// must flow from the returned value rather than be re-read here. Re-reading
/// would be a live race: the thread's last-error slot is overwritten by any
/// intervening Win32 call, including ones the wrapper itself made on the way
/// back out.
///
/// (The Linux side learned the same lesson from the opposite direction — see
/// `docs/learning/002-…`, where Track P's raw syscalls return the errno in
/// the return register and never touch the thread-local at all. Here the
/// thread-local *is* still written; it just stops being the authority.)
#[cfg(feature = "track-w")]
pub fn trackw_err(op: &'static str, e: rusty_win32::error::Win32Error) -> PlatformError {
    let code = e.code();
    PlatformError::new(kind_of_win32(code), OsCode::Win32(code), op)
}

/// Error from a failed ConPTY HRESULT (`CreatePseudoConsole`/
/// `ResizePseudoConsole`, rustils#83). An `HRESULT` isn't `GetLastError`'s
/// or `NtCreateFile`'s number space, but a Win32-facility HRESULT
/// (`HRESULT_FROM_WIN32`'s shape — the common case for a wrapped OS
/// failure) carries the original Win32 code in its low 16 bits: extracted
/// here so `kind_of_win32`'s classification still applies when it can,
/// rather than every ConPTY failure falling back to `ErrorKind::Other`.
/// A non-Win32-facility `HRESULT` (rare for this call — an allocation
/// failure, say) stores its own bit pattern verbatim; still diagnosable,
/// just not classified beyond `Other`.
pub fn hresult_err(hr: i32, op: &'static str) -> PlatformError {
    const FACILITY_WIN32: i32 = 0x7;
    let facility = (hr >> 16) & 0x1FFF;
    if hr < 0 && facility == FACILITY_WIN32 {
        let code = (hr & 0xFFFF) as u32;
        return PlatformError::new(kind_of_win32(code), OsCode::Win32(code), op);
    }
    PlatformError::new(ErrorKind::Other, OsCode::Win32(hr as u32), op)
}

/// Error from a failed `NtCreateFile`-family NTSTATUS.
pub fn nt_err(status: w::NTSTATUS, op: &'static str, path: &OsStr) -> PlatformError {
    // SAFETY: `RtlNtStatusToDosError` is a pure translation function with
    // no preconditions on its argument.
    let code = unsafe { w::RtlNtStatusToDosError(status) };
    let e = PlatformError::new(kind_of_ntstatus(status), OsCode::Win32(code), op);
    if path.is_empty() {
        e
    } else {
        e.with_path(path)
    }
}
