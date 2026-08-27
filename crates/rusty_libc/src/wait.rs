//! `waitpid` (via `wait4`) and the `W*` status-decoding helpers.
//!
//! The status helpers are pure bit tests over the kernel's wait-status
//! encoding; they take the raw `i32` status filled in by [`waitpid`].

use crate::arch::nr;
use crate::arch::{from_ret, from_ret_i32, syscall2, syscall4, syscall5, Errno};

/// `waitpid` option: return immediately if no child has changed state.
pub const WNOHANG: i32 = 1;
/// `waitpid` option: also report stopped children.
pub const WUNTRACED: i32 = 2;
/// `waitpid` option: also report continued children.
pub const WCONTINUED: i32 = 8;

/// Wait for a state change in a child process.
///
/// Returns `(pid, status)` where `pid` is the child that changed state (`0`
/// when [`WNOHANG`] is set and no child is ready) and `status` is the raw
/// value to pass to the `w*` helpers below.
pub fn waitpid(pid: i32, options: i32) -> Result<(i32, i32), Errno> {
    let mut status: i32 = 0;
    // wait4(pid, &mut status, options, rusage = NULL).
    // SAFETY: `status` is a valid `*mut i32`; `rusage` is null.
    let ret = unsafe {
        syscall4(
            nr::WAIT4,
            pid as usize,
            &mut status as *mut i32 as usize,
            options as usize,
            0,
        )
    };
    let child = from_ret_i32(ret)?;
    Ok((child, status))
}

/// A `struct timeval`-shaped time value (seconds + microseconds), as used by
/// [`Rusage`].
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Timeval {
    /// Whole seconds.
    pub tv_sec: i64,
    /// Microseconds within the second.
    pub tv_usec: i64,
}

/// Resource usage accounting (kernel/glibc-ABI-compatible `struct rusage`,
/// 144 bytes) for a terminated child, as filled in by [`waitpid_rusage`]/
/// [`waitid_rusage`].
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Rusage {
    /// User CPU time consumed.
    pub utime: Timeval,
    /// System CPU time consumed.
    pub stime: Timeval,
    /// Maximum resident set size, in kilobytes.
    pub maxrss: i64,
    /// Integral shared memory size (unused on Linux; always 0).
    pub ixrss: i64,
    /// Integral unshared data size (unused on Linux; always 0).
    pub idrss: i64,
    /// Integral unshared stack size (unused on Linux; always 0).
    pub isrss: i64,
    /// Soft page faults (reclaimed without I/O).
    pub minflt: i64,
    /// Hard page faults (required I/O).
    pub majflt: i64,
    /// Swaps (unused on Linux; always 0).
    pub nswap: i64,
    /// Block input operations.
    pub inblock: i64,
    /// Block output operations.
    pub oublock: i64,
    /// IPC messages sent (unused on Linux; always 0).
    pub msgsnd: i64,
    /// IPC messages received (unused on Linux; always 0).
    pub msgrcv: i64,
    /// Signals received (unused on Linux; always 0).
    pub nsignals: i64,
    /// Voluntary context switches.
    pub nvcsw: i64,
    /// Involuntary context switches.
    pub nivcsw: i64,
}

const _: () = assert!(core::mem::size_of::<Timeval>() == 16);
const _: () = assert!(core::mem::size_of::<Rusage>() == 144);
const _: () = assert!(core::mem::offset_of!(Rusage, stime) == 16);
const _: () = assert!(core::mem::offset_of!(Rusage, maxrss) == 32);
const _: () = assert!(core::mem::offset_of!(Rusage, nivcsw) == 136);

/// [`getrusage`] `who`: the calling process (or, `waitpid_rusage`/
/// `waitid_rusage`-style, thread group).
pub const RUSAGE_SELF: i32 = 0;
/// [`getrusage`] `who`: all children this process has already waited for
/// and reaped, summed together (not the currently-running ones — this
/// crate has no `RUSAGE_BOTH`/`RUSAGE_THREAD` support since the kernel
/// gates them behind privileges/thread-specific semantics this crate's
/// job-control-shell use case doesn't need).
pub const RUSAGE_CHILDREN: i32 = -1;

