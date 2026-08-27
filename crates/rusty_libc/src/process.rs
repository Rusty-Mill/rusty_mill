//! Process identity, process groups, signalling, and exit, plus the raw
//! [`fork`] primitive (Phase 4).

use crate::arch::nr;
use crate::arch::{
    from_ret, from_ret_i32, syscall0, syscall1, syscall2, syscall3, syscall4, syscall5, syscall6,
    Errno,
};
use crate::fd::{IoSlice, IoSliceMut};
use core::ffi::{c_char, CStr};

/// Get the calling process's ID. Cannot fail.
#[inline]
pub fn getpid() -> i32 {
    // SAFETY: getpid takes no arguments and never fails.
    unsafe { syscall0(nr::GETPID) as i32 }
}

/// Get the parent process's ID. Cannot fail.
#[inline]
pub fn getppid() -> i32 {
    // SAFETY: getppid takes no arguments and never fails.
    unsafe { syscall0(nr::GETPPID) as i32 }
}

/// Get the calling thread's kernel thread id (`SYS_gettid`), distinct from
/// [`getpid`] (the thread-*group* id, shared by every thread in the
/// process) whenever the caller isn't the main thread. Cannot fail.
#[inline]
pub fn gettid() -> i32 {
    // SAFETY: gettid takes no arguments and never fails.
    unsafe { syscall0(nr::GETTID) as i32 }
}

/// Get the calling process's real user ID. Cannot fail.
#[inline]
pub fn getuid() -> u32 {
    // SAFETY: getuid takes no arguments and never fails.
    unsafe { syscall0(nr::GETUID) as u32 }
}

/// Get the calling process's effective user ID. Cannot fail.
///
/// This is the id the kernel checks for permissions; a shell compares it to `0`
/// to decide the root prompt (`#` vs `$`).
#[inline]
pub fn geteuid() -> u32 {
    // SAFETY: geteuid takes no arguments and never fails.
    unsafe { syscall0(nr::GETEUID) as u32 }
}

/// Get the calling process's real group ID. Cannot fail.
#[inline]
pub fn getgid() -> u32 {
    // SAFETY: getgid takes no arguments and never fails.
    unsafe { syscall0(nr::GETGID) as u32 }
}

/// Get the calling process's effective group ID. Cannot fail.
#[inline]
pub fn getegid() -> u32 {
    // SAFETY: getegid takes no arguments and never fails.
    unsafe { syscall0(nr::GETEGID) as u32 }
}

/// Get the number of supplementary group IDs the calling process belongs to,
/// without fetching them. Convenience for sizing the buffer passed to
/// [`getgroups`] (`getgroups(2)` with `size == 0` leaves the buffer
/// untouched and just returns the count).
pub fn ngroups() -> Result<usize, Errno> {
    // SAFETY: a null pointer is valid here precisely because size == 0 means
    // the kernel never dereferences it.
    let ret = unsafe { syscall2(nr::GETGROUPS, 0, 0) };
    from_ret(ret)
}

/// Fill `buf` with the calling process's supplementary group IDs, returning
/// the filled prefix. Fails with `EINVAL` if `buf` is smaller than the
/// process's actual group count -- call [`ngroups`] first to size it, the
/// same bring-your-own-buffer convention as [`crate::fd::read`].
pub fn getgroups(buf: &mut [u32]) -> Result<&[u32], Errno> {
    // SAFETY: `buf` is a valid, exclusively-borrowed slice of `buf.len()`
    // `u32`s (matching the kernel's `gid_t`); the kernel writes at most that
    // many entries.
    let ret = unsafe { syscall2(nr::GETGROUPS, buf.len(), buf.as_mut_ptr() as usize) };
    let n = from_ret(ret)?;
    Ok(&buf[..n])
}

// --- privilege: setuid/setgid family ----------------------------------------

/// Sentinel for [`setresuid`]/[`setresgid`]: leave that particular id
/// unchanged. Matches the kernel's own `(uid_t)-1`/`(gid_t)-1` convention.
pub const KEEP_ID: u32 = u32::MAX;

/// Set the calling process's real, effective, **and** saved user id to
/// `uid` in one call. Requires `CAP_SETUID` (or dropping privilege from
/// uid `0`); an unprivileged process may only set its ids to its current
/// real, effective, or saved uid.
pub fn setuid(uid: u32) -> Result<(), Errno> {
    // SAFETY: plain integer argument, no memory referenced.
    let ret = unsafe { syscall1(nr::SETUID, uid as usize) };
    from_ret(ret).map(|_| ())
}

/// Set the calling process's real, effective, **and** saved group id to
/// `gid` in one call. Same privilege rule as [`setuid`], via `CAP_SETGID`.
pub fn setgid(gid: u32) -> Result<(), Errno> {
    // SAFETY: plain integer argument, no memory referenced.
    let ret = unsafe { syscall1(nr::SETGID, gid as usize) };
    from_ret(ret).map(|_| ())
}

/// Get the calling process's real, effective, and saved user ids in one
/// call, as `(ruid, euid, suid)`. The getter half of [`setresuid`] --
/// [`getuid`]/[`geteuid`] only expose two of the three; the saved id has no
/// dedicated getter of its own on Linux.
pub fn getresuid() -> Result<(u32, u32, u32), Errno> {
    let (mut ruid, mut euid, mut suid) = (0u32, 0u32, 0u32);
    // getresuid(&mut ruid, &mut euid, &mut suid).
    // SAFETY: all three out-params are valid, exclusively-borrowed `u32`s
    // the kernel writes.
    let ret = unsafe {
        syscall3(
            nr::GETRESUID,
            &mut ruid as *mut u32 as usize,
            &mut euid as *mut u32 as usize,
            &mut suid as *mut u32 as usize,
        )
    };
    from_ret(ret)?;
    Ok((ruid, euid, suid))
}

/// Get the calling process's real, effective, and saved group ids in one
/// call, as `(rgid, egid, sgid)`. The getter half of [`setresgid`], mirroring
/// [`getresuid`].
pub fn getresgid() -> Result<(u32, u32, u32), Errno> {
    let (mut rgid, mut egid, mut sgid) = (0u32, 0u32, 0u32);
    // getresgid(&mut rgid, &mut egid, &mut sgid).
    // SAFETY: all three out-params are valid, exclusively-borrowed `u32`s
    // the kernel writes.
    let ret = unsafe {
        syscall3(
            nr::GETRESGID,
            &mut rgid as *mut u32 as usize,
            &mut egid as *mut u32 as usize,
            &mut sgid as *mut u32 as usize,
        )
    };
    from_ret(ret)?;
    Ok((rgid, egid, sgid))
}

/// Independently set the real, effective, and saved user ids; pass
/// [`KEEP_ID`] for any of the three to leave it unchanged. The general
/// primitive [`setuid`]/[`seteuid`] are convenience shorthands over —
/// e.g. dropping privilege irrevocably (no way back to the old effective
/// id) needs all three set together, which `setuid` alone cannot express
/// on its own without also touching the saved id as a side effect.
pub fn setresuid(ruid: u32, euid: u32, suid: u32) -> Result<(), Errno> {
    // SAFETY: plain integer arguments, no memory referenced.
    let ret = unsafe { syscall3(nr::SETRESUID, ruid as usize, euid as usize, suid as usize) };
    from_ret(ret).map(|_| ())
}

/// Independently set the real, effective, and saved group ids; pass
/// [`KEEP_ID`] for any of the three to leave it unchanged. See
/// [`setresuid`] for why this exists alongside [`setgid`]/[`setegid`].
pub fn setresgid(rgid: u32, egid: u32, sgid: u32) -> Result<(), Errno> {
    // SAFETY: plain integer arguments, no memory referenced.
    let ret = unsafe { syscall3(nr::SETRESGID, rgid as usize, egid as usize, sgid as usize) };
    from_ret(ret).map(|_| ())
}

/// Set only the calling process's *effective* user id to `euid`, leaving
/// the real and saved ids untouched — the shorthand a shell wants to
/// temporarily assume then later relinquish a privilege (e.g. a setuid
/// helper that needs elevated access for one operation). There is no
/// dedicated `seteuid(2)` syscall on Linux; this is
/// `setresuid(KEEP_ID, euid, KEEP_ID)`, the same substitution glibc's own
/// `seteuid` makes.
#[inline]
pub fn seteuid(euid: u32) -> Result<(), Errno> {
    setresuid(KEEP_ID, euid, KEEP_ID)
}

/// Set only the calling process's *effective* group id to `egid`. See
/// [`seteuid`]; this is `setresgid(KEEP_ID, egid, KEEP_ID)`.
#[inline]
pub fn setegid(egid: u32) -> Result<(), Errno> {
    setresgid(KEEP_ID, egid, KEEP_ID)
}

/// Set the calling process's complete supplementary group list to `gids`
/// (replacing it, not appending). Requires `CAP_SETGID`. The counterpart
/// to [`getgroups`] — dropping supplementary groups (an empty slice) is
/// part of the standard privilege-dropping sequence alongside
/// [`setresgid`]/[`setresuid`] (groups must be dropped *before* the real
/// uid, while the process still has `CAP_SETGID`).
pub fn setgroups(gids: &[u32]) -> Result<(), Errno> {
    // SAFETY: `gids` is a valid slice of `gids.len()` `u32`s (matching the
    // kernel's `gid_t`), read-only for the kernel.
    let ret = unsafe { syscall2(nr::SETGROUPS, gids.len(), gids.as_ptr() as usize) };
    from_ret(ret).map(|_| ())
}

// --- scheduling priority ("nice") ------------------------------------------

/// [`getpriority`]/[`setpriority`] `which`: `who` is a pid (`0` = self).
pub const PRIO_PROCESS: i32 = 0;
/// [`getpriority`]/[`setpriority`] `which`: `who` is a process-group id (`0` =
/// the caller's own group).
pub const PRIO_PGRP: i32 = 1;
/// [`getpriority`]/[`setpriority`] `which`: `who` is a uid (`0` = the
/// caller's own real uid).
pub const PRIO_USER: i32 = 2;

