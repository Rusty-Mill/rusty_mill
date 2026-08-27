//! Signal handling via `rt_sigaction`, plus the x86_64 `SA_RESTORER`
//! trampoline.
//!
//! This mirrors glibc's BSD-style `signal(3)`: the handler is persistent
//! (not reset after delivery) and installed with `SA_RESTART`, so slow
//! syscalls resume rather than failing with `EINTR`. The delivered signal is
//! blocked for the duration of its own handler by the kernel default.
//!
//! ## The x86_64 restorer problem
//!
//! On x86_64 the kernel has **no** default signal-return path: when a handler
//! returns, control resumes at whatever address `sa_restorer` points to, and
//! that code must invoke `SYS_rt_sigreturn` to unwind the kernel-pushed signal
//! frame. glibc supplies this trampoline; since we do not link glibc's
//! `sigaction`, we must supply our own. `sigreturn_trampoline` is a naked
//! function doing exactly `mov rax, RT_SIGRETURN; syscall`, and every
//! [`signal`] install sets `SA_RESTORER` and points `sa_restorer` at it.
//!
//! aarch64 (Phase 5) needs no restorer: its kernel installs a default vDSO
//! trampoline, so `SA_RESTORER` is neither set nor required there.

use crate::arch::nr;
use crate::arch::{from_ret, from_ret_i32, syscall2, syscall3, syscall4, Errno};

/// A signal handler: [`SIG_DFL`], [`SIG_IGN`], or a function pointer
/// (`extern "C" fn(i32)` cast to `usize`). Modelled as `usize` to match
/// libc's `sighandler_t`, which must also represent the two sentinel values.
pub type Sighandler = usize;

/// Default disposition for the signal.
pub const SIG_DFL: Sighandler = 0;
/// Ignore the signal.
pub const SIG_IGN: Sighandler = 1;

// Standard Linux signal numbers (x86_64 / asm-generic).
/// Hangup.
pub const SIGHUP: i32 = 1;
/// Interrupt (typically Ctrl-C).
pub const SIGINT: i32 = 2;
/// Quit (typically Ctrl-\).
pub const SIGQUIT: i32 = 3;
/// Illegal instruction.
pub const SIGILL: i32 = 4;
/// Trace/breakpoint trap.
pub const SIGTRAP: i32 = 5;
/// Abort.
pub const SIGABRT: i32 = 6;
/// Bus error.
pub const SIGBUS: i32 = 7;
/// Floating-point exception.
pub const SIGFPE: i32 = 8;
/// Kill (cannot be caught or ignored).
pub const SIGKILL: i32 = 9;
/// User-defined signal 1.
pub const SIGUSR1: i32 = 10;
/// Invalid memory reference.
pub const SIGSEGV: i32 = 11;
/// User-defined signal 2.
pub const SIGUSR2: i32 = 12;
/// Write to a pipe with no readers.
pub const SIGPIPE: i32 = 13;
/// Alarm clock.
pub const SIGALRM: i32 = 14;
/// Termination request.
pub const SIGTERM: i32 = 15;
/// Stack fault (unused on most systems).
pub const SIGSTKFLT: i32 = 16;
/// Child stopped or terminated.
pub const SIGCHLD: i32 = 17;
/// Continue if stopped.
pub const SIGCONT: i32 = 18;
/// Stop (cannot be caught or ignored).
pub const SIGSTOP: i32 = 19;
/// Terminal stop (typically Ctrl-Z).
pub const SIGTSTP: i32 = 20;
/// Background read from terminal.
pub const SIGTTIN: i32 = 21;
/// Background write to terminal.
pub const SIGTTOU: i32 = 22;
/// Urgent condition on socket.
pub const SIGURG: i32 = 23;
/// CPU time limit exceeded.
pub const SIGXCPU: i32 = 24;
/// File size limit exceeded.
pub const SIGXFSZ: i32 = 25;
/// Virtual alarm clock.
pub const SIGVTALRM: i32 = 26;
/// Profiling timer expired.
pub const SIGPROF: i32 = 27;
/// Window resize.
pub const SIGWINCH: i32 = 28;
/// I/O now possible.
pub const SIGIO: i32 = 29;
/// Power failure.
pub const SIGPWR: i32 = 30;
/// Bad system call.
pub const SIGSYS: i32 = 31;