/// Get the resource usage (CPU time, page faults, context switches, …) of
/// the calling process itself, per [`RUSAGE_SELF`]/[`RUSAGE_CHILDREN`].
///
/// [`waitpid_rusage`]/[`waitid_rusage`] expose this same data, but only
/// for one specific already-reaped *child* — there was no way to ask for
/// the calling process's *own* accumulated usage, which a `time` builtin
/// needs when timing the shell's own work (e.g. a pipeline with
/// shell-internal stages) rather than only an external command's child
/// process.
pub fn getrusage(who: i32) -> Result<Rusage, Errno> {
    let mut rusage = Rusage::default();
    // getrusage(who, &mut rusage).
    // SAFETY: `rusage` is a valid, exclusively-borrowed out parameter the
    // kernel writes completely on success.
    let ret = unsafe {
        syscall2(
            nr::GETRUSAGE,
            who as usize,
            &mut rusage as *mut Rusage as usize,
        )
    };
    from_ret(ret)?;
    Ok(rusage)
}

/// Like [`waitpid`], but also returns the terminated child's resource usage
/// (CPU time, page faults, context switches, …) -- the data a `time` builtin
/// (`real`/`user`/`sys`) needs, which the kernel already computes for free
/// but [`waitpid`] discards.
pub fn waitpid_rusage(pid: i32, options: i32) -> Result<(i32, i32, Rusage), Errno> {
    let mut status: i32 = 0;
    let mut rusage = Rusage::default();
    // wait4(pid, &mut status, options, &mut rusage).
    // SAFETY: `status`/`rusage` are valid, exclusively-borrowed out
    // parameters the kernel writes.
    let ret = unsafe {
        syscall4(
            nr::WAIT4,
            pid as usize,
            &mut status as *mut i32 as usize,
            options as usize,
            &mut rusage as *mut Rusage as usize,
        )
    };
    let child = from_ret_i32(ret)?;
    Ok((child, status, rusage))
}

/// True if the child terminated normally (via `exit`/`return`).
#[inline]
pub fn wifexited(status: i32) -> bool {
    (status & 0x7f) == 0
}

/// Exit code of a normally-terminated child (valid when [`wifexited`]).
#[inline]
pub fn wexitstatus(status: i32) -> i32 {
    (status >> 8) & 0xff
}

/// True if the child was terminated by a signal.
#[inline]
pub fn wifsignaled(status: i32) -> bool {
    // Neither "exited" (low 7 bits == 0) nor "stopped" (low 7 bits == 0x7f).
    let sig = status & 0x7f;
    sig != 0 && sig != 0x7f
}

/// Signal that terminated the child (valid when [`wifsignaled`]).
#[inline]
pub fn wtermsig(status: i32) -> i32 {
    status & 0x7f
}

/// True if the child is currently stopped.
#[inline]
pub fn wifstopped(status: i32) -> bool {
    (status & 0xff) == 0x7f
}

/// Signal that stopped the child (valid when [`wifstopped`]).
#[inline]
pub fn wstopsig(status: i32) -> i32 {
    (status >> 8) & 0xff
}

/// True if the child was resumed by `SIGCONT`.
#[inline]
pub fn wifcontinued(status: i32) -> bool {
    status == 0xffff
}

// --- waitid ------------------------------------------------------------------

/// [`waitid`] `idtype`: wait for any child (the `id` argument is ignored).
pub const P_ALL: i32 = 0;
/// [`waitid`] `idtype`: wait for the child whose pid equals `id`.
pub const P_PID: i32 = 1;
/// [`waitid`] `idtype`: wait for any child in the process group `id`.
pub const P_PGID: i32 = 2;
/// [`waitid`] `idtype`: wait for the child referred to by the pidfd `id`
/// (Track P — the `crate::process::pidfd_open` companion). Requires
/// [`WEXITED`], and only for a fd `pidfd_open` actually returned, not any
/// arbitrary open fd.
pub const P_PIDFD: i32 = 3;

/// [`waitid`] option: report children that have terminated. At least one of
/// `WEXITED`/[`WSTOPPED`]/[`WCONTINUED`] must be set.
pub const WEXITED: i32 = 4;
/// [`waitid`] option: report children that have stopped (same bit as
/// [`WUNTRACED`]).
pub const WSTOPPED: i32 = 2;
/// [`waitid`] option: leave the child in a waitable state — report its status
/// **without** reaping it, so a later `waitpid`/`waitid` still sees it. The key
/// flag for peeking at a job table.
pub const WNOWAIT: i32 = 0x0100_0000;

