//! Process adapter: the small, direct OS primitives this project needs
//! that `rusty_tokio::process::Command`/`Child` do not cover.
//!
//! Everything here is about one of three things `rusty_tokio` has no
//! reason to wrap:
//!
//! 1. **A pid this process did not spawn.** A supervisor that restarted
//!    is checking worker pids it read back off disk, possibly written by
//!    a previous instance of itself that has since died.
//!    `Child::try_wait` needs an owned handle and is therefore useless
//!    here.
//! 2. **Spawn-time behaviour with no builder method** -- real detachment.
//! 3. **Platform entropy**, for session ids.
//!
//! Ported directly from `rusty_prime_agent`'s `src/procutil.rs`, which
//! solved this exact problem for the same reasons on the same platforms;
//! PLAN.md names it as a pattern to reuse rather than reinvent. The
//! reasoning preserved in the comments below is that project's, verified
//! against its source rather than restated from memory.
//!
//! # No Job Objects, deliberately
//!
//! `rustils` has a working Windows Job Object implementation, and
//! SCOPE.md originally called for using it on every spawned child. This
//! project does not, because a Job Object is a **kill-on-close** process
//! group: when the handle closes, everything in the job dies. That is
//! structurally incompatible with "sessions survive the manager app
//! closing", which is the single capability this architecture exists to
//! deliver. `rusty_prime_agent` reached the same conclusion independently
//! and documented it in its own `ARCHITECTURE.md`.
//!
//! Note also that `rustils`' Windows Job Object code is a lifecycle
//! primitive, **not a security sandbox** -- its `platform::security::
//! Sandbox` Windows implementation returns `Unsupported` from every
//! method. Nothing in this project should ever be described as
//! "sandboxed" on Windows.

use std::io;

use sessionmgr_core::ports::ProcessPort;

/// Marks `cmd` to survive this process exiting -- including crashing --
/// and its terminal closing.
///
/// - **Windows**: `CREATE_NEW_PROCESS_GROUP | DETACHED_PROCESS`, applied
///   via `std::os::windows::process::CommandExt::creation_flags` through
///   `rusty_tokio::process::Command::as_std_mut`'s escape hatch.
/// - **Unix**: `process_group(0)` alone would put the child in a fresh
///   *group* but leave it in the same *session*, still reachable by a
///   `SIGHUP` this process's controlling terminal delivers session-wide
///   on hangup. A `pre_exec` hook calling `setsid()` gets real
///   session-leader detachment.
pub fn prepare_detached(cmd: &mut rusty_tokio::process::Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
        const DETACHED_PROCESS: u32 = 0x0000_0008;
        cmd.as_std_mut()
            .creation_flags(CREATE_NEW_PROCESS_GROUP | DETACHED_PROCESS);
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        // SAFETY: the closure calls only `libc::setsid()` -- one
        // async-signal-safe POSIX call -- and always returns `Ok`. It
        // never panics, allocates, or touches the parent's memory: the
        // exact restricted-operation contract `pre_exec`'s own safety
        // documentation requires for the post-fork, pre-exec window.
        unsafe {
            cmd.as_std_mut().pre_exec(|| {
                libc::setsid();
                Ok(())
            });
        }
    }
}

