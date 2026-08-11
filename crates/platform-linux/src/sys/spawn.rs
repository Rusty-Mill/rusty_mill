//! Process spawning over `posix_spawn` (RFC v2 §5.4). Extraction map D1
//! informs the shape; `posix_spawn` is preferred over hand-rolled
//! fork+exec because it removes the async-signal-safe critical region
//! from this crate entirely — every allocation (CStrings, pointer arrays,
//! file actions) happens before the call, in the parent, which is the fix
//! standard for the v1 scaffold's B-1/B-2 bug class by construction.

#![allow(unsafe_code)]

use std::ffi::{CStr, CString, OsStr, OsString};
use std::os::fd::{FromRawFd, OwnedFd};
use std::os::unix::ffi::OsStrExt;

use platform::error::{ErrorKind, OsCode, PlatformError, Result};
use platform::process::{EnvSpec, ExitStatus, GroupSpec, Signal, Stdio};

use crate::ffi::libc_surface as c;

/// `pub(crate)`: shared with `sys::pty`, which builds its own
/// `posix_spawn` file actions/attributes for the "spawn attached to a pty
/// slave" path (`docs/design-discussion-pty.md`) rather than duplicating
/// this exact CString-conversion boilerplate.
pub(crate) fn to_cstring(s: &OsStr, op: &'static str) -> Result<CString> {
    CString::new(s.as_bytes())
        .map_err(|_| PlatformError::new(ErrorKind::InvalidInput, OsCode::None, op).with_path(s))
}

pub(crate) fn errno_err(op: &'static str, code: i32, path: &OsStr) -> PlatformError {
    let kind = match code {
        libc::ENOENT => ErrorKind::NotFound,
        libc::EACCES | libc::EPERM => ErrorKind::PermissionDenied,
        libc::ENOTDIR => ErrorKind::NotADirectory,
        libc::EINVAL => ErrorKind::InvalidInput,
        _ => ErrorKind::Other,
    };
    let e = PlatformError::new(kind, OsCode::Errno(code), op);
    if path.is_empty() {
        e
    } else {
        e.with_path(path)
    }
}

/// The environment for the child, fully materialized. `Inherit` snapshots
/// the parent's environment at spawn time (the same semantics std's
/// spawn has); `Explicit` contains exactly the given variables. `pub(crate)`
/// for `sys::pty`'s reuse — see [`to_cstring`].
pub(crate) fn build_env(env: &EnvSpec) -> Result<Vec<CString>> {
    let pairs: Vec<(OsString, OsString)> = match env {
        EnvSpec::Inherit => std::env::vars_os().collect(),
        EnvSpec::Explicit(map) => map.iter().map(|(k, v)| (k.clone(), v.clone())).collect(),
    };
    let mut out = Vec::with_capacity(pairs.len());
    for (k, v) in pairs {
        let mut kv = k;
        kv.push("=");
        kv.push(&v);
        out.push(to_cstring(&kv, "posix_spawn env")?);
    }
    Ok(out)
}

/// RAII for `posix_spawn_file_actions_t` so every early-error path
/// destroys what it initialized. `pub(crate)`, shared with `sys::pty`.
pub(crate) struct FileActions(pub(crate) c::posix_spawn_file_actions_t);

impl FileActions {
    pub(crate) fn new() -> Result<Self> {
        // SAFETY: `actions` is a valid out-pointer; init writes it before
        // any use, and the value is destroyed exactly once by Drop.
        let (r, actions) = unsafe {
            let mut actions: c::posix_spawn_file_actions_t = std::mem::zeroed();
            let r = c::posix_spawn_file_actions_init(&mut actions);
            (r, actions)
        };
        if r != 0 {
            return Err(errno_err(
                "posix_spawn_file_actions_init",
                r,
                OsStr::new(""),
            ));
        }
        Ok(Self(actions))
    }
}

impl Drop for FileActions {
    fn drop(&mut self) {
        // SAFETY: `self.0` was successfully initialized at construction
        // and is destroyed exactly once here.
        unsafe {
            c::posix_spawn_file_actions_destroy(&mut self.0);
        }
    }
}

/// RAII for `posix_spawnattr_t`, mirroring [`FileActions`]. `pub(crate)`,
/// shared with `sys::pty`.
pub(crate) struct SpawnAttr(pub(crate) c::posix_spawnattr_t);

impl SpawnAttr {
    pub(crate) fn new() -> Result<Self> {
        // SAFETY: `attr` is a valid out-pointer; init writes it before
        // any use, and the value is destroyed exactly once by Drop.
        let (r, attr) = unsafe {
            let mut attr: c::posix_spawnattr_t = std::mem::zeroed();
            let r = c::posix_spawnattr_init(&mut attr);
            (r, attr)
        };
        if r != 0 {
            return Err(errno_err("posix_spawnattr_init", r, OsStr::new("")));
        }
        Ok(Self(attr))
    }
}