/// `sa_flags`: for `SIGCHLD`, do not receive it when children merely stop or
/// continue (only on death). Job-control shells that reap stops via `wait` set
/// this to avoid duplicate notifications.
pub const SA_NOCLDSTOP: u64 = 0x0000_0001;
/// `sa_flags`: for `SIGCHLD`, do not turn terminated children into zombies.
pub const SA_NOCLDWAIT: u64 = 0x0000_0002;
/// `sa_flags`: deliver a three-argument `sigaction`-style handler (the handler
/// must have the matching `siginfo` signature).
pub const SA_SIGINFO: u64 = 0x0000_0004;
/// `sa_flags`: resume slow syscalls instead of failing with `EINTR`.
pub const SA_RESTART: u64 = 0x1000_0000;
/// `sa_flags`: do not block the signal within its own handler.
pub const SA_NODEFER: u64 = 0x4000_0000;
/// `sa_flags`: reset the disposition to the default on the first delivery.
pub const SA_RESETHAND: u64 = 0x8000_0000;
/// `sa_flags`: `sa_restorer` is valid and should be used (x86_64 requires it;
/// aarch64 has no restorer and never sets this).
#[cfg(target_arch = "x86_64")]
const SA_RESTORER: u64 = 0x0400_0000;

/// Size in bytes of the kernel `sigset_t` we pass to `rt_sigaction`. The
/// kernel checks this against its own expectation; on 64-bit Linux it is 8.
const SIGSETSIZE: usize = 8;

/// The kernel's `struct sigaction` (a.k.a. `struct kernel_sigaction`) for
/// x86_64. Note the field order is the **kernel's** — handler, flags,
/// restorer, mask — which differs from glibc's userspace layout.
#[repr(C)]
struct KernelSigaction {
    sa_handler: usize,
    sa_flags: u64,
    sa_restorer: usize,
    /// Signals blocked during the handler; a single 64-bit word here.
    sa_mask: u64,
}

const _: () = assert!(core::mem::size_of::<KernelSigaction>() == 32);
const _: () = assert!(core::mem::offset_of!(KernelSigaction, sa_handler) == 0);
const _: () = assert!(core::mem::offset_of!(KernelSigaction, sa_flags) == 8);
const _: () = assert!(core::mem::offset_of!(KernelSigaction, sa_restorer) == 16);
const _: () = assert!(core::mem::offset_of!(KernelSigaction, sa_mask) == 24);

/// The x86_64 signal-return trampoline: `sa_restorer` points here so that,
/// when a handler returns, this code issues `SYS_rt_sigreturn` to restore the
/// pre-signal context. Naked and never inlined so its address is a stable,
/// valid entry point. aarch64 needs no equivalent (kernel vDSO handles it).
#[cfg(target_arch = "x86_64")]
#[unsafe(naked)]
unsafe extern "C" fn sigreturn_trampoline() {
    core::arch::naked_asm!(
        "mov rax, {nr}",
        "syscall",
        nr = const nr::RT_SIGRETURN,
    )
}

/// `sigprocmask`/`rt_sigprocmask` `how`: add the signals in `set` to the mask.
pub const SIG_BLOCK: i32 = 0;
/// `how`: remove the signals in `set` from the mask.
pub const SIG_UNBLOCK: i32 = 1;
/// `how`: set the mask to exactly `set`.
pub const SIG_SETMASK: i32 = 2;

/// Build the single-signal bit for `sig` in a [`sigprocmask`] mask.
///
/// The kernel `sigset_t` numbers signals from 1, so signal `n` is bit `n - 1`.
#[inline]
pub const fn sigmask(sig: i32) -> u64 {
    1u64 << (sig - 1) as u64
}

/// Examine and change the calling thread's blocked-signal mask via
/// `rt_sigprocmask`, returning the **previous** mask.
///
/// `how` is [`SIG_BLOCK`], [`SIG_UNBLOCK`], or [`SIG_SETMASK`]; `set` is a mask
/// built from [`sigmask`] (OR several together). To read the current mask
/// without changing it, pass `SIG_BLOCK` with `set == 0`.
///
/// A job-control shell uses this to block `SIGCHLD` around the fork/record
/// critical section, then restore the saved mask.
pub fn sigprocmask(how: i32, set: u64) -> Result<u64, Errno> {
    let mut old: u64 = 0;
    // rt_sigprocmask(how, &set, &mut old, sigsetsize).
    // SAFETY: `set`/`old` are valid 8-byte kernel sigsets; `sigsetsize` matches.
    let ret = unsafe {
        syscall4(
            nr::RT_SIGPROCMASK,
            how as usize,
            &set as *const u64 as usize,
            &mut old as *mut u64 as usize,
            SIGSETSIZE,
        )
    };
    from_ret(ret)?;
    Ok(old)
}

