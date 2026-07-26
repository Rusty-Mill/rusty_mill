//! Terminal primitives (extraction map D9): isatty, window size, raw
//! mode with saved-state restore, and (slice 2, roadmap Phase 2) live
//! raw-mode probing, poll/read on stdin, and echo toggling. Both
//! floors: libc (`tcgetattr` / `cfmakeraw` / `ioctl(TIOCGWINSZ)`) and
//! rusty_libc raw syscalls under `track-p` (kernel-shape `Termios`,
//! NCCS=19 — the D4 layout landmine lives in the dependency, not here).

#![allow(unsafe_code)]

use std::time::Duration;

use platform::error::{ErrorKind, OsCode, PlatformError, Result};

use crate::ffi::libc_surface as c;

fn term_err(op: &'static str, code: i32) -> PlatformError {
    let kind = match code {
        // ENOTTY is the "stream is not a terminal" answer — the caller-
        // visible case behavior/term.md specifies.
        libc::ENOTTY => ErrorKind::Other,
        libc::EBADF => ErrorKind::InvalidInput,
        _ => ErrorKind::Other,
    };
    PlatformError::new(kind, OsCode::Errno(code), op)
}

/// The raw fd of a standard stream — private to this module; fd numbers
/// stop at the sys boundary (RFC v2 §5).
pub fn stream_fd(stream: platform::term::TermStream) -> i32 {
    match stream {
        platform::term::TermStream::Stdin => c::STDIN_FILENO,
        platform::term::TermStream::Stdout => c::STDOUT_FILENO,
        platform::term::TermStream::Stderr => c::STDERR_FILENO,
    }
}

/// `isatty(fd)`.
#[cfg(not(feature = "track-p"))]
pub fn is_tty(fd: i32) -> bool {
    // SAFETY: isatty takes a plain integer and has no pointer arguments.
    (unsafe { c::isatty(fd) }) == 1
}

/// `isatty(fd)` — Track P: a `tcgetattr` probe (that is all isatty is).
#[cfg(feature = "track-p")]
pub fn is_tty(fd: i32) -> bool {
    rusty_libc::termios::isatty(fd)
}

/// `ioctl(fd, TIOCGWINSZ)` → (rows, cols).
#[cfg(not(feature = "track-p"))]
pub fn window_size(fd: i32) -> Result<(u16, u16)> {
    // SAFETY: `winsize` is plain-old-data; all-zeroes is a valid value
    // the kernel overwrites on success.
    let mut ws: c::winsize = unsafe { std::mem::zeroed() };
    // SAFETY: valid fd; `ws` is a valid out-pointer of the exact struct
    // TIOCGWINSZ writes, outliving the call.
    let r = unsafe { c::ioctl(fd, c::TIOCGWINSZ, &mut ws) };
    if r != 0 {
        let code = std::io::Error::last_os_error().raw_os_error().unwrap_or(0);
        return Err(term_err("TIOCGWINSZ", code));
    }
    Ok((ws.ws_row, ws.ws_col))
}

/// Window size — Track P: raw `ioctl(TIOCGWINSZ)` via rusty_libc.
#[cfg(feature = "track-p")]
pub fn window_size(fd: i32) -> Result<(u16, u16)> {
    let ws = rusty_libc::tty::window_size(fd).map_err(|e| term_err("TIOCGWINSZ", e.0))?;
    Ok((ws.ws_row, ws.ws_col))
}

/// The saved terminal state [`enter_raw`] returns and [`restore`] takes
/// back — opaque to callers, one per backend arm.
#[cfg(not(feature = "track-p"))]
pub struct SavedTermios(c::termios);
/// Track P saved state: the kernel-shape `Termios`.
#[cfg(feature = "track-p")]
pub struct SavedTermios(rusty_libc::termios::Termios);

