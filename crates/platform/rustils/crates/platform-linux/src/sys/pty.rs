//! PTY primitives (RFC v2 R5+, D13, convergence roadmap Phase 7,
//! rustils#82): opening a pty pair, spawning a child attached to its
//! slave, resize, and master read/write. Not track-p-gated — like
//! `sys::net`/`sys::tun`, PTY hosting was never in rush's required
//! surface, so `rusty_libc` has nothing here to route through yet; one
//! implementation for both configurations.
//!
//! **No raw `fork`.** `sys::spawn` already exists specifically to keep
//! `fork`+`pre_exec`'s async-signal-safety hazard out of this crate
//! (its own module doc); reopening that for PTY hosting isn't this
//! module's call to make, since the roadmap parks the `fork`-vs-
//! `posix_spawn` question as its own separate, still-undecided owner
//! decision. Instead, `spawn_attached` reaches the same outcome shh's
//! donor `fork`+`TIOCSCTTY` gets — the child ends up a session leader
//! with the pty slave as its controlling terminal — through
//! `posix_spawn`'s own mechanism: `POSIX_SPAWN_SETSID` (a glibc
//! extension) makes the child call `setsid()` before its file actions
//! run, and then a file action opens the slave **by pathname** (not a
//! `dup2` of an already-open fd) for fd 0. Opening a terminal device by
//! path, without `O_NOCTTY`, from a session leader with no controlling
//! terminal yet is standard POSIX/Linux behavior that assigns it as the
//! controlling terminal automatically — see
//! `docs/design-discussion-pty.md`'s "The `posix_spawn` substitute for
//! `fork`+`TIOCSCTTY`" section for the full reasoning. This is the kind
//! of thing that needs live verification, not just inspection — the
//! integration test spawns `sh -c 'tty'` and checks the printed path
//! matches the slave, not just that `posix_spawn` returned success.

#![allow(unsafe_code)]

use std::ffi::{CStr, CString, OsStr, OsString};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};

use platform::error::{ErrorKind, OsCode, PlatformError, Result};
use platform::process::EnvSpec;
use platform::term::WinSize;

use crate::ffi::libc_surface as c;
use crate::sys::spawn;

fn errno() -> i32 {
    std::io::Error::last_os_error().raw_os_error().unwrap_or(0)
}

fn pty_err(op: &'static str) -> PlatformError {
    let code = errno();
    let kind = match code {
        libc::ENOENT => ErrorKind::NotFound,
        libc::EACCES | libc::EPERM => ErrorKind::PermissionDenied,
        libc::EAGAIN => ErrorKind::WouldBlock,
        libc::EINTR => ErrorKind::Interrupted,
        _ => ErrorKind::Other,
    };
    PlatformError::new(kind, OsCode::Errno(code), op)
}