/// Strips this process's own inherited stdin/stdout/stderr of their
/// "inheritable by a child" property. **Must run before this process
/// spawns anything.**
///
/// Without it, a `Stdio::null()`-everywhere detached spawn still leaks
/// this process's own stdio into the child:
///
/// - **Windows**: an explicit-handle spawn still passes
///   `bInheritHandles = TRUE`, which duplicates *every* currently
///   inheritable handle into the child, not just the three named in
///   `STARTUPINFO`. If this process's stdout is itself an inherited pipe
///   -- exactly what happens when a test harness captures output -- that
///   pipe handle rides along uninvited.
/// - **Unix**: `fork`+`exec` inherits every fd without `FD_CLOEXEC`,
///   independent of the child's own fd 0/1/2 configuration.
///
/// Either way a detached child that outlives this process holds the pipe
/// open forever, so a parent reading this process's output until EOF
/// (`std::process::Command::output`, which is exactly what this project's
/// black-box tests do) blocks indefinitely -- not because this process is
/// still running, but because something it spawned is. This hung
/// `rusty_prime_agent`'s own integration tests before it added the
/// equivalent function.
pub fn harden_inherited_stdio() {
    #[cfg(windows)]
    {
        use windows_sys::Win32::Foundation::{SetHandleInformation, HANDLE_FLAG_INHERIT};
        use windows_sys::Win32::System::Console::{
            GetStdHandle, STD_ERROR_HANDLE, STD_INPUT_HANDLE, STD_OUTPUT_HANDLE,
        };
        for which in [STD_INPUT_HANDLE, STD_OUTPUT_HANDLE, STD_ERROR_HANDLE] {
            // SAFETY: `GetStdHandle` with one of the three documented
            // constants always returns a checkable handle value; no
            // ownership is taken (nothing is closed or duplicated), only
            // the inherit flag is cleared. `SetHandleInformation` fails
            // harmlessly on a null/invalid handle, so calling it
            // unconditionally and ignoring the result is simpler than
            // special-casing and no less sound.
            unsafe {
                let handle = GetStdHandle(which);
                SetHandleInformation(handle, HANDLE_FLAG_INHERIT, 0);
            }
        }
    }
    #[cfg(unix)]
    {
        for fd in [0, 1, 2] {
            // SAFETY: `fcntl(fd, F_SETFD, FD_CLOEXEC)` on a standard,
            // always-valid fd number is a plain POSIX call. Failure is
            // ignored: this is best-effort hygiene, not a correctness
            // requirement of this process's own behaviour.
            unsafe {
                libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC);
            }
        }
    }
}

/// One whitespace-separated field of `/proc/<pid>/stat`, indexed from the
/// field *after* the executable-name field (so index 0 is field 3,
/// `state`).
///
/// Split after the **last** `)` rather than by whitespace from the start:
/// field 2 is the executable name in parentheses and may itself contain
/// spaces and parentheses, which is the classic way a naive
/// `split_whitespace().nth(n)` silently reads the wrong field.
#[cfg(target_os = "linux")]
fn proc_stat_field(pid: u32, index: usize) -> io::Result<Option<String>> {
    let stat = match std::fs::read_to_string(format!("/proc/{pid}/stat")) {
        Ok(s) => s,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e),
    };
    let Some(after_comm) = stat.rsplit_once(')').map(|(_, rest)| rest) else {
        return Ok(None);
    };
    Ok(after_comm.split_whitespace().nth(index).map(str::to_string))
}

/// Has `pid` already exited but not yet been reaped by its parent?
///
/// A zombie answers `kill(pid, 0)` successfully and is emphatically not
/// serving anything, so omitting this check makes a just-exited worker
/// read as healthy for the whole window before its parent reaps it --
/// which is exactly the window in which the supervisor is deciding
/// whether the session crashed.
///
/// A start-time fingerprint cannot substitute: a zombie has both the same
/// pid *and* the same start time as the process it is the remains of.
///
/// Unix-only, and gated rather than stubbed: Windows has no zombie
/// concept -- a terminated process's handle reports its exit code, which
/// [`is_alive`]'s own `GetExitCodeProcess` check already distinguishes.
#[cfg(unix)]
fn is_zombie(pid: u32) -> bool {
    #[cfg(target_os = "linux")]
    {
        matches!(proc_stat_field(pid, 0).ok().flatten().as_deref(), Some("Z"))
    }
    #[cfg(all(unix, not(target_os = "linux")))]
    {
        // `ps -o state=` reports `Z` for a zombie on macOS/BSD too; the
        // column can carry trailing flag characters (`Z+`), so check the
        // leading character rather than the whole field.
        std::process::Command::new("ps")
            .args(["-o", "state=", "-p", &pid.to_string()])
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().starts_with('Z'))
            .unwrap_or(false)
    }
}

