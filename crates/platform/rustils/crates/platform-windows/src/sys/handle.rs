//! Owned HANDLE wrapper — RAII done once, used everywhere.
//!
//! Constructors take ownership of a freshly-returned, valid HANDLE and
//! close it exactly once on drop. No `Copy`, no shared-close: the v1
//! scaffold's close-through-shared-reference bug (B-4) is unrepresentable
//! against this type.

#![allow(unsafe_code)]

use std::ffi::OsStr;
use std::os::windows::io::{AsHandle, BorrowedHandle, FromRawHandle, IntoRawHandle, OwnedHandle};

use platform::error::Result;

use crate::ffi::win32_surface as w;
use crate::sys::errmap;

/// An owned Win32 HANDLE, closed on drop.
#[derive(Debug)]
pub struct OwnedWinHandle(w::HANDLE);

// SAFETY: a Win32 HANDLE is an opaque, thread-affinity-free value — the
// underlying OS object doesn't care which thread issues calls against
// it, and concurrent use of the *same* open handle from multiple
// threads (e.g. one thread's `ReadFile` racing another's, or a
// background watcher thread waiting on a handle another thread also
// holds — `sys::pty::spawn_exit_watcher`'s own reason for needing this)
// is a documented-safe, common Windows pattern. `w::HANDLE` is a raw
// pointer type, which is `!Send`/`!Sync` by default only because the
// compiler can't know that about an arbitrary pointer — this type's own
// single-owner, close-once-on-drop contract (this module's own doc
// comment) isn't weakened by letting that owner live on a different
// thread than the one that created it, or by sharing `&OwnedWinHandle`
// across threads (no interior mutability here for a second thread to
// observe torn).
unsafe impl Send for OwnedWinHandle {}
// SAFETY: same reasoning as the `Send` impl above — `&OwnedWinHandle`
// only ever exposes `as_raw()` (a `Copy` read of the handle value), so
// sharing that immutable view across threads has nothing to race.
unsafe impl Sync for OwnedWinHandle {}

impl OwnedWinHandle {
    /// Take ownership of `handle`.
    ///
    /// # Safety contract (checked by the caller)
    /// `handle` must be a valid handle returned by a Win32 creation call,
    /// not `INVALID_HANDLE_VALUE`, and not owned elsewhere.
    pub fn from_raw(handle: w::HANDLE) -> Option<Self> {
        (handle != w::INVALID_HANDLE_VALUE && !handle.is_null()).then_some(Self(handle))
    }

    pub fn as_raw(&self) -> w::HANDLE {
        self.0
    }

    /// Relinquish ownership: the caller (or the type it hands the raw
    /// value to) becomes responsible for the single close.
    pub fn into_raw(self) -> w::HANDLE {
        let handle = self.0;
        std::mem::forget(self);
        handle
    }
}

// std interop (RFC v2 §5.1): conversions to and from `std::os::windows`
// handle types, so this layer is adoptable incrementally.

impl AsHandle for OwnedWinHandle {
    fn as_handle(&self) -> BorrowedHandle<'_> {
        // SAFETY: `self.0` is a valid open handle for as long as `self`
        // lives, which bounds the returned borrow's lifetime.
        unsafe { BorrowedHandle::borrow_raw(self.0) }
    }
}

impl From<OwnedWinHandle> for OwnedHandle {
    fn from(handle: OwnedWinHandle) -> OwnedHandle {
        // SAFETY: `into_raw` transfers sole ownership of a valid,
        // currently-open handle; the std type now closes it exactly once.
        unsafe { OwnedHandle::from_raw_handle(handle.into_raw()) }
    }
}

impl From<OwnedHandle> for OwnedWinHandle {
    fn from(handle: OwnedHandle) -> OwnedWinHandle {
        // `OwnedHandle` guarantees a valid open handle, satisfying this
        // type's construction contract; `into_raw_handle` transfers sole
        // ownership.
        Self(handle.into_raw_handle())
    }
}

/// `DuplicateHandle` within this process (`File::try_clone`, D5,
/// rustils#51; also the spawn-time `Stdio::File` inheritable-wiring
/// step in `sys::proc`). `DUPLICATE_SAME_ACCESS` copies the source's own
/// access rights rather than requiring the caller to know them; the
/// duplicate shares the same underlying open-file object (position
/// included — that sharing is `DuplicateHandle`'s defining property,
/// same as `dup(2)`'s). `inheritable` controls only whether *this*
/// duplicate's own inherit flag is set — `try_clone`'s callers want
/// `false` (a clone is not automatically handed to a future child just
/// because it was requested), `sys::proc`'s spawn-time wiring wants
/// `true`.
///
/// Track W (D-15): `rusty_win32::handle::duplicate`. Same call, same
/// `DUPLICATE_SAME_ACCESS` / current-process-to-current-process shape,
/// same `inheritable` meaning — the donor's wrapper takes the boolean
/// directly where this arm converts it to the `BOOL` itself.
#[cfg(feature = "track-w")]
pub fn duplicate(handle: &OwnedWinHandle, inheritable: bool) -> Result<OwnedWinHandle> {
    // SAFETY: `handle.as_raw()` is a valid open handle for the life of
    // `handle` — `duplicate`'s whole safety contract, and the reason it
    // takes `&OwnedWinHandle` rather than a raw value.
    let dup = unsafe { rusty_win32::handle::duplicate(handle.as_raw(), inheritable) }
        .map_err(|e| errmap::trackw_err("DuplicateHandle", e))?;
    OwnedWinHandle::from_raw(dup)
        .ok_or_else(|| errmap::last_win32_err("DuplicateHandle", OsStr::new("")))
}

#[cfg(not(feature = "track-w"))]
pub fn duplicate(handle: &OwnedWinHandle, inheritable: bool) -> Result<OwnedWinHandle> {
    let mut dup: w::HANDLE = std::ptr::null_mut();
    // SAFETY: `handle.as_raw()` is a valid open handle for the life of
    // `handle`; `GetCurrentProcess()` is a pseudo-handle valid without
    // acquisition or release; `dup` is a valid out-pointer.
    let ok = unsafe {
        w::DuplicateHandle(
            w::GetCurrentProcess(),
            handle.as_raw(),
            w::GetCurrentProcess(),
            &mut dup,
            0,
            i32::from(inheritable),
            w::DUPLICATE_SAME_ACCESS,
        )
    };
    if ok == 0 {
        return Err(errmap::last_win32_err("DuplicateHandle", OsStr::new("")));
    }
    OwnedWinHandle::from_raw(dup)
        .ok_or_else(|| errmap::last_win32_err("DuplicateHandle", OsStr::new("")))
}

impl Drop for OwnedWinHandle {
    /// Track W (D-15): `rusty_win32::handle::close`. Its `Result` is
    /// discarded for the same reason the windows-sys arm ignores
    /// `CloseHandle`'s `BOOL` — a destructor has nowhere to report to, and
    /// this type's own construction invariant is what makes the close
    /// correct in the first place.
    #[cfg(feature = "track-w")]
    fn drop(&mut self) {
        // SAFETY: `self.0` was validated non-invalid at construction, is
        // owned uniquely by this value, and is closed exactly once here —
        // `close`'s "not used again after this call" obligation is
        // discharged by this being the drop glue.
        let _ = unsafe { rusty_win32::handle::close(self.0) };
    }

    #[cfg(not(feature = "track-w"))]
    fn drop(&mut self) {
        // SAFETY: `self.0` was validated non-invalid at construction, is
        // owned uniquely by this value, and is closed exactly once here.
        unsafe {
            w::CloseHandle(self.0);
        }
    }
}