impl Drop for SpawnAttr {
    fn drop(&mut self) {
        // SAFETY: `self.0` was successfully initialized at construction
        // and is destroyed exactly once here.
        unsafe {
            c::posix_spawnattr_destroy(&mut self.0);
        }
    }
}

/// `pipe2(O_CLOEXEC)` returning (read, write) as owned fds. CLOEXEC on
/// both ends: the child's copy of its end is re-dup2'd onto 0/1/2 by a
/// file action (which clears CLOEXEC on the target), and every other
/// copy closes at exec — no leaked pipe ends keeping a reader from EOF
/// (extraction map D5's deadlock class).
#[cfg(not(feature = "track-p"))]
fn make_pipe() -> Result<(OwnedFd, OwnedFd)> {
    let mut fds: [c::c_int; 2] = [0; 2];
    // SAFETY: `fds` is a valid out-array of exactly two ints.
    let r = unsafe { c::pipe2(fds.as_mut_ptr(), c::O_CLOEXEC) };
    if r != 0 {
        let code = std::io::Error::last_os_error().raw_os_error().unwrap_or(0);
        return Err(errno_err("pipe2", code, OsStr::new("")));
    }
    // SAFETY: both fds are freshly returned, valid, otherwise-unowned;
    // each is wrapped exactly once.
    Ok(unsafe { (OwnedFd::from_raw_fd(fds[0]), OwnedFd::from_raw_fd(fds[1])) })
}

/// Track P `make_pipe`: raw `SYS_pipe2`, same CLOEXEC discipline.
#[cfg(feature = "track-p")]
fn make_pipe() -> Result<(OwnedFd, OwnedFd)> {
    let (r, w) = rusty_libc::fd::pipe2(rusty_libc::fd::O_CLOEXEC)
        .map_err(|e| errno_err("pipe2", e.0, OsStr::new("")))?;
    // SAFETY: both fds are freshly returned, valid, otherwise-unowned;
    // each is wrapped exactly once.
    Ok(unsafe { (OwnedFd::from_raw_fd(r), OwnedFd::from_raw_fd(w)) })
}

/// Parent-side pipe ends for piped stdio slots: `[stdin write, stdout
/// read, stderr read]`.
pub type ParentPipes = [Option<OwnedFd>; 3];

