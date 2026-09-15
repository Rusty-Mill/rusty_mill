//! `read(2)`/`write(2)` on an already-open fd — the byte-transfer half
//! of a connected socket, identical to `platform-linux::sys::fdio`'s
//! pair (kept as its own tiny module rather than folded into
//! `sys::net`, matching that crate's split, since a future `fs`/
//! `process` slice would want to reuse it the same way `fs.rs` reuses
//! `fdio::read`/`write` there — not adding it speculatively, just not
//! naming it as `net`-only when it isn't).

#![allow(unsafe_code)]

use std::os::fd::{AsRawFd, OwnedFd};

use platform::error::{ErrorKind, OsCode, PlatformError, Result};

use crate::ffi::libc_surface as c;

fn os_err(op: &'static str) -> PlatformError {
    let code = std::io::Error::last_os_error().raw_os_error().unwrap_or(0);
    let kind = match code {
        libc::EAGAIN => ErrorKind::WouldBlock,
        libc::EINTR => ErrorKind::Interrupted,
        libc::EPIPE => ErrorKind::BrokenPipe,
        _ => ErrorKind::Other,
    };
    PlatformError::new(kind, OsCode::Errno(code), op)
}

/// `read(2)` into `buf`.
pub fn read(fd: &OwnedFd, buf: &mut [u8]) -> Result<usize> {
    // SAFETY: `buf` is a valid, writable region of exactly `buf.len()`
    // bytes for the duration of the call; `fd` is a valid open
    // descriptor.
    let n = unsafe { c::read(fd.as_raw_fd(), buf.as_mut_ptr().cast(), buf.len()) };
    if n < 0 {
        return Err(os_err("read"));
    }
    Ok(n as usize)
}

/// `write(2)` from `buf`.
pub fn write(fd: &OwnedFd, buf: &[u8]) -> Result<usize> {
    // SAFETY: `buf` is a valid readable region of exactly `buf.len()`
    // bytes for the duration of the call; `fd` is a valid open
    // descriptor.
    let n = unsafe { c::write(fd.as_raw_fd(), buf.as_ptr().cast(), buf.len()) };
    if n < 0 {
        return Err(os_err("write"));
    }
    Ok(n as usize)
}