/// Shared `rt_sigaction` installer. `flags` is the caller's `SA_*` set; the
/// x86_64 restorer bits are always ORed in here (mandatory on that arch).
///
/// # Safety
/// Same contract as [`signal`]/[`sigaction`] on `handler`.
unsafe fn install(sig: i32, handler: Sighandler, flags: u64) -> Result<Sighandler, Errno> {
    // x86_64 must supply a restorer via SA_RESTORER; aarch64's kernel provides
    // one through the vDSO, so it sets neither the flag nor sa_restorer.
    #[cfg(target_arch = "x86_64")]
    let (sa_flags, sa_restorer) = (
        flags | SA_RESTORER,
        sigreturn_trampoline as *const () as usize,
    );
    #[cfg(target_arch = "aarch64")]
    let (sa_flags, sa_restorer) = (flags, 0usize);

    let new = KernelSigaction {
        sa_handler: handler,
        sa_flags,
        sa_restorer,
        sa_mask: 0,
    };
    let mut old = KernelSigaction {
        sa_handler: 0,
        sa_flags: 0,
        sa_restorer: 0,
        sa_mask: 0,
    };
    // rt_sigaction(sig, &new, &old, sigsetsize).
    // SAFETY: both pointers are valid, correctly-sized kernel sigaction
    // structs; `sigsetsize` matches the kernel's 64-bit sigset.
    let ret = unsafe {
        syscall4(
            nr::RT_SIGACTION,
            sig as usize,
            &new as *const KernelSigaction as usize,
            &mut old as *mut KernelSigaction as usize,
            SIGSETSIZE,
        )
    };
    from_ret(ret)?;
    Ok(old.sa_handler)
}

/// Install `handler` for signal `sig`, returning the previous handler.
///
/// The handler is persistent and installed with [`SA_RESTART`] (glibc BSD
/// `signal(3)` semantics), so slow syscalls resume rather than failing with
/// `EINTR`. `handler` is [`SIG_DFL`], [`SIG_IGN`], or an `extern "C" fn(i32)`
/// cast to `usize`. For control over the flags (e.g. [`SA_NOCLDSTOP`], or
/// omitting `SA_RESTART`), use [`sigaction`].
///
/// # Safety
/// Installing an arbitrary handler is inherently unsafe: the handler runs
/// asynchronously in signal context, where only async-signal-safe operations
/// are permitted, and `handler` (when not a sentinel) must be a valid
/// `extern "C" fn(i32)` pointer that lives at least until it is replaced.
#[inline]
pub unsafe fn signal(sig: i32, handler: Sighandler) -> Result<Sighandler, Errno> {
    // SAFETY: forwarded to the caller's `handler` contract.
    unsafe { install(sig, handler, SA_RESTART) }
}

/// Install `handler` for signal `sig` with an explicit `flags` set (an OR of
/// `SA_*` constants), returning the previous handler.
///
/// This is [`signal`] without the hardcoded [`SA_RESTART`]: pass `SA_RESTART`
/// yourself if you want restarting, or omit it so a blocked syscall breaks with
/// `EINTR` (e.g. a `SIGINT` handler that must interrupt a blocking `read`). The
/// x86_64 return trampoline is still installed automatically. `SA_SIGINFO` is
/// accepted but the caller is then responsible for providing a handler with the
/// three-argument signature.
///
/// # Safety
/// Same contract as [`signal`].
#[inline]
pub unsafe fn sigaction(sig: i32, handler: Sighandler, flags: u64) -> Result<Sighandler, Errno> {
    // SAFETY: forwarded to the caller's `handler` contract.
    unsafe { install(sig, handler, flags) }
}

/// Return the set of signals that are pending (raised while blocked) for the
/// calling thread, as a mask of [`sigmask`] bits.
pub fn sigpending() -> Result<u64, Errno> {
    let mut set: u64 = 0;
    // rt_sigpending(&mut set, sigsetsize).
    // SAFETY: `set` is a valid 8-byte kernel sigset the kernel writes.
    let ret = unsafe { syscall2(nr::RT_SIGPENDING, &mut set as *mut u64 as usize, SIGSETSIZE) };
    from_ret(ret)?;
    Ok(set)
}