/// Is `pid` currently alive? Works for any pid, including one recorded in
/// `state.json` long after the process that spawned it exited.
///
/// This answers "is *a* live process using this number", which is **not**
/// the same question as "is this still the process that recorded itself"
/// -- see [`is_same_process`].
pub fn is_alive(pid: u32) -> io::Result<bool> {
    #[cfg(unix)]
    {
        // SAFETY: `kill(pid, 0)` sends no signal; per POSIX it only
        // probes existence/permission, for any pid value.
        let ret = unsafe { libc::kill(pid as libc::pid_t, 0) };
        if ret == 0 {
            // Existence is not liveness -- see `is_zombie`.
            return Ok(!is_zombie(pid));
        }
        match io::Error::last_os_error().raw_os_error() {
            Some(libc::ESRCH) => Ok(false),
            // The pid exists, it just isn't signalable by this process.
            Some(libc::EPERM) => Ok(true),
            _ => Err(io::Error::last_os_error()),
        }
    }
    #[cfg(windows)]
    {
        use windows_sys::Win32::Foundation::CloseHandle;
        use windows_sys::Win32::System::Threading::{
            GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
        };
        const STILL_ACTIVE: u32 = 259;
        const ERROR_INVALID_PARAMETER: i32 = 87;
        // SAFETY: plain Win32 calls on a caller-supplied pid and a handle
        // this function itself opened; the handle is checked before use
        // and closed on every path that opened one.
        unsafe {
            let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
            if handle.is_null() {
                // "No such process" -> false; any other failure (e.g.
                // access denied on another user's pid) still means the
                // pid exists.
                return Ok(
                    io::Error::last_os_error().raw_os_error() != Some(ERROR_INVALID_PARAMETER)
                );
            }
            let mut exit_code: u32 = 0;
            let ok = GetExitCodeProcess(handle, &mut exit_code);
            CloseHandle(handle);
            if ok == 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(exit_code == STILL_ACTIVE)
        }
    }
}

/// An opaque, per-platform fingerprint of *when* `pid` started, used to
/// tell a still-running process apart from an unrelated one that has
/// since been handed the same pid number.
///
/// The value is opaque and only ever compared for equality with an
/// earlier reading of the same pid, so the differing per-platform formats
/// (clock ticks, a date string, a 64-bit `FILETIME`) never need
/// reconciling.
///
/// **Granularity, and why it suffices.** These clocks are coarse: Linux's
/// `starttime` counts clock ticks (typically 10ms), macOS's `ps -o
/// lstart=` resolves only to the second. Two processes started within the
/// same tick genuinely do share a fingerprint. That costs nothing here: a
/// reused pid necessarily arrives only after the pid space has wrapped
/// around, orders of magnitude longer than a second, so the interval
/// where two processes are indistinguishable never overlaps the interval
/// where pid reuse is possible.
pub fn start_fingerprint(pid: u32) -> io::Result<Option<String>> {
    #[cfg(target_os = "linux")]
    {
        // Field 22 (`starttime`) of `/proc/<pid>/stat`. `proc_stat_field`
        // begins at field 3, so field 22 is index 19 within it.
        proc_stat_field(pid, 19)
    }
    #[cfg(all(unix, not(target_os = "linux")))]
    {
        let out = std::process::Command::new("ps")
            .args(["-o", "lstart=", "-p", &pid.to_string()])
            .output()?;
        if !out.status.success() {
            return Ok(None);
        }
        let text = String::from_utf8_lossy(&out.stdout).trim().to_string();
        Ok(if text.is_empty() { None } else { Some(text) })
    }
    #[cfg(windows)]
    {
        use windows_sys::Win32::Foundation::{CloseHandle, FILETIME};
        use windows_sys::Win32::System::Threading::{
            GetProcessTimes, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
        };
        let zero = FILETIME {
            dwLowDateTime: 0,
            dwHighDateTime: 0,
        };
        let mut creation = zero;
        let mut ignored = [zero; 3];
        // SAFETY: plain Win32 calls on a caller-supplied pid; the handle
        // is checked before use and closed on every path that opened one,
        // and all four `FILETIME` out-params are live locals.
        unsafe {
            let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
            if handle.is_null() {
                return Ok(None);
            }
            let ok = GetProcessTimes(
                handle,
                &mut creation,
                &mut ignored[0],
                &mut ignored[1],
                &mut ignored[2],
            );
            CloseHandle(handle);
            if ok == 0 {
                return Ok(None);
            }
        }
        let ticks = ((creation.dwHighDateTime as u64) << 32) | (creation.dwLowDateTime as u64);
        Ok(Some(ticks.to_string()))
    }
}

