#![allow(unsafe_code)] // the one purpose of this module

//! `pidfd_open` and `pidfd_send_signal` support.
//!
//! `pidfd_open` turns a pid into a pollable file descriptor that
//! becomes readable once the process has terminated (Linux 5.3+). This
//! is what makes an async, epoll-driven wait possible instead of the
//! blocking `poll(2)` tick rustils' own portable `wait_any` uses. Async
//! process creation also retains a separate pidfd for identity-safe signaling;
//! there is no numeric-PID fallback after that child is exposed.
//!
//! Mirrors rustils' own `platform-linux::sys::spawn::pidfd_open` exactly
//! (same raw syscall, same `ENOSYS` → `Unsupported` mapping for
//! pre-5.3 kernels) rather than depending on that private function —
//! `platform-linux` does not currently export it.

use std::os::fd::{AsRawFd, BorrowedFd, FromRawFd, OwnedFd};

use platform::error::{ErrorKind, OsCode, PlatformError, Result};
use platform::process::Signal;

/// Open a pidfd for `pid`. `Err(Unsupported)` on a pre-5.3 kernel
/// (`ENOSYS`). Other errors retain their errno and are not presented as a
/// kernel-version failure.
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

/// Signal the process identified by `pidfd`.
///
/// `ESRCH` means the process associated with this stable descriptor has
/// already terminated and is therefore success for the child API. No exit
/// status is invented: the normal wait path remains responsible for reaping.
pub(crate) fn send_signal(pidfd: BorrowedFd<'_>, signal: Signal) -> Result<()> {
    let signum = match signal {
        Signal::Term => libc::SIGTERM,
        Signal::Int => libc::SIGINT,
        Signal::Hup => libc::SIGHUP,
        Signal::Quit => libc::SIGQUIT,
        Signal::Kill => libc::SIGKILL,
        Signal::Stop => libc::SIGSTOP,
        Signal::Cont => libc::SIGCONT,
    };
    // SAFETY: `pidfd` remains borrowed for the entire syscall, `info` is
    // intentionally null as specified by pidfd_send_signal(2), and flags is
    // zero. The descriptor cannot be closed or reused while this borrow exists.
    let result = unsafe {
        libc::syscall(
            libc::SYS_pidfd_send_signal,
            pidfd.as_raw_fd(),
            signum,
            std::ptr::null::<libc::siginfo_t>(),
            0u32,
        )
    };
    if result == 0 {
        return Ok(());
    }
    let code = std::io::Error::last_os_error().raw_os_error().unwrap_or(0);
    if code == libc::ESRCH {
        return Ok(());
    }
    Err(PlatformError::new(
        ErrorKind::Other,
        OsCode::Errno(code),
        "pidfd_send_signal",
    ))
}

#[cfg(test)]
pub(crate) fn descriptor_flags(raw_fd: std::os::fd::RawFd) -> i32 {
    // SAFETY: F_GETFD only reads descriptor-table metadata and has no pointer
    // argument. Tests deliberately also pass a recently closed descriptor.
    unsafe { libc::fcntl(raw_fd, libc::F_GETFD) }
}

#[cfg(test)]
pub(crate) fn close_descriptor(raw_fd: std::os::fd::RawFd) {
    // SAFETY: tests pass a descriptor they intentionally relinquish.
    unsafe { libc::close(raw_fd) };
}

#[cfg(test)]
pub(crate) fn replace_descriptor(source: BorrowedFd<'_>, target: std::os::fd::RawFd) -> i32 {
    // SAFETY: `source` is valid for the call and dup2 atomically replaces the
    // target descriptor. The returned descriptor remains owned by the caller.
    unsafe { libc::dup2(source.as_raw_fd(), target) }
}