/// Atomically replace the thread's signal mask with `mask` and wait until a
/// signal is delivered, then restore the previous mask. Always returns
/// `EINTR` (the only way it returns).
///
/// The race-free "wait for a signal" primitive: block `SIGCHLD` with
/// [`sigprocmask`], check your job state, then `sigsuspend` with `SIGCHLD`
/// unblocked so a `SIGCHLD` arriving in between is not lost.
pub fn sigsuspend(mask: u64) -> Errno {
    // rt_sigsuspend(&mask, sigsetsize) — returns -EINTR on success.
    // SAFETY: `mask` is a valid 8-byte kernel sigset the kernel only reads.
    let ret = unsafe { syscall2(nr::RT_SIGSUSPEND, &mask as *const u64 as usize, SIGSETSIZE) };
    match from_ret(ret) {
        Ok(_) => Errno(0),
        Err(e) => e,
    }
}

// --- signalfd(2) -------------------------------------------------------

/// `signalfd(2)` flag: return a non-blocking descriptor.
pub const SFD_NONBLOCK: i32 = 0o0004000;
/// `signalfd(2)` flag: set close-on-exec on the returned descriptor.
pub const SFD_CLOEXEC: i32 = 0o2000000;

/// Per-signal detail delivered by reading a `signalfd` (kernel
/// `struct signalfd_siginfo`), fixed at 128 bytes on every architecture
/// (the kernel pads it out deliberately so a `read(2)` never needs a compat
/// path).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SignalfdSiginfo {
    /// The signal number ([`SIGCHLD`], [`SIGINT`], ...).
    pub ssi_signo: u32,
    /// `errno` associated with this signal, if any (rarely set on Linux).
    pub ssi_errno: i32,
    /// Signal code (e.g. `CLD_EXITED` for a `SIGCHLD`; see `<bits/siginfo.h>`
    /// for the full set, not reproduced by this crate).
    pub ssi_code: i32,
    /// Sending process's pid, for signals that carry one (e.g. `SIGCHLD`,
    /// `kill`-delivered signals).
    pub ssi_pid: u32,
    /// Sending process's real uid.
    pub ssi_uid: u32,
    /// Source file descriptor, for `SIGIO`/`SIGPOLL`.
    pub ssi_fd: i32,
    /// POSIX timer id, for timer-generated signals.
    pub ssi_tid: u32,
    /// Band event, for `SIGIO`/`SIGPOLL`.
    pub ssi_band: u32,
    /// POSIX timer overrun count.
    pub ssi_overrun: u32,
    /// Trap number, for hardware-fault signals.
    pub ssi_trapno: u32,
    /// Exit status or signal, for `SIGCHLD`.
    pub ssi_status: i32,
    /// Integer value, for `sigqueue`-delivered signals.
    pub ssi_int: i32,
    /// Pointer value, for `sigqueue`-delivered signals.
    pub ssi_ptr: u64,
    /// User CPU time consumed, for `SIGCHLD` (in clock ticks).
    pub ssi_utime: u64,
    /// System CPU time consumed, for `SIGCHLD` (in clock ticks).
    pub ssi_stime: u64,
    /// Faulting address, for hardware-fault signals.
    pub ssi_addr: u64,
    /// Least significant bit of the faulting address, for some hardware
    /// faults.
    pub ssi_addr_lsb: u16,
    __pad2: u16,
    /// Triggering system call number, for a seccomp-generated `SIGSYS`.
    pub ssi_syscall: i32,
    /// Triggering system call's instruction pointer, for a seccomp-generated
    /// `SIGSYS`.
    pub ssi_call_addr: u64,
    /// Architecture of the triggering system call, for a seccomp-generated
    /// `SIGSYS`.
    pub ssi_arch: u32,
    __pad: [u8; 28],
}

const _: () = assert!(core::mem::size_of::<SignalfdSiginfo>() == 128);
const _: () = assert!(core::mem::offset_of!(SignalfdSiginfo, ssi_pid) == 12);
const _: () = assert!(core::mem::offset_of!(SignalfdSiginfo, ssi_ptr) == 48);
const _: () = assert!(core::mem::offset_of!(SignalfdSiginfo, ssi_addr_lsb) == 80);
const _: () = assert!(core::mem::offset_of!(SignalfdSiginfo, ssi_arch) == 96);