/// Is `pid` alive *and* still the same process that recorded
/// `expected_fingerprint`?
///
/// The PID-reuse-safe replacement for a bare [`is_alive`] on a pid read
/// back off disk.
///
/// **Ambiguity resolves toward "alive", deliberately.** If
/// `expected_fingerprint` is `None`, or the current fingerprint cannot be
/// read on this platform, this reduces to [`is_alive`]. Being wrong in
/// that direction costs the narrow pid-reuse case this exists to catch.
/// Being wrong in the other direction would declare a *live* worker dead
/// -- and since this supervisor never respawns, that would mean reporting
/// a perfectly healthy running session as crashed and inviting the user
/// to throw it away.
pub fn is_same_process(pid: u32, expected_fingerprint: Option<&str>) -> io::Result<bool> {
    if !is_alive(pid)? {
        return Ok(false);
    }
    let Some(expected) = expected_fingerprint else {
        return Ok(true);
    };
    match start_fingerprint(pid) {
        Ok(Some(current)) => Ok(current == expected),
        // No reading available: no evidence of a mismatch is not evidence
        // of one. See the doc comment above.
        Ok(None) | Err(_) => Ok(true),
    }
}

/// Terminates an arbitrary pid this process did not itself spawn.
///
/// Unix: `SIGTERM` -- a cooperative shutdown request, not `SIGKILL`.
/// Windows has no signal-delivery equivalent for asking a process to shut
/// down cooperatively, so `TerminateProcess` is the only primitive
/// available there either way.
///
/// A pid that is already gone is **not** an error: the caller's goal is
/// "this is not running", which is already true.
pub fn terminate(pid: u32) -> io::Result<()> {
    #[cfg(unix)]
    {
        // SAFETY: `kill(pid, SIGTERM)` on a caller-supplied pid is a
        // plain, well-defined POSIX call.
        let ret = unsafe { libc::kill(pid as libc::pid_t, libc::SIGTERM) };
        if ret == 0 {
            return Ok(());
        }
        match io::Error::last_os_error().raw_os_error() {
            Some(libc::ESRCH) => Ok(()),
            _ => Err(io::Error::last_os_error()),
        }
    }
    #[cfg(windows)]
    {
        use windows_sys::Win32::Foundation::CloseHandle;
        use windows_sys::Win32::System::Threading::{
            OpenProcess, TerminateProcess, PROCESS_TERMINATE,
        };
        const ERROR_INVALID_PARAMETER: i32 = 87;
        // SAFETY: plain Win32 calls on a caller-supplied pid; the handle
        // is checked before use and closed on every path that opened one.
        unsafe {
            let handle = OpenProcess(PROCESS_TERMINATE, 0, pid);
            if handle.is_null() {
                let err = io::Error::last_os_error();
                // Already gone -- the desired end state.
                if err.raw_os_error() == Some(ERROR_INVALID_PARAMETER) {
                    return Ok(());
                }
                return Err(err);
            }
            let ok = TerminateProcess(handle, 1);
            let err = io::Error::last_os_error();
            CloseHandle(handle);
            if ok != 0 {
                Ok(())
            } else {
                Err(err)
            }
        }
    }
}