/// Get the scheduling priority ("nice value", `-20`..`19`; lower runs more
/// favorably) of the process/process-group/user identified by `which`/`who`.
///
/// The raw `getpriority(2)` syscall actually returns `20 - nice` (always in
/// `1..=40`) specifically so it never collides with the `-errno` range that
/// negative nice values would otherwise fall into; this undoes that bias so
/// callers get the real nice value back directly.
pub fn getpriority(which: i32, who: i32) -> Result<i32, Errno> {
    // SAFETY: plain integer arguments, no memory referenced.
    let ret = unsafe { syscall2(nr::GETPRIORITY, which as usize, who as usize) };
    from_ret(ret).map(|raw| 20 - raw as i32)
}

/// Set the scheduling priority ("nice value") of the process/process-group/
/// user identified by `which`/`who` to `prio` (`-20`..`19`; out-of-range
/// values are clamped by the kernel). Raising priority (lowering `prio`
/// below the caller's current value) requires `CAP_SYS_NICE`; an
/// unprivileged caller gets `EPERM`/`EACCES`.
pub fn setpriority(which: i32, who: i32, prio: i32) -> Result<(), Errno> {
    // SAFETY: plain integer arguments, no memory referenced.
    let ret = unsafe { syscall3(nr::SETPRIORITY, which as usize, who as usize, prio as usize) };
    from_ret(ret).map(|_| ())
}

/// Adjust the calling process's own nice value by `inc` (positive lowers
/// priority, negative raises it and needs `CAP_SYS_NICE`), returning the
/// resulting nice value. The classic `nice(2)` convenience over
/// [`getpriority`]/[`setpriority`] with `PRIO_PROCESS`/`who = 0`.
pub fn nice(inc: i32) -> Result<i32, Errno> {
    let current = getpriority(PRIO_PROCESS, 0)?;
    let target = current + inc;
    setpriority(PRIO_PROCESS, 0, target)?;
    // The kernel clamps out-of-range values; report what it actually set.
    getpriority(PRIO_PROCESS, 0)
}

// --- prctl(2) ----------------------------------------------------------

/// [`prctl`] `option`: set the calling thread's parent-death signal (`arg2`
/// is a signal number, or `0` to clear it).
pub const PR_SET_PDEATHSIG: i32 = 1;
/// [`prctl`] `option`: get the calling thread's parent-death signal (`arg2`
/// is a `*mut i32` the kernel writes it into).
pub const PR_GET_PDEATHSIG: i32 = 2;

/// Raw `prctl(2)`: process-behavior control. Covers whichever `option` the
/// caller passes; [`set_pdeathsig`]/[`get_pdeathsig`] are the safe,
/// narrow convenience this crate names explicitly.
///
/// # Safety
/// Some `option` values interpret `arg2`..`arg5` as pointers (e.g.
/// [`PR_GET_PDEATHSIG`]); the caller must pass values valid for whichever
/// `option` is used.
pub unsafe fn prctl(
    option: i32,
    arg2: usize,
    arg3: usize,
    arg4: usize,
    arg5: usize,
) -> Result<i32, Errno> {
    // SAFETY: forwarded to the caller's contract on `option`/`arg2..arg5`.
    let ret = unsafe { syscall5(nr::PRCTL, option as usize, arg2, arg3, arg4, arg5) };
    from_ret_i32(ret)
}

/// Ask the kernel to send `sig` to the calling thread if its parent thread
/// exits first (`0` clears it). The standard fix for orphaned children
/// surviving a crashed/killed parent: a job-control shell's children can
/// arrange to die with it instead of being silently reparented to init.
///
/// Cleared across `execve` of a set-user-ID/set-group-ID binary, and (per
/// `prctl(2)`) not inherited by a further `fork`, so a process that forks
/// again must re-arm it in the new child if it wants the same protection
/// there.
pub fn set_pdeathsig(sig: i32) -> Result<(), Errno> {
    // SAFETY: PR_SET_PDEATHSIG interprets arg2 as a plain signal number, not
    // a pointer; arg3..arg5 are ignored for this option.
    unsafe { prctl(PR_SET_PDEATHSIG, sig as usize, 0, 0, 0) }.map(|_| ())
}

/// Get the calling thread's current parent-death signal (`0` if none is set).
pub fn get_pdeathsig() -> Result<i32, Errno> {
    let mut sig: i32 = 0;
    // SAFETY: PR_GET_PDEATHSIG writes through arg2 as a valid `*mut i32`;
    // `sig` is exactly that, exclusively borrowed for the call.
    unsafe { prctl(PR_GET_PDEATHSIG, &mut sig as *mut i32 as usize, 0, 0, 0) }?;
    Ok(sig)
}

// --- uname(2) ------------------------------------------------------------

/// System identification (kernel `struct new_utsname`): kernel name,
/// hostname, release, version, machine, and NIS/YP domain name, each a
/// fixed-size, NUL-terminated byte string. The primitive behind
/// `$OSTYPE`/`$MACHTYPE`-style shell variables.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct Utsname {
    /// Kernel name (always `"Linux"` for this crate, which is Linux-only).
    pub sysname: [u8; 65],
    /// Network node hostname.
    pub nodename: [u8; 65],
    /// Kernel release (e.g. `"6.8.0"`).
    pub release: [u8; 65],
    /// Kernel version/build string.
    pub version: [u8; 65],
    /// Hardware/architecture name (e.g. `"x86_64"`, `"aarch64"`).
    pub machine: [u8; 65],
    /// NIS/YP domain name.
    pub domainname: [u8; 65],
}

const _: () = assert!(core::mem::size_of::<Utsname>() == 390);

impl Default for Utsname {
    fn default() -> Self {
        Utsname {
            sysname: [0; 65],
            nodename: [0; 65],
            release: [0; 65],
            version: [0; 65],
            machine: [0; 65],
            domainname: [0; 65],
        }
    }
}

impl Utsname {
    /// Trim a field at its first NUL byte and return the C-string view.
    fn field(bytes: &[u8; 65]) -> &CStr {
        CStr::from_bytes_until_nul(bytes)
            .expect("kernel-filled utsname field is always NUL-terminated")
    }

    /// Kernel name as a C-string view (always `"Linux"` here).
    #[inline]
    pub fn sysname(&self) -> &CStr {
        Self::field(&self.sysname)
    }
    /// Network node hostname as a C-string view.
    #[inline]
    pub fn nodename(&self) -> &CStr {
        Self::field(&self.nodename)
    }
    /// Kernel release as a C-string view.
    #[inline]
    pub fn release(&self) -> &CStr {
        Self::field(&self.release)
    }
    /// Kernel version/build string as a C-string view.
    #[inline]
    pub fn version(&self) -> &CStr {
        Self::field(&self.version)
    }
    /// Hardware/architecture name as a C-string view.
    #[inline]
    pub fn machine(&self) -> &CStr {
        Self::field(&self.machine)
    }
    /// NIS/YP domain name as a C-string view.
    #[inline]
    pub fn domainname(&self) -> &CStr {
        Self::field(&self.domainname)
    }
}

/// Get system identification: kernel name/release/version, hostname,
/// machine/architecture, and domain name.
pub fn uname() -> Result<Utsname, Errno> {
    let mut buf = Utsname::default();
    // SAFETY: `buf` is a valid, exclusively-borrowed 390-byte `struct
    // new_utsname` the kernel writes completely on success.
    let ret = unsafe { syscall1(nr::UNAME, &mut buf as *mut Utsname as usize) };
    from_ret(ret)?;
    Ok(buf)
}

/// Set the process group ID of `pid` to `pgid` (both `0` mean "self").
pub fn setpgid(pid: i32, pgid: i32) -> Result<(), Errno> {
    // SAFETY: plain integer arguments, no memory referenced.
    let ret = unsafe { syscall2(nr::SETPGID, pid as usize, pgid as usize) };
    from_ret(ret).map(|_| ())
}

/// Send signal `sig` to `pid` (see `kill(2)` for the `pid` sign conventions).
pub fn kill(pid: i32, sig: i32) -> Result<(), Errno> {
    // SAFETY: plain integer arguments, no memory referenced.
    let ret = unsafe { syscall2(nr::KILL, pid as usize, sig as usize) };
    from_ret(ret).map(|_| ())
}

/// Send signal `sig` to every process in process group `pgrp`.
///
/// Equivalent to `kill(-pgrp, sig)`; `pgrp == 0` targets the caller's group.
#[inline]
pub fn killpg(pgrp: i32, sig: i32) -> Result<(), Errno> {
    // `kill` reads a negative pid as "process group -pid". Negating maps `pgrp`
    // to that form; `0.wrapping_neg() == 0`, and `kill(0, sig)` already means
    // "the caller's own group", so the `pgrp == 0` case falls out correctly.
    kill(pgrp.wrapping_neg(), sig)
}

/// Open a file descriptor referring to process `pid` (`pidfd_open(2)`,
/// Track P). Unlike a bare pid number, the returned fd stays a stable
/// handle to *this specific* process even if `pid` exits and the kernel
/// recycles the number for an unrelated process later — the reuse race a
/// raw pid can't avoid. Poll it for readability (readable once the
/// process exits) or wait on it via `waitid(P_PIDFD, ...)`.
///
/// `flags` must be `0` — the kernel defines no flags for this syscall as
/// of the versions this crate targets; a future one (e.g. `PIDFD_NONBLOCK`)
/// would be admitted here when a consumer needs it.
///
/// The returned fd is `O_CLOEXEC` by kernel default and must be closed
/// with [`crate::fd::close`] like any other fd.
pub fn pidfd_open(pid: i32, flags: u32) -> Result<i32, Errno> {
    // SAFETY: plain integer arguments, no memory referenced.
    let ret = unsafe { syscall2(nr::PIDFD_OPEN, pid as usize, flags as usize) };
    from_ret_i32(ret)
}