/// Create (or, passing an existing signalfd as `fd`, reconfigure) a file
/// descriptor that becomes readable whenever a signal in `mask` (built from
/// [`sigmask`], OR several together) is pending for the caller. Pass
/// `fd = -1` to create a new descriptor; `flags` is an OR of
/// [`SFD_NONBLOCK`]/[`SFD_CLOEXEC`].
///
/// The signals in `mask` must also be blocked via [`sigprocmask`] (typically
/// just before this call, with the same mask) for delivery to route here
/// instead of the default disposition or a handler installed via
/// [`sigaction`]/[`signal`] -- `signalfd` does not itself change the mask.
///
/// Reading the returned fd with [`crate::fd::read`] (sized to whole
/// [`SignalfdSiginfo`] records) atomically dequeues one pending signal per
/// record and runs no handler at all: everything happens in ordinary
/// control flow, composing with [`crate::fd::poll`] the same way any other
/// readiness-based fd does, with none of the async-signal-safety
/// constraints a real handler imposes on what can run in response. This is
/// this crate's recommended path for the asynchronous signals a
/// long-running program (e.g. a job-control shell's `SIGCHLD`/`SIGINT`/
/// `SIGWINCH` handling, or a `trap`-style dispatcher) needs to react to; see
/// [ADR-0002](https://github.com/baileyrd/rusty_libc/blob/main/docs/adr/0002-signalfd-as-primary-event-driven-signal-path.md)
/// for the full reasoning, including where `sigaction` remains the right
/// tool (synchronous hardware-fault signals like `SIGSEGV`).
///
/// The returned fd is closed with [`crate::fd::close`] like any other fd.
pub fn signalfd(fd: i32, mask: u64, flags: i32) -> Result<i32, Errno> {
    // signalfd4(fd, &mask, sigsetsize, flags).
    // SAFETY: `mask` is a valid, exclusively-borrowed 8-byte kernel sigset
    // the kernel only reads.
    let ret = unsafe {
        syscall4(
            nr::SIGNALFD4,
            fd as usize,
            &mask as *const u64 as usize,
            SIGSETSIZE,
            flags as usize,
        )
    };
    from_ret_i32(ret)
}

// --- sigqueue(3) / rt_sigqueueinfo(2) -----------------------------------

/// `si_code` value marking a signal as application-queued via `sigqueue`
/// (as opposed to kernel-generated, e.g. `SI_KERNEL`, or plain `kill`,
/// `SI_USER`). The kernel rejects `rt_sigqueueinfo` calls that claim any
/// `si_code >= 0` from a normal (non-kernel) caller, so this is the only
/// value [`sigqueue`] ever sends.
const SI_QUEUE: i32 = -1;

/// The subset of the kernel's `siginfo_t` that `rt_sigqueueinfo` reads:
/// the three-int header every `siginfo_t` variant shares, followed by the
/// `_rt` union member (`_pid`, `_uid`, `_sigval`) `SI_QUEUE`-flavored
/// signals use. Fixed at 128 bytes to match the kernel's own `siginfo_t`
/// size, the same convention [`crate::wait::Siginfo`] and
/// [`SignalfdSiginfo`] follow for their own (differently-shaped) views
/// into the same union.
#[repr(C)]
struct SigqueueInfo {
    si_signo: i32,
    si_errno: i32,
    si_code: i32,
    __pad0: i32,
    si_pid: i32,
    si_uid: u32,
    /// `sigval_t`: the caller-chosen payload, delivered back verbatim as
    /// [`SignalfdSiginfo::ssi_ptr`]/`ssi_int`'s low bits to a
    /// [`signalfd`] reader, or as the handler's third argument for a
    /// `SA_SIGINFO` [`sigaction`]. A plain integer fits in the low bits;
    /// a pointer-sized value fills the whole field.
    si_value: usize,
    __pad: [u8; 96],
}

const _: () = assert!(core::mem::size_of::<SigqueueInfo>() == 128);
const _: () = assert!(core::mem::offset_of!(SigqueueInfo, si_pid) == 16);
const _: () = assert!(core::mem::offset_of!(SigqueueInfo, si_value) == 24);

