//! Deferred console-control delivery — the D6 core on Windows. There are
//! no signals here; `SetConsoleCtrlHandler`'s callback (invoked on its
//! own thread by the console host) does exactly one atomic store, the
//! same discipline as the Linux handler (divergence 003: console control
//! events are the deliverable identities, and only console processes
//! receive them).
//!
//! Track W (D-15): `rusty_win32::console::install_ctrl_handler` — this
//! module's whole reason to exist is the donor's *Phase 1*, the binding
//! it was created for, so this is the family that should have been
//! migrated first rather than last.

#![allow(unsafe_code)]

#[cfg(not(feature = "track-w"))]
use std::ffi::OsStr;
use std::sync::atomic::{AtomicU32, Ordering};

use crate::ffi::win32_surface as w;
use crate::sys::errmap;
use platform::error::Result;

/// Sentinel for "empty" — the real events are 0/1/2, so the slot stores
/// `event + 1`.
const NONE: u32 = 0;

static PENDING: AtomicU32 = AtomicU32::new(NONE);

/// The entire handler: store and claim handled (returning 0 would let
/// the default handler terminate the process before the safe-point
/// consumer ever runs).
///
/// A *safe* `extern "system" fn`, not `unsafe extern "system" fn` — its
/// body touches nothing but an atomic, so it never needed the `unsafe`
/// marker on its own account, and being safe is what lets one fn item
/// serve both arms below. windows-sys' `PHANDLER_ROUTINE` wants
/// `Option<unsafe extern "system" fn(u32) -> BOOL>`; the donor's
/// `HandlerRoutine` wants a safe `extern "system" fn(u32) -> i32`. A
/// safe function item coerces to either — safe-to-unsafe fn-pointer
/// coercion is ordinary, sound Rust — so `record` itself needn't (and
/// under `track-w` couldn't) be declared unsafe.
extern "system" fn record(ctrl_type: u32) -> i32 {
    match ctrl_type {
        w::CTRL_C_EVENT | w::CTRL_BREAK_EVENT | w::CTRL_CLOSE_EVENT => {
            PENDING.store(ctrl_type + 1, Ordering::SeqCst);
            1
        }
        _ => 0,
    }
}

/// Install the deferral handler (idempotent — the same routine added
/// twice is still one registration as far as our slot is concerned).
///
/// Track W (D-15): `rusty_win32::console::install_ctrl_handler`.
#[cfg(feature = "track-w")]
pub fn install() -> Result<()> {
    rusty_win32::console::install_ctrl_handler(record)
        .map_err(|e| errmap::trackw_err("SetConsoleCtrlHandler", e))
}

#[cfg(not(feature = "track-w"))]
pub fn install() -> Result<()> {
    // SAFETY: `record` matches PHANDLER_ROUTINE's contract (safe-fn to
    // unsafe-fn-pointer coercion — see `record`'s own doc comment) and
    // touches only an atomic from its console-host thread.
    let ok = unsafe { w::SetConsoleCtrlHandler(Some(record), 1) };
    if ok == 0 {
        return Err(errmap::last_win32_err(
            "SetConsoleCtrlHandler",
            OsStr::new(""),
        ));
    }
    Ok(())
}

/// Consume the pending control event, if any.
pub fn take() -> Option<u32> {
    match PENDING.swap(NONE, Ordering::SeqCst) {
        NONE => None,
        stored => Some(stored - 1),
    }
}