/// Read the current attributes, apply the raw-mode recipe
/// (`cfmakeraw`), and return the previous state for [`restore`].
#[cfg(not(feature = "track-p"))]
pub fn enter_raw(fd: i32) -> Result<SavedTermios> {
    // SAFETY: `termios` is plain-old-data; zeroed is valid scratch that
    // tcgetattr overwrites on success.
    let mut cur: c::termios = unsafe { std::mem::zeroed() };
    // SAFETY: valid fd; `cur` is a valid out-pointer outliving the call.
    if unsafe { c::tcgetattr(fd, &mut cur) } != 0 {
        let code = std::io::Error::last_os_error().raw_os_error().unwrap_or(0);
        return Err(term_err("tcgetattr", code));
    }
    let saved = SavedTermios(cur);
    let mut raw = cur;
    // SAFETY: `raw` is a valid termios initialized by tcgetattr above;
    // cfmakeraw only writes its flag fields.
    unsafe { c::cfmakeraw(&mut raw) };
    // SAFETY: valid fd; `raw` is a fully initialized termios outliving
    // the call. TCSADRAIN lets pending output flush before the switch.
    if unsafe { c::tcsetattr(fd, c::TCSADRAIN, &raw) } != 0 {
        let code = std::io::Error::last_os_error().raw_os_error().unwrap_or(0);
        return Err(term_err("tcsetattr", code));
    }
    Ok(saved)
}

/// Raw mode — Track P: rusty_libc's kernel-layout `Termios` and its
/// `make_raw` (the same cfmakeraw recipe, spelled against NCCS=19).
#[cfg(feature = "track-p")]
pub fn enter_raw(fd: i32) -> Result<SavedTermios> {
    use rusty_libc::termios as rt;
    let cur = rt::tcgetattr(fd).map_err(|e| term_err("tcgetattr", e.0))?;
    let saved = SavedTermios(cur);
    let mut raw = cur;
    raw.make_raw();
    rt::tcsetattr_with(fd, rt::TCSADRAIN, &raw).map_err(|e| term_err("tcsetattr", e.0))?;
    Ok(saved)
}

/// Restore the state saved by [`enter_raw`].
#[cfg(not(feature = "track-p"))]
pub fn restore(fd: i32, saved: &SavedTermios) -> Result<()> {
    // SAFETY: valid fd; `saved.0` is the fully initialized termios
    // captured by enter_raw, outliving the call.
    if unsafe { c::tcsetattr(fd, c::TCSADRAIN, &saved.0) } != 0 {
        let code = std::io::Error::last_os_error().raw_os_error().unwrap_or(0);
        return Err(term_err("tcsetattr", code));
    }
    Ok(())
}

/// Restore — Track P.
#[cfg(feature = "track-p")]
pub fn restore(fd: i32, saved: &SavedTermios) -> Result<()> {
    rusty_libc::termios::tcsetattr_with(fd, rusty_libc::termios::TCSADRAIN, &saved.0)
        .map_err(|e| term_err("tcsetattr", e.0))
}

/// Live probe: does `fd`'s *current* attributes look raw (no `ICANON`,
/// no `ECHO`)? Swallows errors as "not raw" — this is a best-effort
/// probe, not a fallible operation; a stream that cannot be queried at
/// all is not usefully "raw".
#[cfg(not(feature = "track-p"))]
pub fn is_raw(fd: i32) -> bool {
    // SAFETY: `termios` is plain-old-data; zeroed is valid scratch that
    // tcgetattr overwrites on success.
    let mut cur: c::termios = unsafe { std::mem::zeroed() };
    // SAFETY: valid fd; `cur` is a valid out-pointer outliving the call.
    if unsafe { c::tcgetattr(fd, &mut cur) } != 0 {
        return false;
    }
    let mask = (c::ICANON | c::ECHO) as libc::tcflag_t;
    cur.c_lflag & mask == 0
}