/// Open a fresh pty pair: `posix_openpt` (master, `O_NOCTTY` — the master
/// is never anyone's controlling terminal) + `grantpt` + `unlockpt`
/// (mandatory: the slave can't be opened at all until this runs) +
/// `ptsname_r` for the slave's device path. The master fd is
/// `O_CLOEXEC` — only the slave crosses into the spawned child, by
/// pathname, in [`spawn_attached`], not by fd inheritance.
pub fn open_pty_pair() -> Result<(OwnedFd, CString)> {
    // SAFETY: posix_openpt has no pointer arguments; a negative return is
    // the documented error sentinel (checked immediately below), and any
    // non-negative return is a freshly opened, otherwise-unowned fd.
    let fd = unsafe { c::posix_openpt(c::O_RDWR | c::O_NOCTTY | c::O_CLOEXEC) };
    if fd < 0 {
        return Err(pty_err("posix_openpt"));
    }
    // SAFETY: `fd` is a freshly returned, valid, otherwise-unowned
    // descriptor; wrapped exactly once.
    let master = unsafe { OwnedFd::from_raw_fd(fd) };

    // SAFETY: `master` is a valid open pty master fd for the duration of
    // this call.
    let r = unsafe { c::grantpt(master.as_raw_fd()) };
    if r != 0 {
        return Err(pty_err("grantpt"));
    }
    // SAFETY: same as above. Mandatory — the slave device stays locked
    // (opens fail `EIO`) until this succeeds.
    let r = unsafe { c::unlockpt(master.as_raw_fd()) };
    if r != 0 {
        return Err(pty_err("unlockpt"));
    }

    // `/dev/pts/<n>` paths are short; 64 bytes is generous headroom over
    // any real value glibc's devpts implementation produces.
    let mut buf = [0u8; 64];
    // SAFETY: `master` is a valid open pty master fd; `buf` is a valid,
    // writable region of exactly `buf.len()` bytes for the duration of
    // the call, and `ptsname_r` NUL-terminates within it on success
    // (checked below) rather than writing past its length.
    let r = unsafe {
        c::ptsname_r(
            master.as_raw_fd(),
            buf.as_mut_ptr().cast(),
            buf.len() as c::size_t,
        )
    };
    if r != 0 {
        return Err(pty_err("ptsname_r"));
    }
    // SAFETY: `ptsname_r` wrote a NUL-terminated string within `buf` on
    // success (just checked above).
    let slave_path = unsafe { CStr::from_ptr(buf.as_ptr().cast()) }.to_owned();

    Ok((master, slave_path))
}

/// Spawn `path` (already-resolved) attached to `slave_path` as its
/// controlling terminal and sole stdio — see this module's doc comment
/// for the `posix_spawn`-native mechanism. Returns the child pid.
pub fn spawn_attached(
    path: &OsStr,
    argv0: &OsStr,
    args: &[OsString],
    cwd: &OsStr,
    env: &EnvSpec,
    slave_path: &CStr,
) -> Result<c::pid_t> {
    // Every allocation happens here, before the spawn call — the same
    // B-1/B-2 fix standard `sys::spawn::spawn`'s own doc comment states.
    let c_path = spawn::to_cstring(path, "posix_spawn")?;
    let c_cwd = spawn::to_cstring(cwd, "posix_spawn chdir")?;
    let mut c_argv: Vec<CString> = Vec::with_capacity(args.len() + 1);
    c_argv.push(spawn::to_cstring(argv0, "posix_spawn argv")?);
    for a in args {
        c_argv.push(spawn::to_cstring(a, "posix_spawn argv")?);
    }
    let c_env = spawn::build_env(env)?;

    let mut argv_ptrs: Vec<*mut c::c_char> = c_argv.iter().map(|s| s.as_ptr().cast_mut()).collect();
    argv_ptrs.push(std::ptr::null_mut());
    let mut env_ptrs: Vec<*mut c::c_char> = c_env.iter().map(|s| s.as_ptr().cast_mut()).collect();
    env_ptrs.push(std::ptr::null_mut());

    let mut actions = spawn::FileActions::new()?;
    // SAFETY: `actions.0` is initialized; `c_cwd` is a valid
    // NUL-terminated path outliving the spawn call.
    let r = unsafe { c::posix_spawn_file_actions_addchdir_np(&mut actions.0, c_cwd.as_ptr()) };
    if r != 0 {
        return Err(spawn::errno_err("posix_spawn chdir", r, cwd));
    }

    // Open the slave **by pathname** for fd 0 — not a `dup2` of an
    // already-open fd, since the open() call itself, from the
    // about-to-be session leader `POSIX_SPAWN_SETSID` below produces, is
    // what assigns the controlling terminal (this module's doc comment).
    // SAFETY: `actions.0` is initialized; `slave_path` is a valid
    // NUL-terminated path outliving the spawn call.
    let r = unsafe {
        c::posix_spawn_file_actions_addopen(&mut actions.0, 0, slave_path.as_ptr(), c::O_RDWR, 0)
    };
    if r != 0 {
        return Err(spawn::errno_err("posix_spawn addopen", r, OsStr::new("")));
    }
    // SAFETY: `actions.0` is initialized; fd 0 was just opened by the
    // action above, ordered before these run.
    let r = unsafe { c::posix_spawn_file_actions_adddup2(&mut actions.0, 0, 1) };
    if r != 0 {
        return Err(spawn::errno_err("posix_spawn adddup2", r, OsStr::new("")));
    }
    // SAFETY: same as above.
    let r = unsafe { c::posix_spawn_file_actions_adddup2(&mut actions.0, 0, 2) };
    if r != 0 {
        return Err(spawn::errno_err("posix_spawn adddup2", r, OsStr::new("")));
    }

    let mut attr = spawn::SpawnAttr::new()?;
    // SAFETY: `attr.0` is initialized; setflags has no pointer arguments
    // beyond it. Runs before file actions/exec (glibc's `__spawni`
    // ordering) — the session (and therefore "no controlling terminal
    // yet") is established before the addopen action above runs.
    let r = unsafe { c::posix_spawnattr_setflags(&mut attr.0, c::POSIX_SPAWN_SETSID as _) };
    if r != 0 {
        return Err(spawn::errno_err(
            "posix_spawnattr_setflags",
            r,
            OsStr::new(""),
        ));
    }

    let mut pid: c::pid_t = 0;
    // SAFETY: every pointer argument references an owned value that
    // outlives this call (`c_path`, the NUL-terminated `argv_ptrs`/
    // `env_ptrs` arrays whose elements point into `c_argv`/`c_env`, the
    // initialized `actions.0` and `attr.0`); `pid` is a valid
    // out-pointer.
    let r = unsafe {
        c::posix_spawn(
            &mut pid,
            c_path.as_ptr(),
            &actions.0,
            &attr.0,
            argv_ptrs.as_ptr(),
            env_ptrs.as_ptr(),
        )
    };
    if r != 0 {
        return Err(spawn::errno_err("posix_spawn", r, path));
    }
    Ok(pid)
}