/// Send signal `sig` to the process referred to by `pidfd` (`pidfd_open`'s
/// companion on the signalling side, Track P). Unlike [`kill`], which
/// re-targets a numeric pid the kernel may since have recycled for an
/// unrelated process, `pidfd` is a stable handle to *this specific*
/// process: closing the last reuse-race window in the pidfd story
/// ([`pidfd_open`] closed it for lookup, [`crate::wait::waitid`] with
/// `P_PIDFD` closed it for reaping, this closes it for signalling).
///
/// `flags` must be `0` — the kernel defines no flags for this syscall as
/// of the versions this crate targets.
pub fn pidfd_send_signal(pidfd: i32, sig: i32) -> Result<(), Errno> {
    // pidfd_send_signal(pidfd, sig, info = NULL, flags = 0).
    // SAFETY: `pidfd`/`sig` are plain integers; a null `info` is valid and
    // means "no siginfo payload", matching a plain `kill`.
    let ret = unsafe { syscall4(nr::PIDFD_SEND_SIGNAL, pidfd as usize, sig as usize, 0, 0) };
    from_ret(ret).map(|_| ())
}

/// One `{ base, len }` range in another process's address space, for
/// [`process_vm_readv`]/[`process_vm_writev`]. Kernel `struct iovec` layout
/// (`void *iov_base; size_t iov_len;`), but `base` is a plain [`usize`]
/// address rather than a pointer: unlike [`crate::fd::IoSlice`], it names a
/// location in a *different* process's address space, which this process
/// has no valid reference to and must never dereference.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct RemoteIoVec {
    base: usize,
    len: usize,
}

const _: () = assert!(core::mem::size_of::<RemoteIoVec>() == 16);
const _: () = assert!(core::mem::offset_of!(RemoteIoVec, base) == 0);
const _: () = assert!(core::mem::offset_of!(RemoteIoVec, len) == 8);

impl RemoteIoVec {
    /// Describe `len` bytes starting at address `base` in the target
    /// process's address space.
    #[inline]
    pub fn new(base: usize, len: usize) -> Self {
        RemoteIoVec { base, len }
    }
}

/// Read `pid`'s memory into `local` (this process's buffers), scatter/gather
/// style, without the target stopped or attached via `ptrace` — a one-shot
/// snapshot read, not a debugging session. `remote` describes the source
/// ranges in the target's address space; `local`/`remote` are matched up
/// positionally but need not have the same segment sizes, only the same
/// total length. Requires `CAP_SYS_PTRACE` (or the same UID as `pid` with
/// matching `/proc/sys/kernel/yama/ptrace_scope` permissions). `flags` must
/// be `0` — the kernel defines no flags for this syscall as of the versions
/// this crate targets.
pub fn process_vm_readv(
    pid: i32,
    local: &mut [IoSliceMut],
    remote: &[RemoteIoVec],
) -> Result<usize, Errno> {
    // process_vm_readv(pid, lvec, liovcnt, rvec, riovcnt, flags = 0).
    // SAFETY: `local` exclusively borrows the buffers the kernel writes
    // into; `remote` only describes (never dereferences on our side) the
    // target's address ranges to read from.
    let ret = unsafe {
        syscall6(
            nr::PROCESS_VM_READV,
            pid as usize,
            local.as_ptr() as usize,
            local.len(),
            remote.as_ptr() as usize,
            remote.len(),
            0,
        )
    };
    from_ret(ret)
}

/// Write `local` (this process's buffers) into `pid`'s memory, scatter/gather
/// style — the write counterpart of [`process_vm_readv`]; same permission
/// requirements, same positional matching between `local` and `remote`.
pub fn process_vm_writev(
    pid: i32,
    local: &[IoSlice],
    remote: &[RemoteIoVec],
) -> Result<usize, Errno> {
    // process_vm_writev(pid, lvec, liovcnt, rvec, riovcnt, flags = 0).
    // SAFETY: `local` only borrows buffers the kernel reads from; `remote`
    // only describes the target's address ranges to write into.
    let ret = unsafe {
        syscall6(
            nr::PROCESS_VM_WRITEV,
            pid as usize,
            local.as_ptr() as usize,
            local.len(),
            remote.as_ptr() as usize,
            remote.len(),
            0,
        )
    };
    from_ret(ret)
}

/// Terminate all threads in the process with status `status`. Never returns.
pub fn exit_group(status: i32) -> ! {
    // SAFETY: exit_group never returns; the kernel tears the process down.
    unsafe {
        syscall1(nr::EXIT_GROUP, status as usize);
        // Unreachable, but keep the type `!` honest if the kernel ever did.
        core::hint::unreachable_unchecked()
    }
}

/// `SIGCHLD`: sent to the parent on child termination. Passed to `clone` as the
/// low byte of the flags so a plain wait reaps the child, matching `fork`.
const SIGCHLD: usize = 17;

/// Create a child process, returning the child's pid to the parent and `0` to
/// the child. Backed by `clone(SIGCHLD, stack = NULL, …)` — a null stack gives
/// the child a copy-on-write clone of the parent's stack, i.e. `fork`
/// semantics.
///
/// # Safety
///
/// This is a **raw** fork. Unlike glibc's `fork()`, it does **not** reset
/// glibc's internal malloc/stdio locks in the child, run `pthread_atfork`
/// handlers, or otherwise make a multithreaded parent safe. If any *other*
/// thread in the parent holds a lock (e.g. the malloc arena) at the instant of
/// the call, the child inherits it locked and deadlocks the first time it needs
/// it — and a Rust child that keeps running (rather than going straight to
/// `exec`/[`exit_group`]) will need the allocator almost immediately.
///
/// Only call this when the process is effectively single-threaded at the fork
/// point (no other thread can be mid-allocation), or when the child touches
/// nothing but async-signal-safe syscalls before `exec`/[`exit_group`]. See
/// rush's `LIBC_DEPENDENCY_ANALYSIS.md` §4.2.
pub unsafe fn fork() -> Result<i32, Errno> {
    // clone(flags = SIGCHLD, stack = 0, parent_tid = 0, child_tid = 0, tls = 0).
    // Argument order is the same on x86_64 and aarch64.
    // SAFETY: all pointer arguments are null; a null stack requests fork-style
    // copy-on-write of the caller's stack.
    let ret = unsafe { syscall5(nr::CLONE, SIGCHLD, 0, 0, 0, 0) };
    from_ret_i32(ret)
}

/// `clone3(2)` flag: place a pidfd for the new child in `Clone3Args::pidfd`.
const CLONE_PIDFD: u64 = 0x1000;

/// Arguments for `clone3(2)` (kernel `struct clone_args`). Every field is a
/// plain `u64`; pointer/fd-shaped fields are cast in and out rather than
/// typed, matching the kernel ABI, which is the same on x86_64 and aarch64.
#[repr(C)]
#[derive(Default)]
struct Clone3Args {
    flags: u64,
    pidfd: u64,
    child_tid: u64,
    parent_tid: u64,
    exit_signal: u64,
    stack: u64,
    stack_size: u64,
    tls: u64,
    set_tid: u64,
    set_tid_size: u64,
    cgroup: u64,
}

const _: () = assert!(core::mem::size_of::<Clone3Args>() == 88);

/// Create a child process together with a pidfd for it, atomically —
/// `clone3(2)` with `CLONE_PIDFD`. Returns `(child_pid, pidfd)` to the
/// parent and `(0, -1)` to the child (the kernel never writes back the
/// pidfd field in the child's copy of the arguments, so `-1` is used as an
/// explicit not-applicable sentinel rather than leaking the meaningless raw
/// value).
///
/// This closes the pid-reuse race in the `fork()` + [`pidfd_open`] pattern:
/// between those two separate syscalls the child can exit and, under a busy
/// pid space, have its pid recycled before `pidfd_open` runs. `clone3`
/// hands back the pidfd as part of process creation itself, so there is no
/// window in which the pid can be reused out from under the caller. Keep
/// `fork` + [`pidfd_open`] as the fallback for kernels without `clone3`
/// (added in Linux 5.3).
///
/// The returned pidfd is `O_CLOEXEC` by kernel default and must be closed
/// with [`crate::fd::close`] like any other fd.
///
/// # Safety
///
/// Same caveats as [`fork`]: this is a **raw** clone with `fork` semantics
/// (a null stack gives the child a copy-on-write clone of the parent's
/// stack). It does not reset glibc's malloc/stdio locks or run
/// `pthread_atfork` handlers. Only call this when the process is
/// effectively single-threaded at the call point, or when the child touches
/// nothing but async-signal-safe syscalls before `exec`/[`exit_group`].
pub unsafe fn fork_with_pidfd() -> Result<(i32, i32), Errno> {
    let mut args = Clone3Args {
        flags: CLONE_PIDFD,
        exit_signal: SIGCHLD as u64,
        ..Clone3Args::default()
    };
    // clone3(&args, size_of(args)).
    // SAFETY: `args` is a valid, exclusively-borrowed `*mut clone_args` of
    // the size passed; a null stack requests fork-style copy-on-write of
    // the caller's stack, as in `fork`.
    let ret = unsafe {
        syscall2(
            nr::CLONE3,
            &mut args as *mut Clone3Args as usize,
            core::mem::size_of::<Clone3Args>(),
        )
    };
    let pid = from_ret_i32(ret)?;
    if pid == 0 {
        return Ok((0, -1));
    }
    Ok((pid, args.pidfd as i32))
}

/// Replace the current process image with the program at `path`.
///
/// `argv` and `envp` must each point to a **null-terminated** array of C-string
/// pointers (`argv[0]` is conventionally the program name; `envp` may be a lone
/// null for an empty environment). On success this never returns — the process
/// image is replaced. It returns **only on failure**, yielding the [`Errno`].
///
/// # Safety
/// `argv` and `envp` must be valid, null-terminated arrays of valid C-string
/// pointers that live for the duration of the call, and `path` a valid C
/// string. After a `fork` in a multithreaded parent, the pre-`exec` child must
/// stay async-signal-safe (build the arrays without allocating); see [`fork`].
pub unsafe fn execve(path: &CStr, argv: *const *const c_char, envp: *const *const c_char) -> Errno {
    // SAFETY: pointer validity/termination is the caller's contract; the kernel
    // reads the C string and the two pointer arrays.
    let ret = unsafe {
        syscall3(
            nr::EXECVE,
            path.as_ptr() as usize,
            argv as usize,
            envp as usize,
        )
    };
    // execve returns only on error; decode the -errno. The `Ok` arm is
    // unreachable in practice (a successful exec never returns here).
    match from_ret(ret) {
        Ok(_) => Errno(0),
        Err(e) => e,
    }
}

