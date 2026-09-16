//! aarch64 Linux syscall stubs and syscall numbers.
//!
//! Calling convention: syscall number in `x8`; args in `x0`..`x5`; result in
//! `x0`. The `svc #0` instruction traps to the kernel, which preserves every
//! register except `x0` (and the condition flags), so no clobbers beyond the
//! `x0` result are required and `preserves_flags` is sound. As on x86_64 we
//! never claim `nomem`: syscalls read/write caller memory through pointers.
//!
//! Numbers come from the generic syscall table (`<asm-generic/unistd.h>`) that
//! aarch64 uses; several differ from x86_64's, and aarch64 lacks `poll`,
//! `dup2`, and the legacy `select`/`open` entirely (callers use `ppoll` and
//! `dup3`). No `rt_sigreturn` restorer is needed: the kernel points signal
//! returns at its vDSO `__kernel_rt_sigreturn`.

use core::arch::asm;

/// aarch64 syscall numbers used by this crate (from `<asm-generic/unistd.h>`).
#[allow(missing_docs)] // each constant's name is its documentation.
pub mod nr {
    pub const GETCWD: usize = 17;
    pub const DUP: usize = 23;
    pub const DUP3: usize = 24;
    pub const FCNTL: usize = 25;
    pub const IOCTL: usize = 29;
    pub const FACCESSAT: usize = 48;
    pub const FCHMODAT: usize = 53;
    pub const FCHOWNAT: usize = 54;
    pub const UTIMENSAT: usize = 88;
    pub const CHDIR: usize = 49;
    pub const FCHDIR: usize = 50;
    pub const OPENAT: usize = 56;
    pub const LSEEK: usize = 62;
    pub const WRITE: usize = 64;
    pub const CLOSE: usize = 57;
    pub const PIPE2: usize = 59;
    pub const MKDIRAT: usize = 34;
    pub const UNLINKAT: usize = 35;
    pub const SYMLINKAT: usize = 36;
    pub const LINKAT: usize = 37;
    pub const READ: usize = 63;
    pub const READLINKAT: usize = 78;
    pub const PPOLL: usize = 73;
    pub const EXIT_GROUP: usize = 94;
    pub const NANOSLEEP: usize = 101;
    pub const CLOCK_NANOSLEEP: usize = 115;
    pub const CLOCK_GETTIME: usize = 113;
    pub const TIMERFD_CREATE: usize = 85;
    pub const TIMERFD_SETTIME: usize = 86;
    pub const TIMERFD_GETTIME: usize = 87;
    pub const PREAD64: usize = 67;
    pub const PWRITE64: usize = 68;
    pub const READV: usize = 65;
    pub const WRITEV: usize = 66;
    pub const FTRUNCATE: usize = 46;
    pub const WAITID: usize = 95;
    pub const KILL: usize = 129;
    pub const RT_SIGSUSPEND: usize = 133;
    pub const RT_SIGACTION: usize = 134;
    pub const RT_SIGPROCMASK: usize = 135;
    pub const RT_SIGPENDING: usize = 136;
    pub const RT_SIGQUEUEINFO: usize = 138;
    pub const SIGNALFD4: usize = 74;
    pub const SETPGID: usize = 154;
    pub const GETPGID: usize = 155;
    pub const GETSID: usize = 156;
    pub const SETSID: usize = 157;
    pub const UMASK: usize = 166;
    pub const GETPID: usize = 172;
    pub const GETPPID: usize = 173;
    pub const GETUID: usize = 174;
    pub const GETEUID: usize = 175;
    pub const GETGID: usize = 176;
    pub const GETEGID: usize = 177;
    pub const GETGROUPS: usize = 158;
    pub const SETUID: usize = 146;
    pub const SETGID: usize = 144;
    pub const SETGROUPS: usize = 159;
    pub const SETRESUID: usize = 147;
    pub const SETRESGID: usize = 149;
    pub const GETRESUID: usize = 148;
    pub const GETRESGID: usize = 150;
    pub const SETPRIORITY: usize = 140;
    pub const GETPRIORITY: usize = 141;
    pub const PRCTL: usize = 167;
    pub const UNAME: usize = 160;
    pub const CLONE: usize = 220;
    pub const EXECVE: usize = 221;
    pub const RENAMEAT2: usize = 276;
    pub const MEMFD_CREATE: usize = 279;
    pub const GETRANDOM: usize = 278;
    pub const WAIT4: usize = 260;
    pub const GETRUSAGE: usize = 165;
    pub const PRLIMIT64: usize = 261;
    pub const STATX: usize = 291;
    pub const EXECVEAT: usize = 281;
    pub const GETDENTS64: usize = 61;
    pub const PIDFD_OPEN: usize = 434;
    pub const CLONE3: usize = 435;
    pub const PIDFD_SEND_SIGNAL: usize = 424;
    pub const MUNMAP: usize = 215;
    pub const MMAP: usize = 222;
    pub const MPROTECT: usize = 226;
    pub const MSYNC: usize = 227;
    pub const MLOCK: usize = 228;
    pub const MUNLOCK: usize = 229;
    pub const MADVISE: usize = 233;
    pub const SOCKET: usize = 198;
    pub const BIND: usize = 200;
    pub const LISTEN: usize = 201;
    pub const ACCEPT: usize = 202;
    pub const CONNECT: usize = 203;
    pub const SENDTO: usize = 206;
    pub const RECVFROM: usize = 207;
    pub const SHUTDOWN: usize = 210;
    pub const ACCEPT4: usize = 242;
    pub const GETSOCKNAME: usize = 204;
    pub const GETPEERNAME: usize = 205;
    pub const TRUNCATE: usize = 45;
    pub const GETTID: usize = 178;
    pub const CLOSE_RANGE: usize = 436;
    pub const FSYNC: usize = 82;
    pub const FDATASYNC: usize = 83;
    pub const CLOCK_GETRES: usize = 114;
    pub const PREADV: usize = 69;
    pub const PWRITEV: usize = 70;
    pub const EVENTFD2: usize = 19;
    pub const EPOLL_CREATE1: usize = 20;
    pub const EPOLL_CTL: usize = 21;
    pub const EPOLL_PWAIT: usize = 22;
    pub const INOTIFY_INIT1: usize = 26;
    pub const INOTIFY_ADD_WATCH: usize = 27;
    pub const INOTIFY_RM_WATCH: usize = 28;
    pub const SYSINFO: usize = 179;
    pub const PROCESS_VM_READV: usize = 270;
    pub const PROCESS_VM_WRITEV: usize = 271;
    pub const PTRACE: usize = 117;
}