/// `ioctl(master_fd, TIOCSWINSZ)` — `TIOCGWINSZ`'s write-side sibling
/// (`sys::termios::window_size`'s read counterpart). Valid on either the
/// master or slave fd of a pty pair; the master is what
/// [`crate::pty::LinuxPtyMaster`] holds.
pub fn resize(master: &OwnedFd, size: WinSize) -> Result<()> {
    let ws = c::winsize {
        ws_row: size.rows,
        ws_col: size.cols,
        ws_xpixel: 0,
        ws_ypixel: 0,
    };
    // SAFETY: `master` is a valid open pty master fd; `ws` is a valid,
    // fully-initialized `winsize` the kernel reads from for the duration
    // of the call.
    let r = unsafe { c::ioctl(master.as_raw_fd(), c::TIOCSWINSZ, &ws) };
    if r != 0 {
        return Err(pty_err("TIOCSWINSZ"));
    }
    Ok(())
}

/// `read(2)` from the pty master, with `EIO` (the kernel's "slave side
/// closed" signal) translated to `Ok(0)` — this trait's own EOF
/// contract (`platform::pty`'s module doc), not a raw errno a caller
/// would otherwise have to special-case per OS.
pub fn read(master: &OwnedFd, buf: &mut [u8]) -> Result<usize> {
    // SAFETY: `buf` is a valid, writable region of exactly `buf.len()`
    // bytes for the duration of the call; `master` is a valid open fd.
    let n = unsafe { c::read(master.as_raw_fd(), buf.as_mut_ptr().cast(), buf.len()) };
    if n < 0 {
        let code = errno();
        if code == libc::EIO {
            return Ok(0);
        }
        return Err(pty_err("read"));
    }
    Ok(n as usize)
}

/// `write(2)` to the pty master.
pub fn write(master: &OwnedFd, buf: &[u8]) -> Result<usize> {
    // SAFETY: `buf` is a valid readable region of exactly `buf.len()`
    // bytes for the duration of the call; `master` is a valid open fd.
    let n = unsafe { c::write(master.as_raw_fd(), buf.as_ptr().cast(), buf.len()) };
    if n < 0 {
        return Err(pty_err("write"));
    }
    Ok(n as usize)
}