/// Like [`execve`] but resolves `path` relative to `dirfd` (or [`AT_FDCWD`])
/// and takes `flags` (e.g. `AT_EMPTY_PATH` to exec an already-open fd).
///
/// [`AT_FDCWD`]: crate::fd::AT_FDCWD
///
/// # Safety
/// Same contract as [`execve`] for `path`/`argv`/`envp`.
pub unsafe fn execveat(
    dirfd: i32,
    path: &CStr,
    argv: *const *const c_char,
    envp: *const *const c_char,
    flags: i32,
) -> Errno {
    // SAFETY: as `execve`, plus `dirfd`/`flags` are plain integers.
    let ret = unsafe {
        syscall5(
            nr::EXECVEAT,
            dirfd as usize,
            path.as_ptr() as usize,
            argv as usize,
            envp as usize,
            flags as usize,
        )
    };
    match from_ret(ret) {
        Ok(_) => Errno(0),
        Err(e) => e,
    }
}

/// `clone(2)` flag: share the calling process's memory (address space) with
/// the child instead of copy-on-write duplicating it.
const CLONE_VM: usize = 0x100;
/// `clone(2)` flag: suspend the parent until the child calls `execve` or
/// exits.
const CLONE_VFORK: usize = 0x4000;

/// Fork via `CLONE_VFORK | CLONE_VM` and immediately `execve` in the child —
/// narrower than raw [`fork`], and safe to call from a multithreaded parent.
///
/// [`fork`]'s own safety note names the hazard this avoids: a raw
/// `clone(SIGCHLD)` child gets a copy-on-write duplicate of the parent's
/// address space, so if another parent thread holds an allocator lock at
/// the instant of the fork, the child inherits it locked and deadlocks the
/// first time it needs the allocator, before it ever reaches `exec`.
/// `CLONE_VM` shares the address space instead of duplicating it, and
/// `CLONE_VFORK` suspends the parent until the child calls `execve`/exits —
/// together, only one of the two ever runs at a time, so there is no
/// window in which a parent thread's lock and a child's allocation can
/// race at all. This is the technique `posix_spawn` implementations use
/// for the fork-then-exec case, which is the overwhelming majority of a
/// shell's forks.
///
/// Because `CLONE_VM` gives the child *actual* shared memory with the
/// parent (not a copy-on-write duplicate the way plain [`fork`] does),
/// ordinary Rust code in the child is not a safe way to reach that
/// `execve`: the child would need to *return* through call frames that
/// live on the same stack the parent's own continuation resumes through,
/// and the compiler is free to reuse a stack slot between "this local is
/// only live in the child branch" and "this local is only live in the
/// parent's continuation" — correct when at most one of those branches
/// ever executes per call, wrong here because both really do run,
/// sequentially, against the same physical memory. [`crate::arch::vfork_execve`]
/// does the entire clone-then-execve as one hand-written asm sequence for
/// exactly this reason — see its doc comment for the full account — and
/// this function is a thin, typed wrapper over it: it never returns
/// control to arbitrary caller code in the child, always either
/// `execve`ing or calling `exit_group(127)` (the shell "command not found"
/// convention) if that fails.
///
/// Returns the child's pid on success. Because nothing crosses back from
/// the child to the parent besides that pid (any richer channel would
/// reintroduce the shared-memory hazard above), an `execve` failure is not
/// distinguishable here from `clone` itself failing — both surface as
/// `Err`; the same `Err(errno)` that `clone(2)` itself produced when it's a
/// `clone` failure, or the child's `exit_group(127)` distinguishes an exec
/// failure after the fact via [`crate::wait`] the same way any fork+exec
/// caller already must.
///
/// # Safety
/// `path`, `argv`, `envp` must satisfy the same contract as [`execve`].
pub unsafe fn vfork_exec(
    path: &CStr,
    argv: *const *const c_char,
    envp: *const *const c_char,
) -> Result<i32, Errno> {
    // SAFETY: forwarded to the caller's contract on path/argv/envp.
    let ret = unsafe {
        crate::arch::vfork_execve(
            CLONE_VFORK | CLONE_VM | SIGCHLD,
            path.as_ptr().cast::<u8>(),
            argv.cast::<*const u8>(),
            envp.cast::<*const u8>(),
        )
    };
    from_ret_i32(ret)
}

/// One `dup2`-shaped fd redirection applied by [`vfork_exec_redirected`]
/// in the child before `execve` — matches `dup2(oldfd, newfd)`
/// (`oldfd == newfd` is a documented no-op, same as `dup2` itself).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Redirect {
    /// The fd to duplicate from (e.g. an already-open file for `cmd < file`).
    pub oldfd: i32,
    /// The fd to duplicate onto (e.g. `0` for stdin, `1` for stdout).
    pub newfd: i32,
}

const _: () = assert!(core::mem::size_of::<Redirect>() == 8);
const _: () = assert!(core::mem::offset_of!(Redirect, newfd) == 4);

/// Like [`vfork_exec`], but first applies `redirects` in the child, before
/// `execve` — the fd setup (`cmd > file`, `cmd 2>&1`, `cmd < file`, ...)
/// almost every real shell command needs, and which [`vfork_exec`] alone
/// cannot do: nothing runs in a `vfork`'s `CLONE_VM` child except the one
/// hand-written asm trampoline (see [`crate::arch::vfork_execve`]'s doc
/// comment for why), so redirection has to be *part of* that same
/// trampoline rather than ordinary Rust code layered on top of it.
/// [`crate::arch::vfork_execve_redirected`] is that extended trampoline: a
/// small loop, still entirely in registers, applying each redirect via a
/// raw `dup2`/`dup3` before the final `execve`.
///
/// If a redirect's `dup2` fails, the child exits `126` (found the command,
/// couldn't set it up to run) without ever reaching `execve`; a failing
/// `execve` itself still exits `127`, matching [`vfork_exec`] — a caller
/// distinguishes the two failure modes via the wait status, the same way
/// it already must distinguish "exec failed" from "ran and exited 127
/// itself".
///
/// # Safety
/// `path`, `argv`, `envp` must satisfy the same contract as [`execve`].
/// Every `oldfd` in `redirects` must be a valid, open fd in the calling
/// process at the time of the call (they are inherited into the vforked
/// child unchanged, since `CLONE_VM` shares everything).
pub unsafe fn vfork_exec_redirected(
    path: &CStr,
    argv: *const *const c_char,
    envp: *const *const c_char,
    redirects: &[Redirect],
) -> Result<i32, Errno> {
    // SAFETY: forwarded to the caller's contract on path/argv/envp/redirects.
    let ret = unsafe {
        crate::arch::vfork_execve_redirected(
            CLONE_VFORK | CLONE_VM | SIGCHLD,
            path.as_ptr().cast::<u8>(),
            argv.cast::<*const u8>(),
            envp.cast::<*const u8>(),
            redirects.as_ptr().cast::<u8>(),
            redirects.len(),
        )
    };
    from_ret_i32(ret)
}

/// A null-terminated array of C-string pointers, owned and kept alive
/// alongside it — the shape [`execve`]/[`execveat`] need for both `argv` and
/// `envp` (they are structurally identical: a NUL-terminated list of C
/// strings). Requires the opt-in `std` feature; the `no_std` core has no
/// allocator to build this with, so callers there still hand-roll the arrays
/// as documented on [`execve`].
///
/// ```rust,ignore
/// use rusty_libc::process::CStrArray;
///
/// let argv = CStrArray::new(["/bin/sh", "-c", "exit 0"]).expect("no interior NUL");
/// let envp = CStrArray::new(["PATH=/bin"]).expect("no interior NUL");
/// unsafe { rusty_libc::process::execve(c"/bin/sh", argv.as_ptr(), envp.as_ptr()) };
/// ```
// Gated on `any(feature = "std", test)`, not just `feature = "std"`, matching
// `Errno`'s `std::io::Error` interop impl: the built-in test harness always
// links `std` (see the crate's top-level `cfg_attr(not(any(test, feature =
// "std")), no_std)`), so this lets the type's own tests run under a plain
// `cargo test` instead of only being compile-checked by the separate "Build
// (std feature)" CI step.
#[cfg(any(feature = "std", test))]
pub struct CStrArray {
    // Kept alive so `ptrs` stays valid; never read directly after construction.
    _owned: std::vec::Vec<std::ffi::CString>,
    // `_owned`'s pointers, plus a trailing null -- this is what `as_ptr` hands
    // to `execve`/`execveat`.
    ptrs: std::vec::Vec<*const c_char>,
}

#[cfg(any(feature = "std", test))]
impl CStrArray {
    /// Build a null-terminated C-string array from owned strings (or byte
    /// slices), copying each into a fresh [`std::ffi::CString`]. Fails if any
    /// input contains an interior NUL byte (which cannot be represented as a
    /// C string).
    pub fn new<I, S>(strings: I) -> Result<Self, std::ffi::NulError>
    where
        I: IntoIterator<Item = S>,
        S: Into<std::vec::Vec<u8>>,
    {
        let owned = strings
            .into_iter()
            .map(std::ffi::CString::new)
            .collect::<Result<std::vec::Vec<_>, _>>()?;
        let mut ptrs: std::vec::Vec<*const c_char> = owned.iter().map(|s| s.as_ptr()).collect();
        ptrs.push(core::ptr::null());
        Ok(CStrArray {
            _owned: owned,
            ptrs,
        })
    }

    /// The null-terminated pointer array, ready to pass as `execve`'s `argv`
    /// or `envp`. Valid for as long as `self` is alive.
    #[inline]
    pub fn as_ptr(&self) -> *const *const c_char {
        self.ptrs.as_ptr()
    }
}

/// Change the calling process's working directory to `path`.
pub fn chdir(path: &CStr) -> Result<(), Errno> {
    // SAFETY: `path` is a valid nul-terminated C string the kernel only reads.
    let ret = unsafe { syscall1(nr::CHDIR, path.as_ptr() as usize) };
    from_ret(ret).map(|_| ())
}

/// Change the calling process's working directory to the one referred to by the
/// open descriptor `fd`.
pub fn fchdir(fd: i32) -> Result<(), Errno> {
    // SAFETY: plain integer argument.
    let ret = unsafe { syscall1(nr::FCHDIR, fd as usize) };
    from_ret(ret).map(|_| ())
}

