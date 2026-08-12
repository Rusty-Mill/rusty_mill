#![allow(unsafe_code)] // the one purpose of this module

//! `pidfd_open` — turns a pid into a pollable file descriptor that
//! becomes readable once the process has terminated (Linux 5.3+). This
//! is what makes an async, epoll-driven wait possible instead of the
//! blocking `poll(2)` tick rustils' own portable `wait_any` uses.
//!
//! Mirrors rustils' own `platform-linux::sys::spawn::pidfd_open` exactly
//! (same raw syscall, same `ENOSYS` → `Unsupported` mapping for
//! pre-5.3 kernels) rather than depending on that private function —
//! `platform-linux` does not currently export it.

use std::os::fd::{FromRawFd, OwnedFd};

use platform::error::{ErrorKind, OsCode, PlatformError, Result};

/// Open a pidfd for `pid`. `Err(Unsupported)` on a pre-5.3 kernel
/// (`ENOSYS`) — the caller falls back to a portable wait, same contract
/// as rustils' own `poll_pids`.
pub fn open(pid: libc::pid_t) -> Result<OwnedFd> {
    // SAFETY: `pidfd_open(pid, flags)` takes two integer arguments and
    // returns an owned fd or -1; no pointer arguments, nothing to
    // uphold beyond checking the return value.
    let fd = unsafe { libc::syscall(libc::SYS_pidfd_open, pid, 0u32) };
    if fd < 0 {
        let code = std::io::Error::last_os_error().raw_os_error().unwrap_or(0);
        if code == libc::ENOSYS {
            return Err(PlatformError::new(
                ErrorKind::Unsupported,
                OsCode::Errno(code),
                "pidfd_open",
            ));
        }
        return Err(PlatformError::new(
            ErrorKind::Other,
            OsCode::Errno(code),
            "pidfd_open",
        ));
    }
    // SAFETY: `fd` is a freshly returned, valid, otherwise-unowned
    // descriptor from the syscall above; wrapped exactly once here.
    Ok(unsafe { OwnedFd::from_raw_fd(fd as std::os::fd::RawFd) })
}