/// `len` bytes from the OS CSPRNG.
///
/// The platform directly rather than a `rand`-shaped dependency, matching
/// the sibling projects' posture: `/dev/urandom` needs no dependency at
/// all on Unix, and `BCryptGenRandom` is one call on Windows.
///
/// Returns `Err` rather than silently falling back to a weak source. The
/// caller ([`session_id`]) decides what to do about it, and does so
/// visibly.
pub fn os_random(buf: &mut [u8]) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::io::Read;
        let mut f = std::fs::File::open("/dev/urandom")?;
        f.read_exact(buf)
    }
    #[cfg(windows)]
    {
        use windows_sys::Win32::Security::Cryptography::{
            BCryptGenRandom, BCRYPT_USE_SYSTEM_PREFERRED_RNG,
        };
        // SAFETY: `BCryptGenRandom` with a null algorithm handle and
        // `BCRYPT_USE_SYSTEM_PREFERRED_RNG` writes exactly `buf.len()`
        // bytes into the caller's live, mutable buffer.
        let status = unsafe {
            BCryptGenRandom(
                std::ptr::null_mut(),
                buf.as_mut_ptr(),
                buf.len() as u32,
                BCRYPT_USE_SYSTEM_PREFERRED_RNG,
            )
        };
        if status == 0 {
            Ok(())
        } else {
            Err(io::Error::other(format!(
                "BCryptGenRandom failed with NTSTATUS 0x{status:08x}"
            )))
        }
    }
}

/// Milliseconds since the Unix epoch.
///
/// A clock before the epoch yields 0 rather than an error: the value
/// feeds a session id's ordering prefix, and a machine with a badly wrong
/// clock should still be able to create a session.
pub fn now_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Mints a fresh session id from the real clock and the OS CSPRNG --
/// the adapter half of `sessionmgr_core::SessionId::new`, which takes
/// both as arguments because the domain crate does no I/O.
pub fn session_id() -> io::Result<sessionmgr_core::SessionId> {
    let mut bytes = [0u8; 4];
    os_random(&mut bytes)?;
    Ok(sessionmgr_core::SessionId::new(
        now_millis(),
        u32::from_le_bytes(bytes),
    ))
}

/// The real implementation of [`ProcessPort`].
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemProcessPort;

impl ProcessPort for SystemProcessPort {
    /// A failed probe reports **not** the same process.
    ///
    /// The inner `is_same_process` already resolves its own ambiguity
    /// toward "alive"; an `Err` here means the probe itself failed to
    /// execute, which is a different thing and rare enough that treating
    /// it as "cannot confirm" is the honest reading.
    fn is_same_process(&self, pid: u32, expected: Option<&str>) -> bool {
        is_same_process(pid, expected).unwrap_or(false)
    }

    fn start_fingerprint(&self, pid: u32) -> Option<String> {
        start_fingerprint(pid).ok().flatten()
    }