/// Spawn `path` (an already-resolved program path) with `argv0` + `args`,
/// working directory `cwd`, environment per `env`, and the given stdio
/// wiring. `GroupSpec::NewGroup` makes the child a fresh process-group
/// leader before it executes (`POSIX_SPAWN_SETPGROUP` with pgroup 0 — the
/// race-free at-spawn placement; extraction map D1's double-`setpgid`
/// lesson is subsumed by the kernel doing it during spawn). `detached`
/// additionally sets `POSIX_SPAWN_SETSID` (`Command::detach`'s
/// contract): the child becomes a new session **and** process-group
/// leader, `pid == sid == pgid`, before its first instruction — combined
/// with a `target_pgid` from `group` (`NewGroup` only; `detached` +
/// `JoinGroup` is refused by the caller before this function is ever
/// reached, per `docs/decision-request-detach-liveness.md`), the
/// resulting `setpgid(0, 0)` `POSIX_SPAWN_SETPGROUP` performs is
/// POSIX-specified-harmless self-targeting: `setsid` already made pid ==
/// pgid, and a process may always `setpgid` itself to its own pid within
/// its own (here, brand new) session. Returns the child pid and the
/// parent ends of any pipes.
pub fn spawn(
    path: &OsStr,
    argv0: &OsStr,
    args: &[OsString],
    cwd: &OsStr,
    env: &EnvSpec,
    stdio: [&Stdio; 3],
    group: GroupSpec,
    detached: bool,
) -> Result<(c::pid_t, ParentPipes)> {
    // Every allocation happens here, before the spawn call (B-1/B-2 fix
    // standard): owned CStrings outlive the raw pointer arrays built from
    // them, and both outlive the call.
    let c_path = to_cstring(path, "posix_spawn")?;
    let c_cwd = to_cstring(cwd, "posix_spawn chdir")?;
    let mut c_argv: Vec<CString> = Vec::with_capacity(args.len() + 1);
    c_argv.push(to_cstring(argv0, "posix_spawn argv")?);
    for a in args {
        c_argv.push(to_cstring(a, "posix_spawn argv")?);
    }
    let c_env = build_env(env)?;

    let mut argv_ptrs: Vec<*mut c::c_char> = c_argv.iter().map(|s| s.as_ptr().cast_mut()).collect();
    argv_ptrs.push(std::ptr::null_mut());
    let mut env_ptrs: Vec<*mut c::c_char> = c_env.iter().map(|s| s.as_ptr().cast_mut()).collect();
    env_ptrs.push(std::ptr::null_mut());

    let mut actions = FileActions::new()?;
    // SAFETY: `actions.0` is initialized; `c_cwd` is a valid
    // NUL-terminated path outliving the spawn call.
    let r = unsafe { c::posix_spawn_file_actions_addchdir_np(&mut actions.0, c_cwd.as_ptr()) };
    if r != 0 {
        return Err(errno_err("posix_spawn chdir", r, cwd));
    }
    // Compile-time-checked literal, not a runtime-fallible conversion: a
    // `panic!` here can only fire if this literal is ever edited to
    // contain a NUL, in which case it fires at compile time.
    const DEVNULL: &CStr = match CStr::from_bytes_with_nul(b"/dev/null\0") {
        Ok(s) => s,
        Err(_) => panic!("DEVNULL literal must not contain an interior NUL"),
    };
    let devnull = DEVNULL;
    let mut parent_ends: ParentPipes = [None, None, None];
    // Child-side pipe ends stay alive (in this array) until after the
    // spawn call, then close in the parent as this function returns.
    let mut child_ends: [Option<OwnedFd>; 3] = [None, None, None];
    for (fd, spec) in stdio.iter().enumerate() {
        match spec {
            Stdio::Inherit => {}
            Stdio::Null => {
                let flags = if fd == 0 { c::O_RDONLY } else { c::O_WRONLY };
                // SAFETY: `actions.0` is initialized; `devnull` is a valid
                // NUL-terminated path outliving the spawn call.
                let r = unsafe {
                    c::posix_spawn_file_actions_addopen(
                        &mut actions.0,
                        fd as c::c_int,
                        devnull.as_ptr(),
                        flags,
                        0,
                    )
                };
                if r != 0 {
                    return Err(errno_err("posix_spawn addopen", r, OsStr::new("/dev/null")));
                }
            }
            Stdio::Pipe => {
                let (read, write) = make_pipe()?;
                // stdin: child reads (read end dup2'd onto 0), parent
                // writes; stdout/stderr: child writes, parent reads.
                let (child, parent) = if fd == 0 {
                    (read, write)
                } else {
                    (write, read)
                };
                use std::os::fd::AsRawFd;
                // SAFETY: `actions.0` is initialized; `child` is a valid
                // open fd that outlives the spawn call (held in
                // `child_ends`); dup2 onto 0/1/2 clears CLOEXEC on the
                // duplicate in the child.
                let r = unsafe {
                    c::posix_spawn_file_actions_adddup2(
                        &mut actions.0,
                        child.as_raw_fd(),
                        fd as c::c_int,
                    )
                };
                if r != 0 {
                    return Err(errno_err("posix_spawn adddup2", r, OsStr::new("")));
                }
                child_ends[fd] = Some(child);
                parent_ends[fd] = Some(parent);
            }
            Stdio::File(file) => {
                use std::os::fd::{AsFd, AsRawFd};
                // Unlike Pipe, no new fd is created here: the source
                // stays owned by the caller's `Stdio::File` (borrowed
                // for exactly this call — `stdio` outlives it), and
                // `adddup2` itself is the duplication, same as it is
                // for Pipe's already-open ends.
                let linux_file = file
                    .as_any()
                    .downcast_ref::<crate::fs::LinuxFile>()
                    .ok_or_else(|| {
                        PlatformError::new(
                            ErrorKind::Unsupported,
                            OsCode::None,
                            "posix_spawn: Stdio::File from a foreign backend",
                        )
                    })?;
                // SAFETY: `actions.0` is initialized; `linux_file`'s fd
                // is open and valid for the duration of this call. dup2
                // onto 0/1/2 clears CLOEXEC on the duplicate in the
                // child even when the source itself is CLOEXEC
                // (`Dir::open`'s default) — fork copies the whole fd
                // table regardless of CLOEXEC, and `adddup2`'s dup2
                // runs in the child before exec, the same reasoning the
                // Pipe arm above already documents.
                let r = unsafe {
                    c::posix_spawn_file_actions_adddup2(
                        &mut actions.0,
                        linux_file.as_fd().as_raw_fd(),
                        fd as c::c_int,
                    )
                };
                if r != 0 {
                    return Err(errno_err("posix_spawn adddup2", r, OsStr::new("")));
                }
            }
        }
    }

    let mut attr = SpawnAttr::new()?;
    // `NewGroup` leads a fresh group (pgroup 0 means "use my own pid");
    // `JoinGroup(pgid)` places the child straight into an existing one —
    // D1's pipeline shape, where stage 2..n join the leader's pgid
    // instead of each starting its own. Both go through the same
    // race-free `POSIX_SPAWN_SETPGROUP` path: set before the child's
    // first instruction, never a post-spawn `setpgid`.
    let target_pgid: Option<c::pid_t> = match group {
        GroupSpec::Inherit => None,
        GroupSpec::JoinGroup(pgid) => Some(pgid as c::pid_t),
        // `detached` already gives the child its own pgid via `setsid`
        // below; an explicit `setpgid(0, 0)` on top is harmless
        // self-targeting (this function's own doc comment), so `NewGroup`
        // still requests it uniformly rather than special-casing the
        // `detached` combination away.
        GroupSpec::NewGroup => Some(0),
    };
    // `POSIX_SPAWN_SETPGROUP` and `POSIX_SPAWN_SETSID` are a single flags
    // word (`posix_spawnattr_setflags` *replaces*, not accumulates) — set
    // once with both bits present rather than two calls that would
    // silently drop the first.
    let mut spawn_flags = 0;
    if target_pgid.is_some() {
        spawn_flags |= c::POSIX_SPAWN_SETPGROUP as _;
    }
    if detached {
        spawn_flags |= c::POSIX_SPAWN_SETSID as _;
    }
    if spawn_flags != 0 {
        // SAFETY: `attr.0` is initialized; setflags/setpgroup have no
        // pointer arguments beyond it.
        let r = unsafe {
            let r = c::posix_spawnattr_setflags(&mut attr.0, spawn_flags);
            if r == 0 {
                match target_pgid {
                    Some(pgid) => c::posix_spawnattr_setpgroup(&mut attr.0, pgid),
                    None => 0,
                }
            } else {
                r
            }
        };
        if r != 0 {
            return Err(errno_err("posix_spawnattr_setpgroup", r, OsStr::new("")));
        }
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
        return Err(errno_err("posix_spawn", r, path));
    }
    // The child's pipe ends close here (`child_ends` drops) — the parent
    // holding a stray copy of a write end would starve the read end of
    // EOF forever (extraction map D5's documented deadlock).
    drop(child_ends);
    Ok((pid, parent_ends))
}