/// Live raw-mode probe — Track P.
#[cfg(feature = "track-p")]
pub fn is_raw(fd: i32) -> bool {
    match rusty_libc::termios::tcgetattr(fd) {
        Ok(cur) => cur.c_lflag & (rusty_libc::termios::ICANON | rusty_libc::termios::ECHO) == 0,
        Err(_) => false,
    }
}

/// Toggle `ECHO` on `fd` without touching any other flag, returning the
/// previous on/off state.
#[cfg(not(feature = "track-p"))]
pub fn set_echo(fd: i32, on: bool) -> Result<bool> {
    // SAFETY: `termios` is plain-old-data; zeroed is valid scratch that
    // tcgetattr overwrites on success.
    let mut cur: c::termios = unsafe { std::mem::zeroed() };
    // SAFETY: valid fd; `cur` is a valid out-pointer outliving the call.
    if unsafe { c::tcgetattr(fd, &mut cur) } != 0 {
        let code = std::io::Error::last_os_error().raw_os_error().unwrap_or(0);
        return Err(term_err("tcgetattr", code));
    }
    let mask = c::ECHO as libc::tcflag_t;
    let was_on = cur.c_lflag & mask != 0;
    if on {
        cur.c_lflag |= mask;
    } else {
        cur.c_lflag &= !mask;
    }
    // SAFETY: valid fd; `cur` is a fully initialized termios (read back
    // above, one field mutated) outliving the call.
    if unsafe { c::tcsetattr(fd, c::TCSADRAIN, &cur) } != 0 {
        let code = std::io::Error::last_os_error().raw_os_error().unwrap_or(0);
        return Err(term_err("tcsetattr", code));
    }
    Ok(was_on)
}

/// Toggle echo — Track P.
#[cfg(feature = "track-p")]
pub fn set_echo(fd: i32, on: bool) -> Result<bool> {
    use rusty_libc::termios as rt;
    let mut cur = rt::tcgetattr(fd).map_err(|e| term_err("tcgetattr", e.0))?;
    let was_on = cur.c_lflag & rt::ECHO != 0;
    if on {
        cur.c_lflag |= rt::ECHO;
    } else {
        cur.c_lflag &= !rt::ECHO;
    }
    rt::tcsetattr_with(fd, rt::TCSADRAIN, &cur).map_err(|e| term_err("tcsetattr", e.0))?;
    Ok(was_on)
}

/// `poll(fd, POLLIN, timeout_ms)`; `None` timeout blocks forever.
#[cfg(not(feature = "track-p"))]
pub fn poll_readable(fd: i32, timeout: Option<Duration>) -> Result<bool> {
    let timeout_ms: c::c_int = match timeout {
        None => -1,
        Some(d) => d.as_millis().min(i32::MAX as u128) as c::c_int,
    };
    let mut pfd = c::pollfd {
        fd,
        events: c::POLLIN,
        revents: 0,
    };
    // SAFETY: `pfd` is a single valid pollfd entry outliving the call.
    let r = unsafe { c::poll(&mut pfd, 1, timeout_ms) };
    if r < 0 {
        let code = std::io::Error::last_os_error().raw_os_error().unwrap_or(0);
        return Err(term_err("poll", code));
    }
    Ok(r > 0)
}

/// Poll for readability — Track P: raw `SYS_poll`/`SYS_ppoll`.
#[cfg(feature = "track-p")]
pub fn poll_readable(fd: i32, timeout: Option<Duration>) -> Result<bool> {
    let timeout_ms: c::c_int = match timeout {
        None => -1,
        Some(d) => d.as_millis().min(i32::MAX as u128) as c::c_int,
    };
    let mut pfd = rusty_libc::fd::PollFd::new(fd, rusty_libc::fd::POLLIN);
    let n = rusty_libc::fd::poll(std::slice::from_mut(&mut pfd), timeout_ms)
        .map_err(|e| term_err("poll", e.0))?;
    Ok(n > 0)
}