/// Send signal `sig` to process `pid`, attaching `value` as a payload a
/// receiver decodes via [`signalfd`] (`SignalfdSiginfo::ssi_ptr`/`ssi_int`)
/// or a `SA_SIGINFO` handler installed with [`sigaction`] — the other half
/// of the payload delivery [`SignalfdSiginfo`] already exposes on the
/// receiving side: this crate had no way to *originate* one. Ordinary
/// [`kill`](crate::process::kill) can only send a bare signal number.
///
/// `value` is delivered as a raw `sigval_t` (an untyped `usize`): pass a
/// plain integer, or a pointer the receiver understands the shape of (the
/// two processes must agree on what it means out of band — the kernel
/// only carries it, it doesn't interpret it).
pub fn sigqueue(pid: i32, sig: i32, value: usize) -> Result<(), Errno> {
    let info = SigqueueInfo {
        si_signo: sig,
        si_errno: 0,
        si_code: SI_QUEUE,
        __pad0: 0,
        si_pid: crate::process::getpid(),
        si_uid: crate::process::getuid(),
        si_value: value,
        __pad: [0; 96],
    };
    // rt_sigqueueinfo(pid, sig, &info).
    // SAFETY: `info` is a valid, fully-initialized `siginfo_t`-shaped
    // struct the kernel only reads.
    let ret = unsafe {
        syscall3(
            nr::RT_SIGQUEUEINFO,
            pid as usize,
            sig as usize,
            &info as *const SigqueueInfo as usize,
        )
    };
    from_ret(ret).map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::arch::{syscall0, syscall3};
    use core::sync::atomic::{AtomicU64, Ordering};

    // gettid/tgkill numbers differ per arch and aren't part of the crate's
    // public surface, so define them locally for the tests.
    #[cfg(target_arch = "x86_64")]
    const NR_GETTID: usize = 186;
    #[cfg(target_arch = "x86_64")]
    const NR_TGKILL: usize = 234;
    #[cfg(target_arch = "aarch64")]
    const NR_GETTID: usize = 178;
    #[cfg(target_arch = "aarch64")]
    const NR_TGKILL: usize = 131;

    // Direct the signal at the *current thread* so delivery is synchronous
    // (it completes before the raising syscall returns), giving exact counts
    // regardless of what the rest of the test harness's threads are doing.
    fn gettid() -> i32 {
        // SAFETY: gettid takes no args and never fails.
        unsafe { syscall0(NR_GETTID) as i32 }
    }
    fn tgkill(tgid: i32, tid: i32, sig: i32) {
        // SAFETY: plain integer arguments.
        unsafe { syscall3(NR_TGKILL, tgid as usize, tid as usize, sig as usize) };
    }

    static COUNT: AtomicU64 = AtomicU64::new(0);
    extern "C" fn counting_handler(_sig: i32) {
        COUNT.fetch_add(1, Ordering::SeqCst);
    }

    #[test]
    fn install_returns_previous_handler() {
        // Fresh disposition for SIGURG (unused here) is the default.
        let prev = unsafe { signal(SIGURG, SIG_IGN) }.expect("install IGN");
        assert_eq!(prev, SIG_DFL);
        // Reinstalling reports the SIG_IGN we just set.
        let prev = unsafe { signal(SIGURG, SIG_DFL) }.expect("install DFL");
        assert_eq!(prev, SIG_IGN);
    }

    #[test]
    fn invalid_signal_is_einval() {
        // SIGKILL cannot be caught: rt_sigaction rejects it with EINVAL.
        assert_eq!(unsafe { signal(SIGKILL, SIG_IGN) }, Err(Errno(22)));
    }

    // Signal dispositions and `COUNT` are process-global, and `cargo test`
    // runs tests in parallel threads, so all signal *delivery* assertions live
    // in this one sequential test to avoid cross-test races on that shared
    // state. (The disposition-only tests above touch neither `COUNT` nor these
    // signals, so they are safe to run concurrently.)
    #[test]
    fn delivery_stress_and_restorer_and_ignore() {
        let (pid, tid) = (crate::process::getpid(), gettid());

        // --- Storm: the crux of Phase 3. If the SA_RESTORER trampoline were
        // wrong, returning from the handler would fault. Delivering the signal
        // thousands of times to *this* thread (synchronous delivery) and
        // surviving with an exact count proves the trampoline and that no
        // deliveries are missed. ---
        COUNT.store(0, Ordering::SeqCst);
        let prev =
            unsafe { signal(SIGUSR1, counting_handler as *const () as usize) }.expect("install");
        const N: u64 = 5000;
        for _ in 0..N {
            tgkill(pid, tid, SIGUSR1);
        }
        assert_eq!(COUNT.load(Ordering::SeqCst), N);
        unsafe { signal(SIGUSR1, prev) }.expect("restore SIGUSR1");

        // --- Ignore: a SIG_IGN'd signal must not reach the handler. ---
        COUNT.store(0, Ordering::SeqCst);
        let prev = unsafe { signal(SIGUSR2, SIG_IGN) }.expect("ignore SIGUSR2");
        tgkill(pid, tid, SIGUSR2);
        assert_eq!(COUNT.load(Ordering::SeqCst), 0);
        unsafe { signal(SIGUSR2, prev) }.expect("restore SIGUSR2");

        // --- Mask: block SIGUSR1, raise it (stays pending, handler not run),
        // then restore the mask and confirm the pending signal is delivered
        // exactly once. rt_sigprocmask masks per-thread, and tgkill targets
        // this thread, so the accounting is exact. ---
        COUNT.store(0, Ordering::SeqCst);
        let prev = unsafe { signal(SIGUSR1, counting_handler as *const () as usize) }
            .expect("install for mask");
        let old_mask = sigprocmask(SIG_BLOCK, sigmask(SIGUSR1)).expect("block SIGUSR1");
        tgkill(pid, tid, SIGUSR1);
        // Blocked: pending, not yet delivered.
        assert_eq!(COUNT.load(Ordering::SeqCst), 0);
        // Restoring the mask unblocks SIGUSR1; the pending signal fires before
        // rt_sigprocmask returns.
        sigprocmask(SIG_SETMASK, old_mask).expect("restore mask");
        assert_eq!(COUNT.load(Ordering::SeqCst), 1);
        unsafe { signal(SIGUSR1, prev) }.expect("restore SIGUSR1 after mask");

        // --- sigpending: a blocked, raised signal shows up as pending. ---
        let prev = unsafe { signal(SIGUSR1, counting_handler as *const () as usize) }
            .expect("install for pending");
        let old_mask = sigprocmask(SIG_BLOCK, sigmask(SIGUSR1)).expect("block SIGUSR1");
        assert_eq!(sigpending().expect("sigpending") & sigmask(SIGUSR1), 0);
        tgkill(pid, tid, SIGUSR1);
        assert_ne!(sigpending().expect("sigpending") & sigmask(SIGUSR1), 0);

        // --- sigsuspend: unblock SIGUSR1 only for the wait; the already-pending
        // signal is delivered immediately and sigsuspend returns EINTR. ---
        COUNT.store(0, Ordering::SeqCst);
        // Suspend with an empty mask (nothing blocked) so the pending SIGUSR1
        // fires at once.
        assert_eq!(sigsuspend(0), Errno::EINTR);
        assert_eq!(COUNT.load(Ordering::SeqCst), 1);
        // sigsuspend restored the pre-call mask (SIGUSR1 still blocked); clean up.
        sigprocmask(SIG_SETMASK, old_mask).expect("restore mask after suspend");
        unsafe { signal(SIGUSR1, prev) }.expect("restore SIGUSR1 after pending");

        // --- sigaction: install without SA_RESTART and confirm delivery still
        // works (behavioural parity with signal(); the flag only affects
        // syscall restart, not delivery). ---
        COUNT.store(0, Ordering::SeqCst);
        let prev = unsafe {
            sigaction(
                SIGUSR1,
                counting_handler as *const () as usize,
                SA_NOCLDSTOP,
            )
        }
        .expect("sigaction install");
        tgkill(pid, tid, SIGUSR1);
        assert_eq!(COUNT.load(Ordering::SeqCst), 1);
        unsafe { signal(SIGUSR1, prev) }.expect("restore SIGUSR1 after sigaction");
    }

    // signalfd tests use SIGPWR, untouched by every other test in this file
    // (all of which live on SIGUSR1/SIGUSR2/SIGURG), so they need no
    // coordination with `delivery_stress_and_restorer_and_ignore`'s shared,
    // process-wide disposition state.

    #[test]
    fn signalfd_reports_pending_signal_and_dequeues_it() {
        use crate::fd;

        let pid = crate::process::getpid();
        let old_mask = sigprocmask(SIG_BLOCK, sigmask(SIGPWR)).expect("block SIGPWR");

        let sfd = signalfd(-1, sigmask(SIGPWR), 0).expect("signalfd");

        // Not readable before the signal is raised.
        let mut fds = [fd::PollFd {
            fd: sfd,
            events: fd::POLLIN,
            revents: 0,
        }];
        assert_eq!(fd::poll(&mut fds, 0).expect("poll before"), 0);

        // `sigprocmask` blocks per-thread, and `cargo test` runs each test on
        // its own thread, so a process-wide `kill` here would land on
        // whichever other thread doesn't have SIGPWR blocked -- its default
        // disposition is to terminate the process. `tgkill` at this exact
        // thread (the same pattern the delivery/mask tests above use)
        // targets only the thread that actually blocked it.
        tgkill(pid, gettid(), SIGPWR);

        let n = fd::poll(&mut fds, 1000).expect("poll after");
        assert_eq!(n, 1, "signalfd did not become readable");
        assert!(fds[0].is_readable());

        let mut info = SignalfdSiginfo::default();
        // SAFETY: `info` is a valid, exclusively-borrowed, plain-data
        // `SignalfdSiginfo` of exactly the size the kernel writes.
        let buf = unsafe {
            core::slice::from_raw_parts_mut(
                &mut info as *mut SignalfdSiginfo as *mut u8,
                core::mem::size_of::<SignalfdSiginfo>(),
            )
        };
        let read_n = fd::read(sfd, buf).expect("read signalfd_siginfo");
        assert_eq!(read_n, core::mem::size_of::<SignalfdSiginfo>());
        assert_eq!(info.ssi_signo, SIGPWR as u32);
        assert_eq!(info.ssi_pid, pid as u32);

        // The read dequeued it: no longer pending.
        assert_eq!(sigpending().expect("sigpending") & sigmask(SIGPWR), 0);

        fd::close(sfd).expect("close signalfd");
        sigprocmask(SIG_SETMASK, old_mask).expect("restore mask");
    }

    #[test]
    fn signalfd_nonblock_read_is_eagain_when_nothing_pending() {
        use crate::fd;

        let old_mask = sigprocmask(SIG_BLOCK, sigmask(SIGPWR)).expect("block SIGPWR");
        let sfd = signalfd(-1, sigmask(SIGPWR), SFD_NONBLOCK).expect("signalfd nonblock");

        let mut buf = [0u8; core::mem::size_of::<SignalfdSiginfo>()];
        assert_eq!(fd::read(sfd, &mut buf), Err(Errno::EAGAIN));

        fd::close(sfd).expect("close signalfd");
        sigprocmask(SIG_SETMASK, old_mask).expect("restore mask");
    }

    #[test]
    fn sigqueue_delivers_signal_and_payload_via_signalfd() {
        use crate::fd;
        use crate::process;
        use crate::wait;

        // rt_sigqueueinfo (like plain kill) is process-directed, not
        // thread-directed, and cargo test runs each test on its own
        // thread -- targeting the whole process from here risks the exact
        // hazard `signalfd_reports_pending_signal_and_dequeues_it` above
        // had to route around with `tgkill`: some sibling test's thread
        // that hasn't blocked SIGPWR could catch it under its default
        // (terminate) disposition. Isolating this in a fresh,
        // single-threaded forked child sidesteps it entirely -- there,
        // "the whole process" and "this one thread" are the same thing.
        match unsafe { process::fork() }.expect("fork") {
            0 => {
                // The child exits right after this block regardless of
                // outcome, so unlike the process-wide tests earlier in
                // this file, there is nothing to restore the mask for.
                if sigprocmask(SIG_BLOCK, sigmask(SIGPWR)).is_err() {
                    process::exit_group(1);
                }
                let sfd = match signalfd(-1, sigmask(SIGPWR), 0) {
                    Ok(fd) => fd,
                    Err(_) => process::exit_group(2),
                };

                const PAYLOAD: usize = 0xdead_beef;
                if sigqueue(process::getpid(), SIGPWR, PAYLOAD).is_err() {
                    process::exit_group(3);
                }

                let mut fds = [fd::PollFd {
                    fd: sfd,
                    events: fd::POLLIN,
                    revents: 0,
                }];
                if fd::poll(&mut fds, 1000) != Ok(1) {
                    process::exit_group(4);
                }

                let mut info = SignalfdSiginfo::default();
                // SAFETY: `info` is a valid, exclusively-borrowed, plain-
                // data `SignalfdSiginfo` of exactly the size the kernel
                // writes.
                let buf = unsafe {
                    core::slice::from_raw_parts_mut(
                        &mut info as *mut SignalfdSiginfo as *mut u8,
                        core::mem::size_of::<SignalfdSiginfo>(),
                    )
                };
                if fd::read(sfd, buf) != Ok(core::mem::size_of::<SignalfdSiginfo>()) {
                    process::exit_group(5);
                }
                if info.ssi_signo != SIGPWR as u32 {
                    process::exit_group(6);
                }
                if info.ssi_pid != process::getpid() as u32 {
                    process::exit_group(7);
                }
                if info.ssi_ptr != PAYLOAD as u64 {
                    process::exit_group(8);
                }

                process::exit_group(0);
            }
            pid => {
                let (_, status) = wait::waitpid(pid, 0).expect("waitpid");
                assert!(wait::wifexited(status), "child did not exit normally");
                assert_eq!(wait::wexitstatus(status), 0, "child assertion failed");
            }
        }
    }

    #[test]
    fn sigqueue_missing_pid_is_esrch() {
        // No process has this pid: rt_sigqueueinfo reports ESRCH, matching
        // `process::getpgid_missing_process_is_esrch`'s own sentinel pid.
        assert_eq!(sigqueue(0x3fff_ffff, 0, 0), Err(Errno::ESRCH));
    }
}