/// `pidfd_open(pid, 0)`, wrapped as an owned fd. `ENOSYS` (pre-5.3 kernel)
/// surfaces as `Unsupported` — [`poll_pids`]'s caller falls back to the
/// portable poll loop rather than treating it as a hard failure.
///
/// There is no libc wrapper for `pidfd_open` at this repo's MSRV
/// baseline, hence the raw syscall in the non-track-p arm — the same
/// situation `renameat2` was in before the symlink slice (see
/// `libc_surface`'s doc comment). Track P closed this specific gap: this
/// was the last raw-`syscall()` escape hatch left in the crate once
/// `rusty_libc::process::pidfd_open` landed.
#[cfg(not(feature = "track-p"))]
fn pidfd_open(pid: c::pid_t) -> Result<OwnedFd> {
    // SAFETY: pidfd_open takes (pid, flags) and returns an fd or -1; no
    // pointer arguments.
    let fd = unsafe { c::syscall(c::SYS_pidfd_open, pid, 0u32) };
    if fd < 0 {
        let code = std::io::Error::last_os_error().raw_os_error().unwrap_or(0);
        if code == libc::ENOSYS {
            return Err(PlatformError::new(
                ErrorKind::Unsupported,
                OsCode::Errno(code),
                "pidfd_open",
            ));
        }
        return Err(errno_err("pidfd_open", code, OsStr::new("")));
    }
    // SAFETY: `fd` is a freshly returned, valid, otherwise-unowned
    // descriptor; wrapped exactly once.
    Ok(unsafe { OwnedFd::from_raw_fd(fd as c::c_int) })
}

/// `pidfd_open(pid, 0)` — Track P: `rusty_libc::process::pidfd_open`.
#[cfg(feature = "track-p")]
fn pidfd_open(pid: c::pid_t) -> Result<OwnedFd> {
    match rusty_libc::process::pidfd_open(pid, 0) {
        // SAFETY: `fd` is a freshly returned, valid, otherwise-unowned
        // descriptor; wrapped exactly once.
        Ok(fd) => Ok(unsafe { OwnedFd::from_raw_fd(fd) }),
        Err(e) if e.code() == libc::ENOSYS => Err(PlatformError::new(
            ErrorKind::Unsupported,
            OsCode::Errno(e.code()),
            "pidfd_open",
        )),
        Err(e) => Err(errno_err("pidfd_open", e.code(), OsStr::new(""))),
    }
}