/// Issue syscall `n` with no arguments; returns the raw `x0` result.
#[inline]
pub unsafe fn syscall0(n: usize) -> usize {
    let ret: usize;
    unsafe {
        asm!(
            "svc #0",
            in("x8") n,
            out("x0") ret,
            options(nostack, preserves_flags),
        );
    }
    ret
}

/// Issue syscall `n` with 1 argument; returns the raw `x0` result.
#[inline]
pub unsafe fn syscall1(n: usize, a1: usize) -> usize {
    let ret: usize;
    unsafe {
        asm!(
            "svc #0",
            in("x8") n,
            inlateout("x0") a1 => ret,
            options(nostack, preserves_flags),
        );
    }
    ret
}

/// Issue syscall `n` with 2 arguments; returns the raw `x0` result.
#[inline]
pub unsafe fn syscall2(n: usize, a1: usize, a2: usize) -> usize {
    let ret: usize;
    unsafe {
        asm!(
            "svc #0",
            in("x8") n,
            inlateout("x0") a1 => ret,
            in("x1") a2,
            options(nostack, preserves_flags),
        );
    }
    ret
}

/// Issue syscall `n` with 3 arguments; returns the raw `x0` result.
#[inline]
pub unsafe fn syscall3(n: usize, a1: usize, a2: usize, a3: usize) -> usize {
    let ret: usize;
    unsafe {
        asm!(
            "svc #0",
            in("x8") n,
            inlateout("x0") a1 => ret,
            in("x1") a2,
            in("x2") a3,
            options(nostack, preserves_flags),
        );
    }
    ret
}

/// Issue syscall `n` with 4 arguments; returns the raw `x0` result.
#[inline]
pub unsafe fn syscall4(n: usize, a1: usize, a2: usize, a3: usize, a4: usize) -> usize {
    let ret: usize;
    unsafe {
        asm!(
            "svc #0",
            in("x8") n,
            inlateout("x0") a1 => ret,
            in("x1") a2,
            in("x2") a3,
            in("x3") a4,
            options(nostack, preserves_flags),
        );
    }
    ret
}