    fn terminate(&self, pid: u32) -> io::Result<()> {
        terminate(pid)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn this_process_is_alive_and_fingerprints_stably() {
        let me = std::process::id();
        assert!(is_alive(me).expect("liveness probe must not error"));
        let a = start_fingerprint(me).expect("fingerprint call must not error");
        assert!(
            a.is_some(),
            "every platform this project targets should fingerprint its own pid"
        );
        let b = start_fingerprint(me).expect("fingerprint call must not error");
        assert_eq!(a, b, "a process's start time must not change between reads");
    }

    #[test]
    fn is_same_process_rejects_a_mismatched_fingerprint() {
        let me = std::process::id();
        let real = start_fingerprint(me).expect("fingerprint");
        assert!(is_same_process(me, real.as_deref()).expect("probe"));

        // Alive, but recorded by something else: the reused-pid case this
        // exists to catch.
        assert!(
            !is_same_process(me, Some("definitely-not-this-processes-start-time")).expect("probe"),
            "a live pid whose start time does not match its recording must not read as the same process"
        );

        // No recording at all: falls back to bare liveness rather than
        // declaring a live process dead.
        assert!(is_same_process(me, None).expect("probe"));
    }

    #[test]
    fn an_unreaped_child_does_not_read_as_alive() {
        // The zombie case. A child that has exited but whose parent (this
        // test) has not yet waited on it still answers `kill(pid, 0)`
        // successfully. It is serving nothing and must not read as alive,
        // or the supervisor's crash detection never fires.
        let mut child = std::process::Command::new(if cfg!(windows) { "cmd" } else { "true" })
            .args(if cfg!(windows) {
                vec!["/C", "exit"]
            } else {
                vec![]
            })
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .expect("spawn a child that exits immediately");
        let pid = child.id();

        // Deliberately no `try_wait`/`wait` before the assertion: either
        // would reap the child and turn this into the easy already-gone
        // case rather than the zombie one.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while is_alive(pid).expect("probe") && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert!(
            !is_alive(pid).expect("probe"),
            "an exited-but-unreaped process must not read as alive"
        );

        child.wait().expect("reap the child");
    }

    #[test]
    fn terminating_an_already_dead_pid_is_not_an_error() {
        let mut child = std::process::Command::new(if cfg!(windows) { "cmd" } else { "true" })
            .args(if cfg!(windows) {
                vec!["/C", "exit"]
            } else {
                vec![]
            })
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .expect("spawn");
        let pid = child.id();
        child.wait().expect("reap");
        // The goal is "not running", which is already true. Teardown runs
        // over a recorded pid list and must not fail because one entry
        // died on its own first.
        assert!(terminate(pid).is_ok());
    }

    #[test]
    fn os_random_fills_the_buffer_and_varies() {
        let mut a = [0u8; 16];
        let mut b = [0u8; 16];
        os_random(&mut a).expect("entropy");
        os_random(&mut b).expect("entropy");
        assert_ne!(a, b, "two CSPRNG reads must not be identical");
        assert!(a.iter().any(|byte| *byte != 0), "buffer left untouched");
    }

    #[test]
    fn minted_session_ids_are_valid_and_unique() {
        let a = session_id().expect("mint");
        let b = session_id().expect("mint");
        assert_ne!(a, b);
        assert!(a.as_str().parse::<sessionmgr_core::SessionId>().is_ok());
    }

    #[test]
    fn the_port_implementation_agrees_with_the_free_functions() {
        let port = SystemProcessPort;
        let me = std::process::id();
        let fp = port.start_fingerprint(me);
        assert!(port.is_same_process(me, fp.as_deref()));
        assert!(!port.is_same_process(me, Some("not-the-real-fingerprint")));
    }

    #[test]
    fn a_detached_child_still_spawns_and_runs() {
        // Not a test of *detachment* -- that needs a process tree
        // outliving its parent, which is what the black-box
        // `supervisor_restart_recovery` test proves. This only asserts
        // the flags/`pre_exec` hook don't break an ordinary spawn, which
        // is the failure mode a typo here would produce.
        let rt = rusty_tokio::Runtime::new().expect("runtime");
        rt.block_on(async {
            let mut cmd = rusty_tokio::process::Command::new(if cfg!(windows) {
                "cmd"
            } else {
                "true"
            });
            if cfg!(windows) {
                cmd.arg("/C").arg("exit");
            }
            cmd.stdin(rusty_tokio::process::Stdio::null())
                .stdout(rusty_tokio::process::Stdio::null())
                .stderr(rusty_tokio::process::Stdio::null());
            prepare_detached(&mut cmd);
            let status = cmd.status().await.expect("spawn a detached child");
            assert!(status.success());
        });
    }
}