/// Multiplexed wait over pidfds (RFC v2 §5.6 reactor internals, R3):
/// `pidfd_open` each pid, `poll` the set with `timeout` (`None` =
/// forever), and return `Some(position)` of a readable pidfd (= that
/// process terminated, waitable without blocking) or `None` on timeout.
/// `Err` with `Unsupported` if the kernel lacks `pidfd_open` (pre-5.3) —
/// the caller falls back to the portable poll loop.
pub fn poll_pids(pids: &[c::pid_t], timeout: Option<std::time::Duration>) -> Result<Option<usize>> {
    let mut pidfds: Vec<OwnedFd> = Vec::with_capacity(pids.len());
    for &pid in pids {
        pidfds.push(pidfd_open(pid)?);
    }

    use std::os::fd::AsRawFd;
    let mut fds: Vec<PollEntry> = pidfds
        .iter()
        .map(|fd| PollEntry {
            fd: fd.as_raw_fd(),
            events: POLL_IN,
            revents: 0,
        })
        .collect();
    let deadline = timeout.map(|t| std::time::Instant::now() + t);
    loop {
        let timeout_ms: c::c_int = match deadline {
            None => -1,
            Some(d) => {
                let left = d.saturating_duration_since(std::time::Instant::now());
                left.as_millis().min(i32::MAX as u128) as c::c_int
            }
        };
        match poll_once(&mut fds, timeout_ms) {
            Ok(0) => return Ok(None),
            Ok(_) => {
                let hit = fds.iter().position(|p| p.revents != 0).ok_or_else(|| {
                    PlatformError::new(
                        ErrorKind::Other,
                        OsCode::None,
                        "poll reported readiness but no pollfd shows revents",
                    )
                })?;
                return Ok(Some(hit));
            }
            Err(e) if e.os == OsCode::Errno(libc::EINTR) => continue,
            Err(e) => return Err(e),
        }
    }
}

/// The `poll(2)` entry type: the kernel's `struct pollfd` under either
/// backend, with identical field names — the construction above is shared.
#[cfg(not(feature = "track-p"))]
use crate::ffi::libc_surface::pollfd as PollEntry;
#[cfg(feature = "track-p")]
use rusty_libc::fd::PollFd as PollEntry;
#[cfg(not(feature = "track-p"))]
const POLL_IN: i16 = c::POLLIN;
#[cfg(feature = "track-p")]
const POLL_IN: i16 = rusty_libc::fd::POLLIN;

/// One `poll(2)` round: `Ok(count)` of ready entries (0 = timeout);
/// `EINTR` surfaces as an `Err` the caller's loop retries on.
#[cfg(not(feature = "track-p"))]
fn poll_once(fds: &mut [PollEntry], timeout_ms: c::c_int) -> Result<usize> {
    // SAFETY: `fds` is a valid array of exactly `fds.len()` pollfds, each
    // holding an open fd owned by the caller, all outliving the call.
    let r = unsafe { c::poll(fds.as_mut_ptr(), fds.len() as c::nfds_t, timeout_ms) };
    if r < 0 {
        let code = std::io::Error::last_os_error().raw_os_error().unwrap_or(0);
        return Err(errno_err("poll", code, OsStr::new("")));
    }
    Ok(r as usize)
}

/// One `poll(2)` round — Track P: raw `SYS_poll` (`SYS_ppoll` on aarch64,
/// absorbed inside rusty_libc; the removed-syscall lesson from extraction
/// map D4 handled at the dependency's layer, not ours).
#[cfg(feature = "track-p")]
fn poll_once(fds: &mut [PollEntry], timeout_ms: c::c_int) -> Result<usize> {
    rusty_libc::fd::poll(fds, timeout_ms).map_err(|e| errno_err("poll", e.0, OsStr::new("")))
}

/// Map the portable [`Signal`] to its raw Linux signal number.
fn signum_of(sig: Signal) -> c::c_int {
    match sig {
        Signal::Term => c::SIGTERM,
        Signal::Int => c::SIGINT,
        Signal::Hup => c::SIGHUP,
        Signal::Quit => c::SIGQUIT,
        Signal::Kill => c::SIGKILL,
        Signal::Stop => c::SIGSTOP,
        Signal::Cont => c::SIGCONT,
    }
}

/// Deliver `sig` to the whole process group led by `pid` (which must have
/// been spawned with `GroupSpec::NewGroup`, making pid == pgid, or be an
/// explicit `JoinGroup` target passed in directly by the caller).
#[cfg(not(feature = "track-p"))]
pub fn kill_group(pid: c::pid_t, sig: Signal) -> Result<()> {
    // SAFETY: kill has no pointer arguments; the negative-pid form
    // targets the process group `pid` leads.
    let r = unsafe { c::kill(-pid, signum_of(sig)) };
    if r != 0 {
        let code = std::io::Error::last_os_error().raw_os_error().unwrap_or(0);
        return Err(errno_err("kill", code, OsStr::new("")));
    }
    Ok(())
}