/// Write the absolute path of the current working directory into `buf` and
/// return the path bytes (**without** the trailing NUL).
///
/// Fails with `ERANGE` if `buf` is too small to hold the path and its NUL.
pub fn getcwd(buf: &mut [u8]) -> Result<&[u8], Errno> {
    // The kernel's getcwd returns the length *including* the NUL terminator.
    // SAFETY: `buf` is a valid, exclusively-borrowed slice of `buf.len()`
    // bytes; the kernel writes at most that many.
    let ret = unsafe { syscall2(nr::GETCWD, buf.as_mut_ptr() as usize, buf.len()) };
    let len = from_ret(ret)?;
    // len is >= 1 (at minimum "/\0"); drop the trailing NUL byte.
    Ok(&buf[..len.saturating_sub(1)])
}

/// Create a new session and set the process group, making the caller the
/// session and group leader. Returns the new session ID. Fails with `EPERM` if
/// the caller is already a process-group leader.
pub fn setsid() -> Result<i32, Errno> {
    // SAFETY: takes no arguments.
    let ret = unsafe { syscall0(nr::SETSID) };
    from_ret_i32(ret)
}

/// Get the process-group ID of `pid` (`0` means the calling process).
pub fn getpgid(pid: i32) -> Result<i32, Errno> {
    // SAFETY: plain integer argument.
    let ret = unsafe { syscall1(nr::GETPGID, pid as usize) };
    from_ret_i32(ret)
}

/// Get the session ID of `pid` (`0` means the calling process).
pub fn getsid(pid: i32) -> Result<i32, Errno> {
    // SAFETY: plain integer argument.
    let ret = unsafe { syscall1(nr::GETSID, pid as usize) };
    from_ret_i32(ret)
}