/// Issue syscall `n` with 5 arguments; returns the raw `x0` result.
#[inline]
pub unsafe fn syscall5(n: usize, a1: usize, a2: usize, a3: usize, a4: usize, a5: usize) -> usize {
    let ret: usize;
    unsafe {
        asm!(
            "svc #0",
            in("x8") n,
            inlateout("x0") a1 => ret,
            in("x1") a2,
            in("x2") a3,
            in("x3") a4,
            in("x4") a5,
            options(nostack, preserves_flags),
        );
    }
    ret
}

/// Issue syscall `n` with 6 arguments; returns the raw `x0` result.
#[inline]
pub unsafe fn syscall6(
    n: usize,
    a1: usize,
    a2: usize,
    a3: usize,
    a4: usize,
    a5: usize,
    a6: usize,
) -> usize {
    let ret: usize;
    unsafe {
        asm!(
            "svc #0",
            in("x8") n,
            inlateout("x0") a1 => ret,
            in("x1") a2,
            in("x2") a3,
            in("x3") a4,
            in("x4") a5,
            in("x5") a6,
            options(nostack, preserves_flags),
        );
    }
    ret
}

/// `clone(2)` with `CLONE_VFORK|CLONE_VM` (`flags`), immediately followed
/// in the child by `execve(2)` on `path`/`argv`/`envp` — entirely within
/// this one asm block, so the child executes not one instruction of
/// compiler-generated Rust/C code of its own.
///
/// This is deliberate, and not just a style choice: a `clone(CLONE_VM)`
/// child shares actual memory with the parent (no copy-on-write, unlike
/// plain `fork`), so any ordinary Rust code running in the child after the
/// syscall returns is unsound in two ways a bare `cbnz`/`svc` sequence
/// avoids. First, the child would need to *return* through the same call
/// frames the parent's own continuation resumes through — frames that live
/// on whatever stack address `clone` used, which for a freshly provided
/// child stack has none of the call/return bookkeeping already-resuming
/// code depends on, and for a shared stack (`stack == 0`, as used here) is
/// fine for returning but reintroduces the second problem: the compiler
/// freely reuses a stack slot between "this local is only live in the
/// child branch" and "this local is only live in the parent's
/// continuation" — a correct optimization for code that, as far as the
/// compiler can see, executes at most one of those branches per call, but
/// wrong here because `CLONE_VM` really does let both branches' writes
/// land in the same physical memory, at genuinely different times. Doing
/// only raw syscalls, entirely in registers, sidesteps both: nothing is
/// ever pushed onto the (unused, since `stack == 0`) child stack, and
/// there is no Rust-level local for the optimizer to alias.
///
/// On `execve` failure the child calls `exit_group(127)`, matching the
/// shell "command not found" convention — the caller distinguishes "ran
/// and exited 127 itself" from "exec failed" the same way any fork+exec
/// caller already must, there being no channel back to the parent that
/// wouldn't reintroduce the hazard above.
///
/// Returns the child's pid to the parent, or the raw `-errno` `clone`
/// itself failed with (same `-4095..=-1` convention as every other raw
/// syscall in this crate). Never returns in the child.
///
/// # Safety
/// `path`, `argv`, `envp` must satisfy the same contract as `execve`:
/// `path` a valid, NUL-terminated C string, `argv`/`envp` valid
/// NUL-terminated arrays of valid C-string pointers, all live for the
/// duration of the call.
#[inline]
pub unsafe fn vfork_execve(
    flags: usize,
    path: *const u8,
    argv: *const *const u8,
    envp: *const *const u8,
) -> usize {
    let ret: usize;
    unsafe {
        asm!(
            "svc #0",                // clone(flags, stack = 0, 0, 0, 0)
            "cbnz x0, 2f",            // parent: x0 already holds the result
            "mov x0, x9",
            "mov x1, x10",
            "mov x2, x11",
            "mov x8, {execve_nr}",
            "svc #0",                 // execve(path, argv, envp)
            "mov x8, {exit_nr}",
            "mov x0, #127",
            "svc #0",                 // exit_group(127): execve failed
            "brk #0",                 // unreachable: exit_group never returns
            "2:",
            in("x8") nr::CLONE,
            inlateout("x0") flags => ret,
            in("x1") 0usize,
            in("x2") 0usize,
            in("x3") 0usize,
            in("x4") 0usize,
            in("x9") path,
            in("x10") argv,
            in("x11") envp,
            execve_nr = const nr::EXECVE,
            exit_nr = const nr::EXIT_GROUP,
            options(nostack),
        );
    }
    ret
}