/// Deliver `sig` to the process group — Track P: raw `SYS_kill` via
/// `killpg` (the negative-pid form, named).
#[cfg(feature = "track-p")]
pub fn kill_group(pid: c::pid_t, sig: Signal) -> Result<()> {
    rusty_libc::process::killpg(pid, signum_of(sig))
        .map_err(|e| errno_err("kill", e.0, OsStr::new("")))
}

/// Deliver `sig` to the single process `pid`.
#[cfg(not(feature = "track-p"))]
pub fn kill_single(pid: c::pid_t, sig: Signal) -> Result<()> {
    // SAFETY: kill has no pointer arguments.
    let r = unsafe { c::kill(pid, signum_of(sig)) };
    if r != 0 {
        let code = std::io::Error::last_os_error().raw_os_error().unwrap_or(0);
        return Err(errno_err("kill", code, OsStr::new("")));
    }
    Ok(())
}

/// Deliver `sig` to the single process `pid` — Track P: raw `SYS_kill`.
#[cfg(feature = "track-p")]
pub fn kill_single(pid: c::pid_t, sig: Signal) -> Result<()> {
    rusty_libc::process::kill(pid, signum_of(sig))
        .map_err(|e| errno_err("kill", e.0, OsStr::new("")))
}

/// Portable liveness probe (`Spawner::is_alive`): `kill(pid, 0)` —
/// POSIX's own no-op signal-existence check, valid for any pid this
/// process didn't necessarily spawn. `Ok(true)` on success (permitted to
/// signal it) or `EPERM` (exists, just not signalable by this process);
/// `Ok(false)` on `ESRCH` (no such process — already reaped, or never
/// existed).
#[cfg(not(feature = "track-p"))]
pub fn is_alive(pid: c::pid_t) -> Result<bool> {
    // SAFETY: kill has no pointer arguments; signal 0 delivers nothing —
    // POSIX specifies it as existence/permission-only probe.
    let r = unsafe { c::kill(pid, 0) };
    if r == 0 {
        return Ok(true);
    }
    let code = std::io::Error::last_os_error().raw_os_error().unwrap_or(0);
    if code == c::ESRCH {
        Ok(false)
    } else if code == c::EPERM {
        Ok(true)
    } else {
        Err(errno_err("kill", code, OsStr::new("")))
    }
}

/// Liveness probe — Track P: raw `SYS_kill` via `rusty_libc::process::kill`.
#[cfg(feature = "track-p")]
pub fn is_alive(pid: c::pid_t) -> Result<bool> {
    match rusty_libc::process::kill(pid, 0) {
        Ok(()) => Ok(true),
        Err(e) if e.0 == c::ESRCH => Ok(false),
        Err(e) if e.0 == c::EPERM => Ok(true),
        Err(e) => Err(errno_err("kill", e.0, OsStr::new(""))),
    }
}

/// Is `pid` a zombie (`Spawner::is_zombie`)? Reads `/proc/<pid>/stat`'s
/// state field — the token right after the `comm` field's closing `)`.
/// Splits on the *last* `)` rather than naively tokenizing: `comm` (a
/// user-settable process name) can itself contain spaces or parens. A
/// missing `/proc` entry means the pid doesn't exist at all —
/// `Ok(false)` (not a zombie, gone), not an error. Plain `std::fs`, no
/// `libc`/syscall involved, so identical under every feature
/// configuration — not `track-p`-gated.
pub fn is_zombie(pid: c::pid_t) -> Result<bool> {
    let path = format!("/proc/{pid}/stat");
    let contents = match std::fs::read_to_string(&path) {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(e) => {
            let code = e.raw_os_error().unwrap_or(0);
            return Err(errno_err("read /proc/<pid>/stat", code, OsStr::new(&path)));
        }
    };
    let after_comm = contents
        .rsplit_once(')')
        .map(|(_, rest)| rest)
        .ok_or_else(|| {
            PlatformError::new(ErrorKind::Other, OsCode::None, "parse /proc/<pid>/stat")
                .with_path(OsStr::new(&path))
        })?;
    let state = after_comm.split_whitespace().next().ok_or_else(|| {
        PlatformError::new(ErrorKind::Other, OsCode::None, "parse /proc/<pid>/stat")
            .with_path(OsStr::new(&path))
    })?;
    Ok(state == "Z")
}

#[cfg(not(feature = "track-p"))]
fn decode(status: c::c_int) -> ExitStatus {
    if c::WIFEXITED(status) {
        ExitStatus::Code(c::WEXITSTATUS(status))
    } else if c::WIFSIGNALED(status) {
        ExitStatus::Signaled(c::WTERMSIG(status))
    } else {
        // Stop/continue events are impossible without WUNTRACED/
        // WCONTINUED flags; classify defensively rather than panic.
        ExitStatus::Code(1)
    }
}