/// Get the calling process's own process-group ID. Convenience for
/// [`getpgid`]`(0)` — the classic no-argument `getpgrp()`.
///
/// Implemented via `getpgid(0)` so it is identical on both arches (aarch64 has
/// no dedicated `getpgrp` syscall); querying your own group cannot meaningfully
/// fail.
#[inline]
pub fn getpgrp() -> Result<i32, Errno> {
    getpgid(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_consistent() {
        assert!(getpid() > 0);
        assert!(getppid() > 0);
        // getpid must match the value std reports.
        assert_eq!(getpid() as u32, std::process::id());
    }

    #[test]
    fn gettid_is_stable_per_thread_and_differs_across_threads() {
        // Note: the test harness itself runs each test on its own worker
        // thread, so this thread's tid is not expected to equal getpid()
        // here -- that equality only holds for a process's actual main
        // thread, which no #[test] body runs on.
        let this_tid = gettid();
        assert!(this_tid > 0);
        assert_eq!(gettid(), this_tid);

        let other_tid = std::thread::spawn(gettid).join().expect("thread join");
        assert_ne!(other_tid, this_tid);
    }

    #[test]
    fn credential_getters_are_consistent() {
        // The test process is not setuid/setgid, so effective == real, and the
        // getters are stable across calls.
        assert_eq!(geteuid(), getuid());
        assert_eq!(getegid(), getgid());
        assert_eq!(getuid(), getuid());
    }

    #[test]
    fn getresuid_getresgid_agree_with_the_two_id_getters() {
        // Not setuid/setgid, so real == effective == saved for both; cross-
        // check against the already-established getuid/geteuid getters
        // rather than just asserting internal self-consistency.
        let (ruid, euid, suid) = getresuid().expect("getresuid");
        assert_eq!(ruid, getuid());
        assert_eq!(euid, geteuid());
        assert_eq!(suid, getuid());

        let (rgid, egid, sgid) = getresgid().expect("getresgid");
        assert_eq!(rgid, getgid());
        assert_eq!(egid, getegid());
        assert_eq!(sgid, getgid());
    }

    #[test]
    fn uname_reports_linux_and_matching_machine() {
        let u = uname().expect("uname");
        assert_eq!(u.sysname().to_bytes(), b"Linux");

        #[cfg(target_arch = "x86_64")]
        assert_eq!(u.machine().to_bytes(), b"x86_64");
        #[cfg(target_arch = "aarch64")]
        assert_eq!(u.machine().to_bytes(), b"aarch64");

        // release/version are non-empty on any real kernel.
        assert!(!u.release().to_bytes().is_empty());
        assert!(!u.version().to_bytes().is_empty());
    }

    #[test]
    fn ngroups_and_getgroups_agree() {
        let n = ngroups().expect("ngroups");

        // A buffer sized exactly by ngroups() must be filled completely.
        let mut buf = std::vec![0u32; n];
        let groups = getgroups(&mut buf).expect("getgroups");
        assert_eq!(groups.len(), n);

        // A too-small (but non-empty, when there's at least one group)
        // buffer is rejected rather than silently truncated.
        if n > 0 {
            let mut tiny = std::vec![0u32; n - 1];
            assert_eq!(getgroups(&mut tiny), Err(Errno::EINVAL));
        }

        // A larger-than-needed buffer still reports exactly `n` entries.
        let mut spare = std::vec![0u32; n + 4];
        assert_eq!(getgroups(&mut spare).expect("getgroups spare").len(), n);
    }

    #[test]
    fn setuid_family_noop_transitions_are_always_permitted() {
        use crate::wait;

        // Setting an id to its own current value (or, for the resuid/resgid
        // form, passing KEEP_ID for every argument) is permitted for every
        // caller regardless of privilege -- unlike an actual transition to a
        // *different* id, this needs no privilege gating. Still isolated in
        // a forked child: these mutate real process-credential state, and
        // this crate's own dev sandbox happens to run as root, but a
        // consumer's CI must not be assumed to. See `fork`'s safety note.
        match unsafe { fork() }.expect("fork") {
            0 => {
                let (uid, gid) = (getuid(), getgid());

                if setuid(uid).is_err() || getuid() != uid {
                    exit_group(1);
                }
                if setgid(gid).is_err() || getgid() != gid {
                    exit_group(2);
                }
                if setresuid(uid, uid, uid).is_err() || getuid() != uid {
                    exit_group(3);
                }
                if setresgid(gid, gid, gid).is_err() || getgid() != gid {
                    exit_group(4);
                }
                // All-KEEP_ID: an explicit no-op, exercising the sentinel.
                if setresuid(KEEP_ID, KEEP_ID, KEEP_ID).is_err() || getuid() != uid {
                    exit_group(5);
                }
                if setresgid(KEEP_ID, KEEP_ID, KEEP_ID).is_err() || getgid() != gid {
                    exit_group(6);
                }

                exit_group(0);
            }
            pid => {
                let (_, status) = wait::waitpid(pid, 0).expect("waitpid");
                assert!(wait::wifexited(status), "child did not exit normally");
                assert_eq!(wait::wexitstatus(status), 0, "child assertion failed");
            }
        }
    }

    #[test]
    fn seteuid_setegid_transition_when_privileged() {
        use crate::wait;

        // A transition to a genuinely *different* id needs CAP_SETUID/
        // CAP_SETGID; only assert it actually took effect when running
        // privileged (issue the syscalls unconditionally either way, so a
        // regression in argument order/flags still shows up as an
        // unexpected error). Isolated in a forked child since a successful
        // transition drops capabilities for the rest of that process.
        match unsafe { fork() }.expect("fork") {
            0 => {
                let target_uid = getuid().wrapping_add(1);
                match seteuid(target_uid) {
                    Ok(()) if geteuid() == target_uid => {}
                    Ok(()) => exit_group(1),
                    Err(Errno::EPERM) => {} // unprivileged: expected, not a crate bug
                    Err(_) => exit_group(2),
                }

                let target_gid = getgid().wrapping_add(1);
                match setegid(target_gid) {
                    Ok(()) if getegid() == target_gid => {}
                    Ok(()) => exit_group(3),
                    Err(Errno::EPERM) => {}
                    Err(_) => exit_group(4),
                }

                exit_group(0);
            }
            pid => {
                let (_, status) = wait::waitpid(pid, 0).expect("waitpid");
                assert!(wait::wifexited(status), "child did not exit normally");
                assert_eq!(wait::wexitstatus(status), 0, "child assertion failed");
            }
        }
    }

    #[test]
    fn setgroups_roundtrip_when_privileged() {
        use crate::wait;

        // Same privilege-gating shape as `seteuid_setegid_transition_when_privileged`:
        // CAP_SETGID is needed to actually change the group list, so only
        // assert the new list took effect when privileged.
        match unsafe { fork() }.expect("fork") {
            0 => {
                let target = [100u32, 200];
                match setgroups(&target) {
                    Ok(()) => {
                        let mut buf = [0u32; 2];
                        match getgroups(&mut buf) {
                            Ok(got) if got == target => {}
                            _ => exit_group(1),
                        }
                    }
                    Err(Errno::EPERM) => {} // unprivileged: expected, not a crate bug
                    Err(_) => exit_group(2),
                }
                exit_group(0);
            }
            pid => {
                let (_, status) = wait::waitpid(pid, 0).expect("waitpid");
                assert!(wait::wifexited(status), "child did not exit normally");
                assert_eq!(wait::wexitstatus(status), 0, "child assertion failed");
            }
        }
    }

    #[test]
    fn getpriority_setpriority_and_nice_roundtrip() {
        use crate::wait;

        // Isolated in a forked child: setpriority mutates real process
        // state, and unprivileged callers (e.g. an unprivileged CI runner --
        // this crate's own suite happens to run as root, but must not
        // assume every consumer's does) can raise the nice value but not
        // lower it back down without CAP_SYS_NICE. Rather than require a
        // restore step this crate can't portably guarantee, let a
        // throwaway child carry the mutation and report pass/fail through
        // its exit code; the mutation dies with it. CI runs single-threaded
        // (see ci.yml) and the child touches only raw syscalls, so this
        // follows `fork`'s safety note.
        match unsafe { fork() }.expect("fork") {
            0 => {
                let original = match getpriority(PRIO_PROCESS, 0) {
                    Ok(v) => v,
                    Err(_) => exit_group(1),
                };

                // Raising the nice value (lowering priority) is always
                // permitted, even unprivileged.
                if setpriority(PRIO_PROCESS, 0, original + 3).is_err() {
                    exit_group(2);
                }
                if getpriority(PRIO_PROCESS, 0) != Ok(original + 3) {
                    exit_group(3);
                }

                // nice() is a relative adjustment over the *current* value,
                // not the original one.
                let n = match nice(2) {
                    Ok(v) => v,
                    Err(_) => exit_group(4),
                };
                if n != original + 5 {
                    exit_group(5);
                }
                if getpriority(PRIO_PROCESS, 0) != Ok(original + 5) {
                    exit_group(6);
                }

                exit_group(0);
            }
            pid => {
                let (_, status) = wait::waitpid(pid, 0).expect("waitpid");
                assert!(wait::wifexited(status), "child did not exit normally");
                assert_eq!(wait::wexitstatus(status), 0, "child assertion failed");
            }
        }
    }

    #[test]
    fn getpriority_pgrp_and_user_variants_do_not_error() {
        // 0 means "the caller's own group/uid" for these `which` values.
        assert!(getpriority(PRIO_PGRP, 0).is_ok());
        assert!(getpriority(PRIO_USER, 0).is_ok());
    }

    #[test]
    fn getpriority_bad_which_is_einval() {
        assert_eq!(getpriority(9999, 0), Err(Errno::EINVAL));
    }

    #[test]
    fn setpgid_self_is_noop_ok() {
        // Making the process its own group leader (or re-affirming it) either
        // succeeds or fails with EPERM depending on session state; both are
        // valid, non-panicking outcomes. Assert it does not blow up.
        let _ = setpgid(0, 0);
    }

    #[test]
    fn session_and_group_ids_are_positive() {
        // 0 means "the calling process" for both queries.
        let pgid = getpgid(0).expect("getpgid(0)");
        let sid = getsid(0).expect("getsid(0)");
        assert!(pgid > 0);
        assert!(sid > 0);
        // Querying by explicit pid agrees with the pid==0 shorthand.
        assert_eq!(getpgid(getpid()).expect("getpgid(pid)"), pgid);
        assert_eq!(getsid(getpid()).expect("getsid(pid)"), sid);
        // getpgrp() is getpgid(0).
        assert_eq!(getpgrp().expect("getpgrp"), pgid);
    }

    #[test]
    fn getpgid_missing_process_is_esrch() {
        // No process has this pid: getpgid reports ESRCH (3).
        assert_eq!(getpgid(0x3fff_ffff), Err(Errno(3)));
    }

    // chdir mutates process-global state, so all cwd assertions live in one
    // sequential test to avoid races with other tests (which all use absolute
    // paths and are unaffected as long as we restore the cwd promptly).
    #[test]
    fn chdir_fchdir_getcwd_roundtrip() {
        let mut buf = [0u8; 4096];
        let saved = getcwd(&mut buf).expect("getcwd").to_vec();

        // chdir to a known absolute directory and read it back.
        chdir(c"/").expect("chdir /");
        let mut buf2 = [0u8; 4096];
        assert_eq!(getcwd(&mut buf2).expect("getcwd /"), b"/");

        // fchdir to /tmp via an open directory fd, then confirm.
        let dirfd = crate::fd::open(c"/tmp", crate::fd::O_RDONLY | crate::fd::O_DIRECTORY, 0)
            .expect("open /tmp");
        fchdir(dirfd).expect("fchdir /tmp");
        let mut buf3 = [0u8; 4096];
        assert_eq!(getcwd(&mut buf3).expect("getcwd /tmp"), b"/tmp");
        crate::fd::close(dirfd).expect("close");

        // A too-small buffer yields ERANGE.
        let mut tiny = [0u8; 1];
        assert_eq!(getcwd(&mut tiny), Err(Errno::ERANGE));

        // Restore the original working directory.
        let back = std::ffi::CString::new(saved).unwrap();
        chdir(&back).expect("chdir back");
    }

    #[test]
    fn setsid_in_child_creates_new_session() {
        use crate::wait;
        // setsid fails with EPERM for a process-group leader, so exercise it in
        // a fresh child (which is not a group leader): it succeeds and returns
        // the new session id, equal to the child's own pid. The child stays
        // async-signal-safe (raw syscalls only); see `fork`'s safety note.
        match unsafe { fork() }.expect("fork") {
            0 => {
                let ok = matches!((setsid(), getpid()), (Ok(sid), pid) if sid == pid);
                exit_group(if ok { 0 } else { 1 });
            }
            pid => {
                let (_, status) = wait::waitpid(pid, 0).expect("waitpid");
                assert!(wait::wifexited(status));
                assert_eq!(wait::wexitstatus(status), 0);
            }
        }
    }

    #[test]
    fn execve_missing_file_returns_enoent() {
        // execve returns only on failure; a bad path never replaces our image,
        // so this is safe to run without a fork.
        let path = c"/nonexistent/rusty_libc/prog";
        let argv: [*const c_char; 2] = [path.as_ptr(), core::ptr::null()];
        let envp: [*const c_char; 1] = [core::ptr::null()];
        let e = unsafe { execve(path, argv.as_ptr(), envp.as_ptr()) };
        assert_eq!(e, Errno::ENOENT);
    }

    #[test]
    fn execveat_missing_file_returns_enoent() {
        // Regression test: `execveat`'s own syscall number, not `execve`'s.
        // aarch64's EXECVEAT constant was wrong for a while (387, an
        // unallocated number, instead of the real generic-ABI 281) and no
        // test called `execveat` directly, so it went uncaught -- `execve`'s
        // test above exercises a different `nr` constant entirely. This
        // fails with ENOSYS instead of ENOENT if the syscall number regresses.
        let path = c"/nonexistent/rusty_libc/prog";
        let argv: [*const c_char; 2] = [path.as_ptr(), core::ptr::null()];
        let envp: [*const c_char; 1] = [core::ptr::null()];
        let e = unsafe { execveat(crate::fd::AT_FDCWD, path, argv.as_ptr(), envp.as_ptr(), 0) };
        assert_eq!(e, Errno::ENOENT);
    }

    #[test]
    fn vfork_exec_missing_file_exits_127() {
        // Unlike `execve_missing_file_returns_enoent`, this always forks
        // (vfork_exec has no "just call it inline" mode), and unlike a bare
        // `execve` call, an exec failure inside the child can't propagate
        // back through the `Result` here -- see `vfork_exec`'s doc comment
        // for why. `clone` itself still succeeds even with a bad path (the
        // failure only shows up once the child tries to exec it), so this
        // returns `Ok`, and the caller checks the exit-127 convention via
        // `waitpid` -- a missing path never actually execs a real image,
        // so this runs identically on every arch/emulator, no need to gate
        // it to x86_64.
        use crate::wait;

        let path = c"/nonexistent/rusty_libc/prog";
        let argv: [*const c_char; 2] = [path.as_ptr(), core::ptr::null()];
        let envp: [*const c_char; 1] = [core::ptr::null()];
        let pid = unsafe { vfork_exec(path, argv.as_ptr(), envp.as_ptr()) }.expect("vfork_exec");
        let (wpid, status) = wait::waitpid(pid, 0).expect("waitpid");
        assert_eq!(wpid, pid);
        assert!(wait::wifexited(status));
        assert_eq!(wait::wexitstatus(status), 127);
    }

    // The success path replaces the child image with a real binary. Gate it to
    // x86_64 so it runs natively; under the aarch64 qemu-user CI job a nested
    // execve of a host binary is not reliably emulated.
    #[cfg(target_arch = "x86_64")]
    #[test]
    fn execve_replaces_child_image() {
        use crate::wait;
        match unsafe { fork() }.expect("fork") {
            0 => {
                // exec `/bin/sh -c "exit 7"`; a delivered argv proves through
                // the child's exit code.
                let path = c"/bin/sh";
                let argv: [*const c_char; 4] = [
                    c"/bin/sh".as_ptr(),
                    c"-c".as_ptr(),
                    c"exit 7".as_ptr(),
                    core::ptr::null(),
                ];
                let envp: [*const c_char; 1] = [core::ptr::null()];
                unsafe { execve(path, argv.as_ptr(), envp.as_ptr()) };
                // Only reached if exec failed.
                exit_group(127);
            }
            pid => {
                let (_, status) = wait::waitpid(pid, 0).expect("waitpid");
                assert!(wait::wifexited(status));
                assert_eq!(wait::wexitstatus(status), 7);
            }
        }
    }

    // Gated to x86_64 for the same reason as `execve_replaces_child_image`: a
    // nested execve of a host binary isn't reliably emulated under the
    // aarch64 qemu-user CI job.
    #[cfg(target_arch = "x86_64")]
    #[test]
    fn vfork_exec_replaces_child_image_and_reports_no_error() {
        use crate::wait;

        let path = c"/bin/sh";
        let argv: [*const c_char; 4] = [
            c"/bin/sh".as_ptr(),
            c"-c".as_ptr(),
            c"exit 7".as_ptr(),
            core::ptr::null(),
        ];
        let envp: [*const c_char; 1] = [core::ptr::null()];

        let pid = unsafe { vfork_exec(path, argv.as_ptr(), envp.as_ptr()) }.expect("vfork_exec");
        let (wpid, status) = wait::waitpid(pid, 0).expect("waitpid");
        assert_eq!(wpid, pid);
        assert!(wait::wifexited(status));
        assert_eq!(wait::wexitstatus(status), 7);
    }

    #[test]
    fn vfork_exec_redirected_bad_oldfd_exits_126() {
        use crate::wait;

        // A bad oldfd makes the dup2/dup3 itself fail, so execve is never
        // reached at all -- the path/argv/envp are never used, and this
        // runs identically on every arch/emulator (no real exec involved),
        // unlike the success-path test below.
        let path = c"/nonexistent/rusty_libc/prog";
        let argv: [*const c_char; 2] = [path.as_ptr(), core::ptr::null()];
        let envp: [*const c_char; 1] = [core::ptr::null()];
        let redirects = [Redirect {
            oldfd: -1,
            newfd: 0,
        }];

        let pid = unsafe { vfork_exec_redirected(path, argv.as_ptr(), envp.as_ptr(), &redirects) }
            .expect("vfork_exec_redirected");
        let (wpid, status) = wait::waitpid(pid, 0).expect("waitpid");
        assert_eq!(wpid, pid);
        assert!(wait::wifexited(status));
        assert_eq!(
            wait::wexitstatus(status),
            126,
            "a redirect failure must be distinguishable from a 127 exec failure"
        );
    }

    #[test]
    fn vfork_exec_redirected_skips_dup2_when_oldfd_equals_newfd() {
        use crate::wait;

        // Portable (no real exec needed) regression test for the
        // oldfd == newfd skip: aarch64's dup3-based implementation has no
        // no-op-on-equal case the way dup2 does, so if the skip were
        // missing there, this redirect alone would fail with EINVAL and
        // the child would exit 126 -- instead of skipping it, reaching
        // execve, and failing on the missing path with the expected 127.
        // `vfork_exec_redirected_applies_dup2_before_exec` covers the same
        // skip on x86_64 as part of a real exec, but that test is gated
        // off aarch64 (qemu-user can't reliably exec a host binary), so
        // this is the only exercise of the skip path there.
        let path = c"/nonexistent/rusty_libc/prog";
        let argv: [*const c_char; 2] = [path.as_ptr(), core::ptr::null()];
        let envp: [*const c_char; 1] = [core::ptr::null()];
        let redirects = [Redirect { oldfd: 2, newfd: 2 }];

        let pid = unsafe { vfork_exec_redirected(path, argv.as_ptr(), envp.as_ptr(), &redirects) }
            .expect("vfork_exec_redirected");
        let (wpid, status) = wait::waitpid(pid, 0).expect("waitpid");
        assert_eq!(wpid, pid);
        assert!(wait::wifexited(status));
        assert_eq!(
            wait::wexitstatus(status),
            127,
            "the equal-fd redirect must be skipped, not fail the whole call"
        );
    }

    // Gated to x86_64 for the same reason as `vfork_exec_replaces_child_image_and_reports_no_error`.
    #[cfg(target_arch = "x86_64")]
    #[test]
    fn vfork_exec_redirected_applies_dup2_before_exec() {
        use crate::fd;
        use crate::wait;

        let (r, w) = fd::pipe2(0).expect("pipe2");
        let path = c"/bin/sh";
        let argv: [*const c_char; 4] = [
            c"/bin/sh".as_ptr(),
            c"-c".as_ptr(),
            c"echo hello".as_ptr(),
            core::ptr::null(),
        ];
        let envp: [*const c_char; 1] = [core::ptr::null()];
        // Two entries: the real redirect (stdout -> the pipe), plus a
        // deliberate no-op (oldfd == newfd) exercising the skip path --
        // if that skip were missing, aarch64's dup3-based implementation
        // would fail this with EINVAL (dup3 has no no-op-on-equal case)
        // and the whole call would exit 126 instead of succeeding.
        let redirects = [
            Redirect { oldfd: w, newfd: 1 },
            Redirect { oldfd: 2, newfd: 2 },
        ];

        let pid = unsafe { vfork_exec_redirected(path, argv.as_ptr(), envp.as_ptr(), &redirects) }
            .expect("vfork_exec_redirected");
        fd::close(w).expect("close w in parent");

        let mut buf = [0u8; 32];
        let n = fd::read(r, &mut buf).expect("read");
        assert_eq!(&buf[..n], b"hello\n");

        let (wpid, status) = wait::waitpid(pid, 0).expect("waitpid");
        assert_eq!(wpid, pid);
        assert!(wait::wifexited(status));
        assert_eq!(wait::wexitstatus(status), 0);

        fd::close(r).expect("close r");
    }

    #[test]
    fn cstrarray_builds_a_null_terminated_pointer_array() {
        let argv = CStrArray::new(["/bin/sh", "-c", "exit 0"]).expect("no interior NUL");
        // SAFETY: reading back what `new` just wrote, purely to check shape.
        let ptrs = unsafe { core::slice::from_raw_parts(argv.as_ptr(), 4) };
        assert!(!ptrs[0].is_null());
        assert!(!ptrs[1].is_null());
        assert!(!ptrs[2].is_null());
        assert!(ptrs[3].is_null(), "must be null-terminated");

        let s0 = unsafe { CStr::from_ptr(ptrs[0]) };
        assert_eq!(s0.to_bytes(), b"/bin/sh");
    }

    #[test]
    fn cstrarray_rejects_interior_nul() {
        assert!(CStrArray::new(["bad\0arg"]).is_err());
    }

    // Gated to x86_64 for the same reason as `execve_replaces_child_image`:
    // a nested execve of a host binary isn't reliably emulated under the
    // aarch64 qemu-user CI job.
    #[cfg(target_arch = "x86_64")]
    #[test]
    fn cstrarray_is_usable_end_to_end_with_execve() {
        use crate::wait;

        let argv = CStrArray::new(["/bin/sh", "-c", "exit 9"]).expect("no interior NUL");
        let envp = CStrArray::new::<_, &str>([]).expect("no interior NUL");

        match unsafe { fork() }.expect("fork") {
            0 => {
                unsafe { execve(c"/bin/sh", argv.as_ptr(), envp.as_ptr()) };
                exit_group(127); // only reached if exec failed
            }
            pid => {
                let (_, status) = wait::waitpid(pid, 0).expect("waitpid");
                assert!(wait::wifexited(status));
                assert_eq!(wait::wexitstatus(status), 9);
            }
        }
    }

    #[test]
    fn fork_child_runs_and_is_reaped() {
        use crate::fd;
        use crate::wait;

        // The child talks to the parent over a pipe, then exits. CI runs the
        // suite with `--test-threads=1` (see ci.yml) so the process is
        // effectively single-threaded at the fork point; even so, the child
        // stays strictly async-signal-safe as defense-in-depth: only raw
        // syscalls, no allocation (the very hazard `fork`'s safety note
        // describes). `exit_group` ends it without running any destructors.
        let (r, w) = fd::pipe2(0).expect("pipe2");
        match unsafe { fork() }.expect("fork") {
            0 => {
                let _ = fd::write(w, b"K");
                exit_group(7);
            }
            pid => {
                fd::close(w).expect("close w");
                let mut buf = [0u8; 1];
                let n = fd::read(r, &mut buf).expect("read");
                assert_eq!(&buf[..n], b"K");
                fd::close(r).expect("close r");

                let (wpid, status) = wait::waitpid(pid, 0).expect("waitpid");
                assert_eq!(wpid, pid);
                assert!(wait::wifexited(status));
                assert_eq!(wait::wexitstatus(status), 7);
            }
        }
    }

    #[test]
    fn pidfd_open_self_yields_a_pollable_fd() {
        use crate::fd;

        let pidfd = pidfd_open(getpid(), 0).expect("pidfd_open(self)");
        assert!(pidfd >= 0);
        // Not readable while the process is alive: a poll with a zero
        // timeout must time out (0 fds ready), not report POLLIN.
        let mut fds = [fd::PollFd {
            fd: pidfd,
            events: fd::POLLIN,
            revents: 0,
        }];
        let n = fd::poll(&mut fds, 0).expect("poll");
        assert_eq!(n, 0);
        fd::close(pidfd).expect("close pidfd");
    }

    #[test]
    fn pidfd_open_refuses_a_dead_pid() {
        // Fork a child, wait for it to exit, then confirm the kernel
        // refuses to open a pidfd for the now-gone pid — ESRCH, not a
        // silent success on a recycled/nonexistent identity.
        use crate::wait;

        let child = match unsafe { fork() }.expect("fork") {
            0 => exit_group(0),
            pid => pid,
        };
        wait::waitpid(child, 0).expect("waitpid");
        assert_eq!(pidfd_open(child, 0), Err(Errno::ESRCH));
    }

    #[test]
    fn pidfd_send_signal_kills_the_referenced_process() {
        use crate::fd;
        use crate::signal::SIGKILL;
        use crate::wait;

        // A long-lived child that just blocks; `pidfd_send_signal` reaches
        // it by pidfd, not by re-deriving/guessing its pid.
        let child = match unsafe { fork() }.expect("fork") {
            0 => loop {
                let req = crate::time::Timespec::from_millis(60_000);
                let _ = crate::time::nanosleep(&req, None);
            },
            pid => pid,
        };
        let pidfd = pidfd_open(child, 0).expect("pidfd_open");

        pidfd_send_signal(pidfd, SIGKILL).expect("pidfd_send_signal");

        let (wpid, status) = wait::waitpid(child, 0).expect("waitpid");
        assert_eq!(wpid, child);
        assert!(wait::wifsignaled(status));
        assert_eq!(wait::wtermsig(status), SIGKILL);

        fd::close(pidfd).expect("close pidfd");
    }

    #[test]
    fn pidfd_send_signal_bad_fd_is_ebadf() {
        assert_eq!(pidfd_send_signal(-1, 0), Err(Errno::EBADF));
    }

    #[test]
    fn fork_with_pidfd_returns_atomic_pidfd() {
        use crate::fd;
        use crate::wait;

        // CI runs single-threaded, and the child only issues raw syscalls;
        // see `fork_with_pidfd`'s safety note.
        //
        // clone3 (Linux 5.3+) is young enough that some restricted sandboxes
        // deny it outright via seccomp instead of reporting ENOSYS -- glibc
        // itself treats any failure of its own first clone3 call as "not
        // usable here" and permanently falls back to legacy `clone()` for
        // the rest of the process, precisely because denial errnos vary
        // across environments. Mirror that instead of hard-failing: this
        // crate's CI runs on unrestricted `ubuntu-latest` VMs where clone3
        // is available, so the skip path exists for exotic sandboxes only.
        let (pid, pidfd) = match unsafe { fork_with_pidfd() } {
            Ok(v) => v,
            Err(e) => {
                eprintln!(
                    "fork_with_pidfd_returns_atomic_pidfd: clone3 unavailable \
                     in this environment ({e:?}); skipping"
                );
                return;
            }
        };
        if pid == 0 {
            // Child: the pidfd field is never written back here.
            assert_eq!(pidfd, -1);
            exit_group(11);
        }

        assert!(pid > 0);
        assert!(pidfd >= 0);

        // The pidfd is usable exactly like one from `pidfd_open`: pollable,
        // and a valid target for `waitid(P_PIDFD, ...)`.
        let mut fds = [fd::PollFd {
            fd: pidfd,
            events: fd::POLLIN,
            revents: 0,
        }];
        let n = fd::poll(&mut fds, 5000).expect("poll");
        assert_eq!(n, 1);

        let info = wait::waitid(wait::P_PIDFD, pidfd, wait::WEXITED).expect("waitid via pidfd");
        assert_eq!(info.si_pid, pid);
        assert_eq!(info.si_code, wait::CLD_EXITED);
        assert_eq!(info.si_status, 11);

        fd::close(pidfd).expect("close pidfd");
    }

    #[test]
    fn set_and_get_pdeathsig_roundtrip() {
        use crate::signal::SIGKILL;
        use crate::wait;

        // Isolated in a forked child: this mutates real per-thread kernel
        // state, and a fresh child always starts with pdeathsig cleared
        // regardless of the parent's setting (prctl(2): "cleared for the
        // child of a fork(2)"), so there's nothing to restore either way.
        match unsafe { fork() }.expect("fork") {
            0 => {
                let ok = get_pdeathsig() == Ok(0)
                    && set_pdeathsig(SIGKILL).is_ok()
                    && get_pdeathsig() == Ok(SIGKILL)
                    && set_pdeathsig(0).is_ok()
                    && get_pdeathsig() == Ok(0);
                exit_group(if ok { 0 } else { 1 });
            }
            pid => {
                let (_, status) = wait::waitpid(pid, 0).expect("waitpid");
                assert!(wait::wifexited(status));
                assert_eq!(wait::wexitstatus(status), 0);
            }
        }
    }

    #[test]
    fn set_pdeathsig_kills_child_when_parent_exits() {
        use crate::fd;
        use crate::signal::SIGKILL;
        use crate::wait;

        // A three-generation dance, entirely to test something that only
        // manifests when a *specific parent* exits:
        //   test process -- fork --> intermediate -- fork --> grandchild
        // The grandchild arms pdeathsig against the intermediate, then the
        // intermediate exits immediately. If pdeathsig fires, the kernel
        // kills the grandchild right away; a pipe held open only by the
        // intermediate and the grandchild reports that via EOF once both
        // exit (the test process closes its own write-end copy up front).
        let (r, w) = fd::pipe2(0).expect("pipe2");
        // Handshake pipe: without it, the intermediate can exit before the
        // grandchild's prctl call has actually run, in which case
        // pdeathsig never arms and the grandchild just survives as an
        // ordinary orphan -- a real, observed flake (roughly 1 in 5 runs),
        // not a fluke. The grandchild writes a byte only after
        // set_pdeathsig succeeds; the intermediate blocks reading it before
        // exiting, so the intermediate's death is guaranteed to happen
        // strictly after pdeathsig is armed.
        let (sync_r, sync_w) = fd::pipe2(0).expect("pipe2 sync");

        match unsafe { fork() }.expect("fork") {
            0 => {
                // Intermediate.
                match unsafe { fork() }.expect("fork") {
                    0 => {
                        // Grandchild: arm pdeathsig against the intermediate,
                        // confirm it over the handshake pipe, then wait to
                        // be killed. The sleep is a fallback bound, not the
                        // mechanism under test -- if pdeathsig regresses,
                        // this keeps the suite from hanging instead of
                        // masking the regression (the poll timeout below is
                        // well under this).
                        let _ = set_pdeathsig(SIGKILL);
                        let _ = fd::write(sync_w, b"x");
                        crate::time::nanosleep(&crate::time::Timespec::from_millis(2000), None)
                            .ok();
                        exit_group(0);
                    }
                    _ => {
                        // Wait for the grandchild's handshake before
                        // exiting, so pdeathsig is guaranteed armed first.
                        let mut byte = [0u8; 1];
                        let _ = fd::read(sync_r, &mut byte);
                        exit_group(0);
                    }
                }
            }
            intermediate_pid => {
                fd::close(w).expect("close w");
                fd::close(sync_r).expect("close sync_r");
                fd::close(sync_w).expect("close sync_w");
                let (_, status) = wait::waitpid(intermediate_pid, 0).expect("waitpid intermediate");
                assert!(wait::wifexited(status));

                // A working pdeathsig kills the grandchild within
                // milliseconds of the intermediate exiting; a deadline far
                // under its own 2s fallback turns a regression into a
                // timeout here instead of a false pass.
                let mut fds = [fd::PollFd::new(r, fd::POLLIN)];
                let n = fd::poll(&mut fds, 500).expect("poll");
                assert_eq!(
                    n, 1,
                    "grandchild did not die promptly -- pdeathsig did not fire"
                );
                assert!(fds[0].is_hup());

                fd::close(r).expect("close r");
            }
        }
    }

    #[test]
    fn process_vm_readv_reads_a_childs_memory() {
        use crate::{fd, wait};

        // The child parks a known buffer on its stack, reports its address
        // back over a pipe, then blocks on a second pipe so the buffer stays
        // alive and at a stable address while the parent reads it out from
        // the outside -- no ptrace attach, just process_vm_readv against a
        // running child, matching this crate's own privileged dev sandbox
        // (root has CAP_SYS_PTRACE over its own children regardless of Yama
        // ptrace_scope).
        let (addr_r, addr_w) = fd::pipe2(0).expect("pipe2 addr");
        let (block_r, block_w) = fd::pipe2(0).expect("pipe2 block");

        match unsafe { fork() }.expect("fork") {
            0 => {
                fd::close(addr_r).ok();
                fd::close(block_w).ok();

                let buf: [u8; 5] = *b"hello";
                let addr = buf.as_ptr() as usize;
                fd::write_all(addr_w, &addr.to_ne_bytes()).expect("write addr");
                fd::close(addr_w).ok();

                // Blocks until the parent closes/writes block_w, keeping
                // `buf` alive and at this address in the meantime.
                let mut byte = [0u8; 1];
                let _ = fd::read(block_r, &mut byte);
                core::hint::black_box(&buf);
                exit_group(0);
            }
            pid => {
                fd::close(addr_w).ok();
                fd::close(block_r).ok();

                let mut addr_bytes = [0u8; core::mem::size_of::<usize>()];
                fd::read(addr_r, &mut addr_bytes).expect("read addr");
                fd::close(addr_r).ok();
                let addr = usize::from_ne_bytes(addr_bytes);

                let mut local = [0u8; 5];
                match process_vm_readv(
                    pid,
                    &mut [IoSliceMut::new(&mut local)],
                    &[RemoteIoVec::new(addr, 5)],
                ) {
                    Ok(n) => {
                        assert_eq!(n, 5);
                        assert_eq!(&local, b"hello");
                    }
                    // qemu-user's syscall translation layer does not
                    // implement process_vm_readv (confirmed empirically:
                    // succeeds natively on x86_64, ENOSYS only under
                    // aarch64-via-qemu-user) -- a gap in the emulator used
                    // by this crate's own CI, not real aarch64 hardware
                    // behavior, so tolerate it here rather than fail an
                    // emulated run over an untestable environment gap.
                    Err(Errno::ENOSYS) => {}
                    Err(e) => panic!("unexpected process_vm_readv error: {e:?}"),
                }

                fd::close(block_w).ok();
                let (_, status) = wait::waitpid(pid, 0).expect("waitpid");
                assert!(wait::wifexited(status));
            }
        }
    }

    #[test]
    fn process_vm_writev_writes_into_a_childs_memory() {
        use crate::{fd, wait};

        let (addr_r, addr_w) = fd::pipe2(0).expect("pipe2 addr");
        let (block_r, block_w) = fd::pipe2(0).expect("pipe2 block");
        let (result_r, result_w) = fd::pipe2(0).expect("pipe2 result");

        match unsafe { fork() }.expect("fork") {
            0 => {
                fd::close(addr_r).ok();
                fd::close(block_w).ok();
                fd::close(result_r).ok();

                let mut buf = [0u8; 5];
                let addr = buf.as_mut_ptr() as usize;
                fd::write_all(addr_w, &addr.to_ne_bytes()).expect("write addr");
                fd::close(addr_w).ok();

                // Blocks until the parent's process_vm_writev has landed.
                let mut byte = [0u8; 1];
                let _ = fd::read(block_r, &mut byte);

                fd::write_all(result_w, &buf).expect("write result");
                fd::close(result_w).ok();
                exit_group(0);
            }
            pid => {
                fd::close(addr_w).ok();
                fd::close(block_r).ok();
                fd::close(result_w).ok();

                let mut addr_bytes = [0u8; core::mem::size_of::<usize>()];
                fd::read(addr_r, &mut addr_bytes).expect("read addr");
                fd::close(addr_r).ok();
                let addr = usize::from_ne_bytes(addr_bytes);

                let writev_result =
                    process_vm_writev(pid, &[IoSlice::new(b"world")], &[RemoteIoVec::new(addr, 5)]);

                fd::close(block_w).ok();

                let mut result = [0u8; 5];
                fd::read(result_r, &mut result).expect("read result");
                fd::close(result_r).ok();

                match writev_result {
                    Ok(n) => {
                        assert_eq!(n, 5);
                        assert_eq!(&result, b"world");
                    }
                    // See the qemu-user note in
                    // process_vm_readv_reads_a_childs_memory: the child's
                    // buffer stays untouched when the syscall itself is
                    // unimplemented by the emulator.
                    Err(Errno::ENOSYS) => assert_eq!(result, [0u8; 5]),
                    Err(e) => panic!("unexpected process_vm_writev error: {e:?}"),
                }

                let (_, status) = wait::waitpid(pid, 0).expect("waitpid");
                assert!(wait::wifexited(status));
            }
        }
    }

    #[test]
    fn process_vm_readv_bad_pid_is_esrch() {
        let mut local = [0u8; 5];
        match process_vm_readv(
            i32::MAX,
            &mut [IoSliceMut::new(&mut local)],
            &[RemoteIoVec::new(0x1000, 5)],
        ) {
            Err(Errno::ESRCH) => {}
            // See the qemu-user note in
            // process_vm_readv_reads_a_childs_memory.
            Err(Errno::ENOSYS) => {}
            other => panic!("expected ESRCH (or ENOSYS under qemu-user), got {other:?}"),
        }
    }
}