/// Like [`vfork_execve`], but first applies `redirects` (a
/// `redirects_len`-element array of `(oldfd, newfd)` pairs, 8 bytes each)
/// as `dup2`-shaped fd redirections in the child, before `execve` — still
/// entirely within one asm block, for the same reasons [`vfork_execve`]'s
/// own doc comment explains at length; this is that same technique with a
/// loop added, not a different one. Uses `dup3` (aarch64 has no legacy
/// `dup2` syscall at all — see this module's own header doc comment).
///
/// A pair with `oldfd == newfd` is skipped rather than calling `dup3` on
/// it: unlike `dup2(2)`, raw `dup3` has no no-op-on-equal case and returns
/// `EINVAL` for it instead (see `fd::dup2`'s own doc comment for the same
/// distinction this crate's higher-level `dup2` already handles). If a
/// redirect's `dup3` fails, the child calls `exit_group(126)` (the shell
/// convention for "found the command, but couldn't set it up to run")
/// without ever reaching `execve`; a failing `execve` itself still exits
/// `127` as in [`vfork_execve`], so a caller can tell the two failure
/// modes apart via the wait status.
///
/// Returns the child's pid to the parent, or the raw `-errno` `clone`
/// itself failed with. Never returns in the child.
///
/// # Safety
/// Same contract as [`vfork_execve`] for `path`/`argv`/`envp`, plus:
/// `redirects` must point to `redirects_len` valid, readable 8-byte
/// `(i32, i32)` records for the duration of the call.
#[inline]
pub unsafe fn vfork_execve_redirected(
    flags: usize,
    path: *const u8,
    argv: *const *const u8,
    envp: *const *const u8,
    redirects: *const u8,
    redirects_len: usize,
) -> usize {
    let ret: usize;
    unsafe {
        asm!(
            "svc #0",                     // clone(flags, stack = 0, 0, 0, 0)
            "cbnz x0, 3f",                 // parent: x0 already holds the result
            "cbz x13, 2f",                 // no redirects
            "mov x14, #0",                 // i = 0
            "1:",
            "add x15, x12, x14, lsl #3",   // x15 = &redirects[i]
            "ldr w0, [x15]",               // oldfd
            "ldr w1, [x15, #4]",           // newfd
            "cmp w0, w1",
            "b.eq 6f",                     // oldfd == newfd: dup3 has no
                                            // no-op case for this, skip
            "mov x8, {dup3_nr}",
            "mov x2, #0",                  // flags = 0
            "svc #0",
            "cmp x0, #0",
            "b.lt 4f",                     // dup3 failed -> exit_group(126)
            "6:",
            "add x14, x14, #1",
            "cmp x14, x13",
            "b.lt 1b",
            "2:",                          // redirects applied (or none)
            "mov x0, x9",
            "mov x1, x10",
            "mov x2, x11",
            "mov x8, {execve_nr}",
            "svc #0",                      // execve(path, argv, envp)
            "mov x8, {exit_nr}",
            "mov x0, #127",
            "svc #0",                      // exit_group(127): execve failed
            "brk #0",
            "4:",
            "mov x8, {exit_nr}",
            "mov x0, #126",
            "svc #0",                      // exit_group(126): a redirect failed
            "brk #0",
            "3:",
            in("x8") nr::CLONE,
            inlateout("x0") flags => ret,
            in("x1") 0usize,
            in("x2") 0usize,
            in("x3") 0usize,
            in("x4") 0usize,
            in("x9") path,
            in("x10") argv,
            in("x11") envp,
            in("x12") redirects,
            in("x13") redirects_len,
            out("x14") _,
            out("x15") _,
            dup3_nr = const nr::DUP3,
            execve_nr = const nr::EXECVE,
            exit_nr = const nr::EXIT_GROUP,
            options(nostack),
        );
    }
    ret
}