/// Track P status decode: same W* bit tests, rusty_libc's plain-fn
/// versions of what libc ships as macros. The raw status word is the
/// kernel's in both cases — the decoders agree bit for bit.
#[cfg(feature = "track-p")]
fn decode(status: c::c_int) -> ExitStatus {
    use rusty_libc::wait as rw;
    if rw::wifexited(status) {
        ExitStatus::Code(rw::wexitstatus(status))
    } else if rw::wifsignaled(status) {
        ExitStatus::Signaled(rw::wtermsig(status))
    } else {
        // Stop/continue events are impossible without WUNTRACED/
        // WCONTINUED flags; classify defensively rather than panic.
        ExitStatus::Code(1)
    }
}

/// Blocking `waitpid` on `pid`, decoding the raw status word into the
/// uniform [`ExitStatus`] (B-5: the raw word never crosses this boundary).
#[cfg(not(feature = "track-p"))]
pub fn wait(pid: c::pid_t) -> Result<ExitStatus> {
    let mut status: c::c_int = 0;
    loop {
        // SAFETY: `status` is a valid out-pointer; `pid` is a child this
        // process spawned and has not yet waited on (enforced by the
        // consuming `Child::wait` above this layer).
        let r = unsafe { c::waitpid(pid, &mut status, 0) };
        if r == pid {
            break;
        }
        let code = std::io::Error::last_os_error().raw_os_error().unwrap_or(0);
        if code == libc::EINTR {
            continue;
        }
        return Err(errno_err("waitpid", code, OsStr::new("")));
    }
    Ok(decode(status))
}

/// Blocking wait — Track P: raw `SYS_wait4` (the kernel has no `waitpid`
/// syscall; libc's waitpid IS wait4 with a null rusage, and rusty_libc
/// makes that explicit).
#[cfg(feature = "track-p")]
pub fn wait(pid: c::pid_t) -> Result<ExitStatus> {
    loop {
        match rusty_libc::wait::waitpid(pid, 0) {
            Ok((_, status)) => return Ok(decode(status)),
            Err(e) if e == rusty_libc::Errno::EINTR => continue,
            Err(e) => return Err(errno_err("waitpid", e.0, OsStr::new(""))),
        }
    }
}

/// Non-blocking `waitpid(WNOHANG)`: `Some(decoded)` if `pid` terminated
/// (the zombie is reaped — the caller must stash the result), `None` if
/// still running.
#[cfg(not(feature = "track-p"))]
pub fn try_wait(pid: c::pid_t) -> Result<Option<ExitStatus>> {
    let mut status: c::c_int = 0;
    loop {
        // SAFETY: `status` is a valid out-pointer; `pid` is a child this
        // process spawned and has not reaped yet (the caller stashes the
        // result of a successful poll and never polls again).
        let r = unsafe { c::waitpid(pid, &mut status, c::WNOHANG) };
        if r == pid {
            return Ok(Some(decode(status)));
        }
        if r == 0 {
            return Ok(None);
        }
        let code = std::io::Error::last_os_error().raw_os_error().unwrap_or(0);
        if code == libc::EINTR {
            continue;
        }
        return Err(errno_err("waitpid", code, OsStr::new("")));
    }
}

/// Non-blocking wait — Track P: raw `SYS_wait4` with `WNOHANG`; a
/// returned pid of 0 means "still running".
#[cfg(feature = "track-p")]
pub fn try_wait(pid: c::pid_t) -> Result<Option<ExitStatus>> {
    loop {
        match rusty_libc::wait::waitpid(pid, rusty_libc::wait::WNOHANG) {
            Ok((0, _)) => return Ok(None),
            Ok((_, status)) => return Ok(Some(decode(status))),
            Err(e) if e == rusty_libc::Errno::EINTR => continue,
            Err(e) => return Err(errno_err("waitpid", e.0, OsStr::new(""))),
        }
    }
}

/// D10 status decode: `decode`'s exit/signal arms plus the
/// `WUNTRACED`/`WCONTINUED` stop/continue arms — only reachable from
/// [`wait_job`]/[`try_wait_job`], which are the only callers that ever
/// pass those flags to `waitpid` in the first place.
#[cfg(not(feature = "track-p"))]
fn decode_job(status: c::c_int) -> ExitStatus {
    if c::WIFEXITED(status) {
        ExitStatus::Code(c::WEXITSTATUS(status))
    } else if c::WIFSIGNALED(status) {
        ExitStatus::Signaled(c::WTERMSIG(status))
    } else if c::WIFSTOPPED(status) {
        ExitStatus::Stopped(c::WSTOPSIG(status))
    } else {
        // The only bit pattern left once WIFEXITED/WIFSIGNALED/
        // WIFSTOPPED are ruled out and WCONTINUED was requested.
        ExitStatus::Continued
    }
}

