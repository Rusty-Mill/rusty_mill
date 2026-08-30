//! Deferred signal delivery — the D6 core, verbatim in spirit from
//! rush's `trap.rs`: the installed handler is a single atomic store,
//! nothing more. No allocation, no locks, no I/O — the entire
//! async-signal-safe budget is one `AtomicI32::store`.

#![allow(unsafe_code)]

use std::ffi::OsStr;
use std::sync::atomic::{AtomicI32, Ordering};

use platform::error::{ErrorKind, OsCode, PlatformError, Result};

use crate::ffi::libc_surface as c;

/// The single slot. Signal dispositions are process-global on every OS,
/// so this static is the honest shape, not a shortcut.
static PENDING: AtomicI32 = AtomicI32::new(0);

/// The entire handler (async-signal-safe by construction: one atomic
/// store, sequentially consistent to pair with [`take`]'s swap).
extern "C" fn record(signum: c::c_int) {
    PENDING.store(signum, Ordering::SeqCst);
}

/// Install `record` for `signum`.
#[cfg(not(feature = "track-p"))]
pub fn install(signum: c::c_int) -> Result<()> {
    let handler: extern "C" fn(c::c_int) = record;
    // SAFETY: `handler` is an async-signal-safe extern "C" routine (one
    // atomic store); `signal` accepts a handler function pointer for a
    // valid signal number. glibc's signal() gives BSD semantics
    // (SA_RESTART), matching the donor.
    let prev = unsafe { c::signal(signum, handler as c::sighandler_t) };
    if prev == c::SIG_ERR {
        let code = std::io::Error::last_os_error().raw_os_error().unwrap_or(0);
        return Err(
            PlatformError::new(ErrorKind::InvalidInput, OsCode::Errno(code), "signal")
                .with_path(OsStr::new("")),
        );
    }
    Ok(())
}

/// Install [`record`] for `signum` — Track P: raw `rt_sigaction`.
///
/// The kernel has no `signal` syscall worth using; rusty_libc's `signal`
/// is `rt_sigaction` with `SA_RESTART` (glibc BSD semantics, matching the
/// libc arm) — and on x86_64 it installs its own hand-written
/// `SA_RESTORER` signal-return trampoline, the D4 landmine that makes a
/// raw-syscall signal layer hard (wrong = crash on first delivery). That
/// asm lives in the dependency; parity's delivered-signal test crosses it.
#[cfg(feature = "track-p")]
pub fn install(signum: c::c_int) -> Result<()> {
    let handler: extern "C" fn(c::c_int) = record;
    // SAFETY: `handler` is an async-signal-safe extern "C" routine (one
    // atomic store) with static lifetime, satisfying rusty_libc's
    // handler contract.
    let r = unsafe { rusty_libc::signal::signal(signum, handler as usize) };
    r.map(|_prev| ()).map_err(|e| {
        PlatformError::new(ErrorKind::InvalidInput, OsCode::Errno(e.0), "signal")
            .with_path(OsStr::new(""))
    })
}

/// Consume the pending signal number, if any (atomic swap with 0).
pub fn take() -> Option<c::c_int> {
    match PENDING.swap(0, Ordering::SeqCst) {
        0 => None,
        signum => Some(signum),
    }
}