/// `read(fd, buf)` — one call, batched, `Ok(0)` = EOF.
#[cfg(not(feature = "track-p"))]
pub fn read_chunk(fd: i32, buf: &mut [u8]) -> Result<usize> {
    // SAFETY: `buf` is a valid, writable region of exactly `buf.len()`
    // bytes for the duration of the call; `fd` is a valid open
    // descriptor (a standard stream, alive for the process lifetime).
    let n = unsafe { c::read(fd, buf.as_mut_ptr().cast(), buf.len()) };
    if n < 0 {
        let code = std::io::Error::last_os_error().raw_os_error().unwrap_or(0);
        return Err(term_err("read", code));
    }
    Ok(n as usize)
}

/// Read a chunk — Track P: raw `SYS_read`.
#[cfg(feature = "track-p")]
pub fn read_chunk(fd: i32, buf: &mut [u8]) -> Result<usize> {
    rusty_libc::fd::read(fd, buf).map_err(|e| term_err("read", e.0))
}

/// Ignore `SIGTTOU` in this process — the D1 precondition
/// `give_terminal` must satisfy on every call: a background process that
/// calls `tcsetpgrp` stops on `SIGTTOU` by default, and a shell about to
/// hand off or reclaim the terminal is exactly that background process
/// from the kernel's point of view at the moment it isn't yet (or no
/// longer is) the foreground group. Idempotent: reinstalling `SIG_IGN`
/// is a harmless no-op, so `give_terminal` can call this every time
/// rather than trust a caller to have done it once at startup.
#[cfg(not(feature = "track-p"))]
fn ignore_sigttou() -> Result<()> {
    // SAFETY: `signal` accepts a handler value for a valid signal
    // number; `SIG_IGN` is a sentinel value, not a called function
    // pointer, so there is no handler-safety contract to uphold.
    let prev = unsafe { c::signal(c::SIGTTOU, c::SIG_IGN) };
    if prev == c::SIG_ERR {
        let code = std::io::Error::last_os_error().raw_os_error().unwrap_or(0);
        return Err(term_err("signal(SIGTTOU, SIG_IGN)", code));
    }
    Ok(())
}

/// Ignore `SIGTTOU` — Track P: rusty_libc's `signal`, same `SIG_IGN`
/// sentinel.
#[cfg(feature = "track-p")]
fn ignore_sigttou() -> Result<()> {
    // SAFETY: `SIG_IGN` is a sentinel value, not a called function
    // pointer, so rusty_libc's handler-safety contract does not apply.
    unsafe { rusty_libc::signal::signal(c::SIGTTOU, rusty_libc::signal::SIG_IGN) }
        .map(|_prev| ())
        .map_err(|e| term_err("signal(SIGTTOU, SIG_IGN)", e.0))
}

/// Hand the controlling terminal's foreground process group to `pgid`
/// (`tcsetpgrp(fd, pgid)`) — D1/D9's job-control terminal handoff, used
/// both to give a foreground job the terminal and, once it stops or
/// exits, to reclaim it for the shell's own group. Ensures `SIGTTOU` is
/// ignored in this process first (see `ignore_sigttou`) rather than
/// assuming the caller remembered to.
#[cfg(not(feature = "track-p"))]
pub fn give_terminal(fd: i32, pgid: c::pid_t) -> Result<()> {
    ignore_sigttou()?;
    // SAFETY: valid fd; tcsetpgrp has no pointer arguments.
    let r = unsafe { c::tcsetpgrp(fd, pgid) };
    if r != 0 {
        let code = std::io::Error::last_os_error().raw_os_error().unwrap_or(0);
        return Err(term_err("tcsetpgrp", code));
    }
    Ok(())
}

/// `give_terminal` — Track P: rusty_libc's `termios::tcsetpgrp`.
#[cfg(feature = "track-p")]
pub fn give_terminal(fd: i32, pgid: c::pid_t) -> Result<()> {
    ignore_sigttou()?;
    rusty_libc::termios::tcsetpgrp(fd, pgid).map_err(|e| term_err("tcsetpgrp", e.0))
}