/// `si_code` for a child that exited normally (via `exit`/`return`).
pub const CLD_EXITED: i32 = 1;
/// `si_code` for a child killed by a signal.
pub const CLD_KILLED: i32 = 2;
/// `si_code` for a child killed by a signal that dumped core.
pub const CLD_DUMPED: i32 = 3;
/// `si_code` for a traced child that trapped.
pub const CLD_TRAPPED: i32 = 4;
/// `si_code` for a child that stopped.
pub const CLD_STOPPED: i32 = 5;
/// `si_code` for a child that continued.
pub const CLD_CONTINUED: i32 = 6;

/// The subset of the kernel `siginfo_t` (128 bytes) that [`waitid`] fills in.
///
/// The trailing bytes are the rest of the kernel structure; only the named
/// fields are meaningful after `waitid`. `si_status` holds the child's exit
/// code when [`si_code`](Self::si_code) is [`CLD_EXITED`], otherwise the
/// signal number.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Siginfo {
    /// Signal number (`SIGCHLD` for `waitid` results).
    pub si_signo: i32,
    /// Error number (0 for `waitid`).
    pub si_errno: i32,
    /// Reason code: a `CLD_*` value describing the child's state change.
    pub si_code: i32,
    __pad0: i32,
    /// PID of the child the event is about.
    pub si_pid: i32,
    /// Real user ID of the child.
    pub si_uid: u32,
    /// Exit code (with [`CLD_EXITED`]) or terminating/stopping signal.
    pub si_status: i32,
    __pad: [u8; 100],
}

const _: () = assert!(core::mem::size_of::<Siginfo>() == 128);
const _: () = assert!(core::mem::offset_of!(Siginfo, si_code) == 8);
const _: () = assert!(core::mem::offset_of!(Siginfo, si_pid) == 16);
const _: () = assert!(core::mem::offset_of!(Siginfo, si_status) == 24);

impl Default for Siginfo {
    fn default() -> Self {
        // All-zero is a valid "no child ready" siginfo (si_pid == 0).
        Siginfo {
            si_signo: 0,
            si_errno: 0,
            si_code: 0,
            __pad0: 0,
            si_pid: 0,
            si_uid: 0,
            si_status: 0,
            __pad: [0; 100],
        }
    }
}

/// Wait for a child state change and report it through a [`Siginfo`], with
/// finer control than [`waitpid`].
///
/// `idtype`/`id` select which children ([`P_ALL`], [`P_PID`], [`P_PGID`]);
/// `options` is an OR of [`WEXITED`]/[`WSTOPPED`]/[`WCONTINUED`] (at least one
/// required) plus optionally [`WNOHANG`] and [`WNOWAIT`]. With [`WNOWAIT`] the
/// child is **not** reaped, so a job controller can peek at its status and
/// still `waitpid` it later. With [`WNOHANG`] and no ready child, the returned
/// `Siginfo` has `si_pid == 0`.
pub fn waitid(idtype: i32, id: i32, options: i32) -> Result<Siginfo, Errno> {
    let mut info = Siginfo::default();
    // waitid(idtype, id, &mut info, options, rusage = NULL).
    // SAFETY: `info` is a valid, exclusively-borrowed 128-byte `siginfo_t` the
    // kernel writes; `rusage` is null.
    let ret = unsafe {
        syscall5(
            nr::WAITID,
            idtype as usize,
            id as usize,
            &mut info as *mut Siginfo as usize,
            options as usize,
            0,
        )
    };
    from_ret(ret)?;
    Ok(info)
}