/// Track P `decode_job`: same arm order, rusty_libc's plain-fn W* set.
#[cfg(feature = "track-p")]
fn decode_job(status: c::c_int) -> ExitStatus {
    use rusty_libc::wait as rw;
    if rw::wifexited(status) {
        ExitStatus::Code(rw::wexitstatus(status))
    } else if rw::wifsignaled(status) {
        ExitStatus::Signaled(rw::wtermsig(status))
    } else if rw::wifstopped(status) {
        ExitStatus::Stopped(rw::wstopsig(status))
    } else {
        ExitStatus::Continued
    }
}

/// Blocking `waitpid(WUNTRACED|WCONTINUED)` on `pid` (D10): the
/// Ctrl-Z-aware counterpart to [`wait`] — observes a stop or a resume in
/// addition to exit/signal termination, decoded through `decode_job`.
#[cfg(not(feature = "track-p"))]
pub fn wait_job(pid: c::pid_t) -> Result<ExitStatus> {
    let mut status: c::c_int = 0;
    loop {
        // SAFETY: `status` is a valid out-pointer; `pid` is a child this
        // process spawned. Unlike plain `wait`, a returned status here
        // may be non-terminal (`Stopped`/`Continued`), so the caller
        // (this layer's `Child::wait_job`) may call again on the same
        // still-alive `pid` — sound because a non-terminal status never
        // reaps the process (only `WIFEXITED`/`WIFSIGNALED` do).
        let r = unsafe { c::waitpid(pid, &mut status, c::WUNTRACED | c::WCONTINUED) };
        if r == pid {
            break;
        }
        let code = std::io::Error::last_os_error().raw_os_error().unwrap_or(0);
        if code == libc::EINTR {
            continue;
        }
        return Err(errno_err("waitpid", code, OsStr::new("")));
    }
    Ok(decode_job(status))
}

/// Blocking job-aware wait — Track P: raw `SYS_wait4` with
/// `WUNTRACED|WCONTINUED`.
#[cfg(feature = "track-p")]
pub fn wait_job(pid: c::pid_t) -> Result<ExitStatus> {
    use rusty_libc::wait::{WCONTINUED, WUNTRACED};
    loop {
        match rusty_libc::wait::waitpid(pid, WUNTRACED | WCONTINUED) {
            Ok((_, status)) => return Ok(decode_job(status)),
            Err(e) if e == rusty_libc::Errno::EINTR => continue,
            Err(e) => return Err(errno_err("waitpid", e.0, OsStr::new(""))),
        }
    }
}

/// Non-blocking `waitpid(WNOHANG|WUNTRACED|WCONTINUED)` on `pid` (D10):
/// the Ctrl-Z-aware counterpart to [`try_wait`]. `None` while `pid` is
/// running and has neither stopped nor continued since the last poll.
#[cfg(not(feature = "track-p"))]
pub fn try_wait_job(pid: c::pid_t) -> Result<Option<ExitStatus>> {
    let mut status: c::c_int = 0;
    loop {
        // SAFETY: same as `wait_job`, plus `WNOHANG`'s non-blocking
        // contract (a returned pid of 0 means "nothing to report yet").
        let r = unsafe { c::waitpid(pid, &mut status, c::WNOHANG | c::WUNTRACED | c::WCONTINUED) };
        if r == pid {
            return Ok(Some(decode_job(status)));
        }
        if r == 0 {
            return Ok(None);
        }
        let code = std::io::Error::last_os_error().raw_os_error().unwrap_or(0);
        if code == libc::EINTR {
            continue;
        }
        return Err(errno_err("waitpid", code, OsStr::new("")));
    }
}

/// Non-blocking job-aware wait — Track P: raw `SYS_wait4` with
/// `WNOHANG|WUNTRACED|WCONTINUED`.
#[cfg(feature = "track-p")]
pub fn try_wait_job(pid: c::pid_t) -> Result<Option<ExitStatus>> {
    use rusty_libc::wait::{WCONTINUED, WNOHANG, WUNTRACED};
    loop {
        match rusty_libc::wait::waitpid(pid, WNOHANG | WUNTRACED | WCONTINUED) {
            Ok((0, _)) => return Ok(None),
            Ok((_, status)) => return Ok(Some(decode_job(status))),
            Err(e) if e == rusty_libc::Errno::EINTR => continue,
            Err(e) => return Err(errno_err("waitpid", e.0, OsStr::new(""))),
        }
    }
}