/// Like [`waitid`], but also returns the child's resource usage. See
/// [`waitpid_rusage`] for why this exists as a sibling rather than changing
/// [`waitid`]'s signature.
pub fn waitid_rusage(idtype: i32, id: i32, options: i32) -> Result<(Siginfo, Rusage), Errno> {
    let mut info = Siginfo::default();
    let mut rusage = Rusage::default();
    // waitid(idtype, id, &mut info, options, &mut rusage).
    // SAFETY: `info`/`rusage` are valid, exclusively-borrowed out parameters
    // the kernel writes.
    let ret = unsafe {
        syscall5(
            nr::WAITID,
            idtype as usize,
            id as usize,
            &mut info as *mut Siginfo as usize,
            options as usize,
            &mut rusage as *mut Rusage as usize,
        )
    };
    from_ret(ret)?;
    Ok((info, rusage))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_normal_exit() {
        // Child exited with code 42: status == 42 << 8.
        let status = 42 << 8;
        assert!(wifexited(status));
        assert_eq!(wexitstatus(status), 42);
        assert!(!wifsignaled(status));
        assert!(!wifstopped(status));
    }

    #[test]
    fn decodes_signal_death() {
        // Killed by SIGKILL (9), no core dump: low 7 bits == 9.
        let status = 9;
        assert!(wifsignaled(status));
        assert_eq!(wtermsig(status), 9);
        assert!(!wifexited(status));
        assert!(!wifstopped(status));
    }

    #[test]
    fn decodes_stop_and_continue() {
        // Stopped by SIGSTOP (19): (19 << 8) | 0x7f.
        let status = (19 << 8) | 0x7f;
        assert!(wifstopped(status));
        assert_eq!(wstopsig(status), 19);
        assert!(!wifsignaled(status));

        assert!(wifcontinued(0xffff));
    }

    #[test]
    fn wnohang_no_child_is_einval_or_echild() {
        // No children exist: waitpid(-1, WNOHANG) fails with ECHILD (10).
        assert_eq!(waitpid(-1, WNOHANG), Err(Errno(10)));
    }

    #[test]
    fn waitid_peeks_with_wnowait_then_reaps() {
        use crate::process::{exit_group, fork};

        // CI runs single-threaded, and the child only issues raw syscalls; see
        // `process::fork`'s safety note.
        match unsafe { fork() }.expect("fork") {
            0 => exit_group(7),
            pid => {
                // Peek at the exit status without reaping (WNOWAIT).
                let info = waitid(P_PID, pid, WEXITED | WNOWAIT).expect("waitid peek");
                assert_eq!(info.si_pid, pid);
                assert_eq!(info.si_code, CLD_EXITED);
                assert_eq!(info.si_status, 7);

                // Because WNOWAIT left it waitable, waitpid still reaps it.
                let (wpid, status) = waitpid(pid, 0).expect("waitpid");
                assert_eq!(wpid, pid);
                assert!(wifexited(status));
                assert_eq!(wexitstatus(status), 7);

                // Now it is gone.
                assert_eq!(waitpid(pid, 0), Err(Errno::ECHILD));
            }
        }
    }

    #[test]
    fn waitid_no_child_is_echild() {
        assert_eq!(waitid(P_ALL, 0, WEXITED | WNOHANG), Err(Errno::ECHILD));
    }

    #[test]
    fn getrusage_self_reports_own_cpu_time() {
        // Burn measurable CPU in this thread/process itself, then confirm
        // RUSAGE_SELF reports it -- unlike waitpid_rusage/waitid_rusage,
        // this needs no child at all.
        let mut x: u64 = 0;
        for i in 0..50_000_000u64 {
            x = core::hint::black_box(x.wrapping_add(i));
        }
        core::hint::black_box(x);

        let rusage = getrusage(RUSAGE_SELF).expect("getrusage");
        assert!(
            rusage.utime.tv_sec > 0 || rusage.utime.tv_usec > 0,
            "expected non-zero self CPU time accounting, got {rusage:?}"
        );
    }

    #[test]
    fn getrusage_children_reports_reaped_child_cpu_time() {
        use crate::process::{exit_group, fork};

        // RUSAGE_CHILDREN accumulates process-wide across every reaped
        // child, including ones from other tests in this same binary, so
        // this only asserts it becomes (further) non-zero after reaping a
        // CPU-burning child -- not that it started at zero, which would be
        // order-dependent on whatever else this process has already
        // reaped.
        match unsafe { fork() }.expect("fork") {
            0 => {
                let mut x: u64 = 0;
                for i in 0..50_000_000u64 {
                    x = core::hint::black_box(x.wrapping_add(i));
                }
                exit_group(0);
            }
            pid => {
                let (wpid, status) = waitpid(pid, 0).expect("waitpid");
                assert_eq!(wpid, pid);
                assert!(wifexited(status));

                let rusage = getrusage(RUSAGE_CHILDREN).expect("getrusage");
                assert!(
                    rusage.utime.tv_sec > 0
                        || rusage.utime.tv_usec > 0
                        || rusage.stime.tv_sec > 0
                        || rusage.stime.tv_usec > 0,
                    "expected non-zero cumulative child CPU time, got {rusage:?}"
                );
            }
        }
    }

    #[test]
    fn getrusage_bad_who_is_einval() {
        assert_eq!(getrusage(9999), Err(Errno::EINVAL));
    }

    #[test]
    fn waitpid_rusage_reports_child_cpu_time() {
        use crate::process::{exit_group, fork};

        // CI runs single-threaded, and the child only issues raw syscalls;
        // see `process::fork`'s safety note.
        match unsafe { fork() }.expect("fork") {
            0 => {
                // Burn measurable CPU so utime/stime aren't just noise.
                let mut x: u64 = 0;
                for i in 0..50_000_000u64 {
                    x = core::hint::black_box(x.wrapping_add(i));
                }
                exit_group(0);
            }
            pid => {
                let (wpid, status, rusage) = waitpid_rusage(pid, 0).expect("waitpid_rusage");
                assert_eq!(wpid, pid);
                assert!(wifexited(status));
                assert_eq!(wexitstatus(status), 0);
                assert!(
                    rusage.utime.tv_sec > 0
                        || rusage.utime.tv_usec > 0
                        || rusage.stime.tv_sec > 0
                        || rusage.stime.tv_usec > 0,
                    "expected non-zero CPU time accounting, got {rusage:?}"
                );
            }
        }
    }

    #[test]
    fn waitid_rusage_reports_status_and_fills_rusage() {
        use crate::process::{exit_group, fork};

        match unsafe { fork() }.expect("fork") {
            0 => {
                // Only worth burning CPU where the rusage content is
                // actually checked below (x86_64) -- see that assertion's
                // comment.
                #[cfg(target_arch = "x86_64")]
                {
                    let mut x: u64 = 0;
                    for i in 0..50_000_000u64 {
                        x = core::hint::black_box(x.wrapping_add(i));
                    }
                }
                exit_group(5);
            }
            pid => {
                let (info, rusage) = waitid_rusage(P_PID, pid, WEXITED).expect("waitid_rusage");
                assert_eq!(info.si_pid, pid);
                assert_eq!(info.si_code, CLD_EXITED);
                assert_eq!(info.si_status, 5);
                // `rusage` is otherwise unused on non-x86_64: the assertion
                // below that reads it is gated to x86_64.
                #[cfg(not(target_arch = "x86_64"))]
                let _ = &rusage;

                // The rusage content check is gated to x86_64: under the
                // aarch64 qemu-user CI job, waitid's rusage out-parameter
                // consistently reads back all-zero even after real CPU
                // work (confirmed: the sibling wait4-based
                // `waitpid_rusage_reports_child_cpu_time` test, using the
                // same CPU-burning child, passes on both arches) -- a gap
                // in qemu-user's syscall translation for waitid
                // specifically, not this crate's argument construction
                // (identical code path on both arches) or a real aarch64
                // kernel limitation. Same reasoning as
                // `vdso_resolves_on_native_x86_64`.
                #[cfg(target_arch = "x86_64")]
                assert!(
                    rusage.utime.tv_sec > 0
                        || rusage.utime.tv_usec > 0
                        || rusage.stime.tv_sec > 0
                        || rusage.stime.tv_usec > 0,
                    "expected non-zero CPU time accounting, got {rusage:?}"
                );
            }
        }
    }

    #[test]
    fn waitid_via_pidfd_reaps_the_child() {
        use crate::fd;
        use crate::process::{exit_group, fork, pidfd_open};

        // CI runs single-threaded, and the child only issues raw syscalls;
        // see `process::fork`'s safety note.
        let pid = match unsafe { fork() }.expect("fork") {
            0 => exit_group(9),
            pid => pid,
        };
        let pidfd = pidfd_open(pid, 0).expect("pidfd_open");

        // Block on the pidfd becoming readable (the child exiting) instead
        // of polling with a timeout — this is the whole point of a pidfd
        // over a bare pid: a normal readiness primitive works on it.
        let mut fds = [fd::PollFd {
            fd: pidfd,
            events: fd::POLLIN,
            revents: 0,
        }];
        let n = fd::poll(&mut fds, 5000).expect("poll");
        assert_eq!(n, 1);

        let info = waitid(P_PIDFD, pidfd, WEXITED).expect("waitid via pidfd");
        assert_eq!(info.si_pid, pid);
        assert_eq!(info.si_code, CLD_EXITED);
        assert_eq!(info.si_status, 9);

        // Reaped: a second waitpid on the bare pid sees nothing left.
        assert_eq!(waitpid(pid, 0), Err(Errno::ECHILD));
        fd::close(pidfd).expect("close pidfd");
    }
}
