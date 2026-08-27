//! `ptrace(2)`: tracer/tracee lifecycle and portable register access.
//!
//! Scoped by [ADR-0004](https://github.com/baileyrd/rusty_libc/blob/main/docs/adr/0004-ptrace-scope-and-shape.md)
//! (see it for the full reasoning and alternatives considered). Unlike every
//! other syscall this crate wraps, `ptrace`'s raw argument meaning is
//! entirely request-dependent, so instead of a single 1:1 passthrough this
//! module exposes one typed function per request, matching every other
//! module's convention.
//!
//! **Out of scope**, on purpose: `PTRACE_PEEKTEXT`/`PEEKDATA`/`POKETEXT`/
//! `POKEDATA` (strictly subsumed by [`crate::process::process_vm_readv`]/
//! [`crate::process::process_vm_writev`], which read/write arbitrary-length
//! ranges without an attach), the legacy `PTRACE_ATTACH` (superseded by
//! [`ptrace_seize`]) and `PTRACE_GETREGS`/`SETREGS` (superseded by
//! [`ptrace_getregset`]/[`ptrace_setregset`]).
//!
//! # Thread affinity
//!
//! The kernel requires the **specific thread** that attached (via
//! [`ptrace_seize`]) or that received [`ptrace_traceme`]'s effect to be the
//! one issuing every further `ptrace` call for that tracee — not just any
//! thread in the tracer process. Calling these from a different thread than
//! the one that established the relationship fails with `ESRCH`, the same
//! error a genuinely nonexistent tracee produces.

use crate::arch::nr;
use crate::arch::{from_ret, syscall4, Errno};
use crate::signal::SIGTRAP;

const PTRACE_TRACEME: i32 = 0;
const PTRACE_CONT: i32 = 7;
const PTRACE_SINGLESTEP: i32 = 9;
const PTRACE_DETACH: i32 = 17;
const PTRACE_SYSCALL: i32 = 24;
const PTRACE_SETOPTIONS: i32 = 0x4200;
const PTRACE_GETREGSET: i32 = 0x4204;
const PTRACE_SETREGSET: i32 = 0x4205;
const PTRACE_SEIZE: i32 = 0x4206;

/// Core-dump note type identifying the general-purpose register set, passed
/// as the `addr` argument to [`ptrace_getregset`]/[`ptrace_setregset`]
/// (kernel `NT_PRSTATUS`, `include/uapi/linux/elf.h`).
const NT_PRSTATUS: i32 = 1;

/// `PTRACE_SETOPTIONS`/[`ptrace_seize`] option: kill the tracee if the
/// tracer exits, instead of leaving an orphaned, permanently-stopped
/// process behind.
pub const PTRACE_O_EXITKILL: i32 = 1 << 20;

/// `PTRACE_SETOPTIONS`/[`ptrace_seize`] option: tag syscall-stops with
/// `SIGTRAP | 0x80` instead of plain `SIGTRAP`, so [`is_syscall_stop`] can
/// tell one apart from a real `SIGTRAP` the tracee raised itself. Required
/// for [`ptrace_syscall`]-driven tracing to be reliable — without it, the
/// two stop kinds are indistinguishable.
pub const PTRACE_O_TRACESYSGOOD: i32 = 1;

/// Kernel `struct iovec`-shaped `{ base, len }` pair describing a local
/// buffer, for [`ptrace_getregset`]/[`ptrace_setregset`]'s `data` argument.
/// Not [`crate::fd::IoSlice`]/[`crate::fd::IoSliceMut`]: those don't expose
/// `len` for the kernel to write back (`PTRACE_GETREGSET` updates it to the
/// actual regset size), and this is a single buffer, never a slice of them.
#[repr(C)]
struct RegSetIoVec {
    base: usize,
    len: usize,
}

/// General-purpose registers (kernel `struct user_regs_struct` on x86_64,
/// `struct user_pt_regs` on aarch64 — genuinely different layouts, verified
/// against raw kernel source: `arch/x86/include/asm/user_64.h` and
/// `arch/arm64/include/uapi/asm/ptrace.h`), as read/written by
/// [`ptrace_getregset`]/[`ptrace_setregset`].
#[cfg(target_arch = "x86_64")]
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GpRegs {
    /// `r15`.
    pub r15: u64,
    /// `r14`.
    pub r14: u64,
    /// `r13`.
    pub r13: u64,
    /// `r12`.
    pub r12: u64,
    /// `rbp` (frame pointer, by convention).
    pub rbp: u64,
    /// `rbx`.
    pub rbx: u64,
    /// `r11`.
    pub r11: u64,
    /// `r10`.
    pub r10: u64,
    /// `r9`.
    pub r9: u64,
    /// `r8`.
    pub r8: u64,
    /// `rax` — the return value at an exit-stop; at a syscall-entry-stop
    /// this also holds the syscall number, but use [`Self::orig_rax`] for
    /// that, which stays reliable across both.
    pub rax: u64,
    /// `rcx`.
    pub rcx: u64,
    /// `rdx`.
    pub rdx: u64,
    /// `rsi`.
    pub rsi: u64,
    /// `rdi`.
    pub rdi: u64,
    /// The syscall number at a syscall-stop, reliably (unlike `rax`, which
    /// the kernel overwrites with the return value by the time an exit-stop
    /// is reported) regardless of entry vs. exit.
    pub orig_rax: u64,
    /// `rip`, the instruction pointer — see [`Self::instruction_pointer`].
    pub rip: u64,
    /// `cs` (code segment selector).
    pub cs: u64,
    /// `eflags` (the CPU flags register).
    pub eflags: u64,
    /// `rsp`, the stack pointer — see [`Self::stack_pointer`].
    pub rsp: u64,
    /// `ss` (stack segment selector).
    pub ss: u64,
    /// `fs_base` (the `%fs` segment's base address, used for
    /// thread-local storage).
    pub fs_base: u64,
    /// `gs_base` (the `%gs` segment's base address).
    pub gs_base: u64,
    /// `ds` (data segment selector; unused in the x86-64 flat model).
    pub ds: u64,
    /// `es` (extra segment selector; unused in the x86-64 flat model).
    pub es: u64,
    /// `fs` (segment selector; the base address is [`Self::fs_base`]).
    pub fs: u64,
    /// `gs` (segment selector; the base address is [`Self::gs_base`]).
    pub gs: u64,
}

#[cfg(target_arch = "x86_64")]
const _: () = assert!(core::mem::size_of::<GpRegs>() == 216);
#[cfg(target_arch = "x86_64")]
const _: () = assert!(core::mem::offset_of!(GpRegs, orig_rax) == 120);
#[cfg(target_arch = "x86_64")]
const _: () = assert!(core::mem::offset_of!(GpRegs, rip) == 128);
#[cfg(target_arch = "x86_64")]
const _: () = assert!(core::mem::offset_of!(GpRegs, rsp) == 152);

#[cfg(target_arch = "x86_64")]
impl GpRegs {
    /// The instruction pointer (`rip`).
    #[inline]
    pub fn instruction_pointer(&self) -> u64 {
        self.rip
    }
    /// The stack pointer (`rsp`).
    #[inline]
    pub fn stack_pointer(&self) -> u64 {
        self.rsp
    }
}

/// General-purpose registers (kernel `struct user_pt_regs` on aarch64 — see
/// [`GpRegs`]'s x86_64 doc for the layout-divergence background).
#[cfg(target_arch = "aarch64")]
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GpRegs {
    /// `x0`..`x30`. The syscall number lives in `regs[8]` (`x8`) at a
    /// syscall-stop — unlike x86_64's `rax`, `x8` is never overwritten by
    /// the return value (which lands in `regs[0]`/`x0` instead), so it
    /// needs no `orig_`-style shadow copy to stay reliable across entry vs.
    /// exit.
    pub regs: [u64; 31],
    /// The stack pointer — see [`Self::stack_pointer`].
    pub sp: u64,
    /// The program counter — see [`Self::instruction_pointer`].
    pub pc: u64,
    /// The saved processor state (`PSTATE`/`CPSR`-equivalent flags).
    pub pstate: u64,
}

#[cfg(target_arch = "aarch64")]
const _: () = assert!(core::mem::size_of::<GpRegs>() == 272);
#[cfg(target_arch = "aarch64")]
const _: () = assert!(core::mem::offset_of!(GpRegs, sp) == 248);
#[cfg(target_arch = "aarch64")]
const _: () = assert!(core::mem::offset_of!(GpRegs, pc) == 256);
#[cfg(target_arch = "aarch64")]
const _: () = assert!(core::mem::offset_of!(GpRegs, pstate) == 264);

#[cfg(target_arch = "aarch64")]
impl GpRegs {
    /// The instruction pointer (`pc`).
    #[inline]
    pub fn instruction_pointer(&self) -> u64 {
        self.pc
    }
    /// The stack pointer (`sp`).
    #[inline]
    pub fn stack_pointer(&self) -> u64 {
        self.sp
    }
}

/// Become a tracee of the calling process's parent: the **child** side of
/// the classic (non-[`ptrace_seize`]) attach handshake, called before
/// [`crate::process::execve`]/`execveat` or before self-signalling
/// (`kill(getpid(), SIGSTOP)`) to enter a deterministic first stop the
/// parent can wait on.
pub fn ptrace_traceme() -> Result<(), Errno> {
    // SAFETY: TRACEME ignores pid/addr/data; all zero.
    let ret = unsafe { syscall4(nr::PTRACE, PTRACE_TRACEME as usize, 0, 0, 0) };
    from_ret(ret).map(|_| ())
}

/// Attach to the already-running process `pid` as its tracer, **without**
/// stopping it (unlike the legacy `PTRACE_ATTACH`, which this crate omits —
/// `PTRACE_SEIZE` strictly supersedes it). `options` accepts
/// [`PTRACE_O_EXITKILL`] and the other `PTRACE_O_*` bits, set at seize time
/// instead of a separate [`ptrace_setoptions`] call.
pub fn ptrace_seize(pid: i32, options: i32) -> Result<(), Errno> {
    // SAFETY: SEIZE ignores addr; `data` carries the options bitmask.
    let ret = unsafe {
        syscall4(
            nr::PTRACE,
            PTRACE_SEIZE as usize,
            pid as usize,
            0,
            options as usize,
        )
    };
    from_ret(ret).map(|_| ())
}

/// Resume a stopped tracee `pid`, optionally re-injecting `signal` (`0` for
/// none). Requires `pid` to currently be in a ptrace-stop.
pub fn ptrace_cont(pid: i32, signal: i32) -> Result<(), Errno> {
    // SAFETY: CONT ignores addr; `data` carries the signal to re-inject.
    let ret = unsafe {
        syscall4(
            nr::PTRACE,
            PTRACE_CONT as usize,
            pid as usize,
            0,
            signal as usize,
        )
    };
    from_ret(ret).map(|_| ())
}

/// Resume a stopped tracee `pid` for exactly one instruction, then stop it
/// again, optionally re-injecting `signal` (`0` for none). Same shape and
/// preconditions as [`ptrace_cont`].
pub fn ptrace_singlestep(pid: i32, signal: i32) -> Result<(), Errno> {
    // SAFETY: SINGLESTEP ignores addr; `data` carries the signal to re-inject.
    let ret = unsafe {
        syscall4(
            nr::PTRACE,
            PTRACE_SINGLESTEP as usize,
            pid as usize,
            0,
            signal as usize,
        )
    };
    from_ret(ret).map(|_| ())
}

/// Resume a stopped tracee `pid` until the next syscall-entry-or-exit
/// boundary, then stop it again, optionally re-injecting `signal` (`0` for
/// none). Same shape and preconditions as [`ptrace_cont`] — the primitive an
/// `strace`-style tracer is built on.
///
/// Reliably telling a syscall-stop apart from an ordinary signal-stop (both
/// report as `SIGTRAP` otherwise) needs [`PTRACE_O_TRACESYSGOOD`] set first
/// via [`ptrace_setoptions`] (or [`ptrace_seize`]'s `options`); see
/// [`is_syscall_stop`].
///
/// At a syscall-stop, `pid`'s syscall number and (at entry) arguments or
/// (at exit) return value are readable via [`ptrace_getregset`] — x86_64's
/// `orig_rax` or aarch64's `regs[8]`/`x8` for the number, reliably across
/// both entry and exit, and the argument/return-value registers per the
/// platform's normal syscall calling convention. Neither this function nor
/// [`is_syscall_stop`] track *which* of entry or exit a given stop is —
/// `PTRACE_SYSCALL` stops strictly alternate entry/exit/entry/exit, so a
/// tracer driving its own `wait4` loop already has to track that sequencing
/// itself, the same way it must for any other multi-step wait protocol in
/// this crate.
pub fn ptrace_syscall(pid: i32, signal: i32) -> Result<(), Errno> {
    // SAFETY: SYSCALL ignores addr; `data` carries the signal to re-inject.
    let ret = unsafe {
        syscall4(
            nr::PTRACE,
            PTRACE_SYSCALL as usize,
            pid as usize,
            0,
            signal as usize,
        )
    };
    from_ret(ret).map(|_| ())
}

/// Resume and detach from tracee `pid`, optionally re-injecting `signal`
/// (`0` for none). `pid` stops being traced by this process; it keeps
/// running (or is reaped normally by an eventual [`crate::wait::waitpid`]
/// if it was a real child). Requires `pid` to currently be in a
/// ptrace-stop, same as [`ptrace_cont`].
pub fn ptrace_detach(pid: i32, signal: i32) -> Result<(), Errno> {
    // SAFETY: DETACH ignores addr; `data` carries the signal to re-inject.
    let ret = unsafe {
        syscall4(
            nr::PTRACE,
            PTRACE_DETACH as usize,
            pid as usize,
            0,
            signal as usize,
        )
    };
    from_ret(ret).map(|_| ())
}

/// Set tracing options for tracee `pid` (an OR of `PTRACE_O_*`, e.g.
/// [`PTRACE_O_EXITKILL`]).
pub fn ptrace_setoptions(pid: i32, options: i32) -> Result<(), Errno> {
    // SAFETY: SETOPTIONS ignores addr; `data` carries the options bitmask.
    let ret = unsafe {
        syscall4(
            nr::PTRACE,
            PTRACE_SETOPTIONS as usize,
            pid as usize,
            0,
            options as usize,
        )
    };
    from_ret(ret).map(|_| ())
}

/// Read tracee `pid`'s general-purpose registers. Requires `pid` to
/// currently be in a ptrace-stop.
pub fn ptrace_getregset(pid: i32) -> Result<GpRegs, Errno> {
    let mut regs = GpRegs::default();
    let mut iov = RegSetIoVec {
        base: &mut regs as *mut GpRegs as usize,
        len: core::mem::size_of::<GpRegs>(),
    };
    // SAFETY: `iov` exclusively borrows `regs` (via a raw pointer scoped to
    // this call) for the kernel to write into; `iov` itself is a valid,
    // exclusively borrowed `struct iovec`.
    let ret = unsafe {
        syscall4(
            nr::PTRACE,
            PTRACE_GETREGSET as usize,
            pid as usize,
            NT_PRSTATUS as usize,
            &mut iov as *mut RegSetIoVec as usize,
        )
    };
    from_ret(ret)?;
    Ok(regs)
}

/// Write tracee `pid`'s general-purpose registers. Requires `pid` to
/// currently be in a ptrace-stop.
pub fn ptrace_setregset(pid: i32, regs: &GpRegs) -> Result<(), Errno> {
    let mut iov = RegSetIoVec {
        base: regs as *const GpRegs as usize,
        len: core::mem::size_of::<GpRegs>(),
    };
    // SAFETY: `iov` describes `regs`, which the kernel only reads; `iov`
    // itself is a valid, exclusively borrowed `struct iovec`.
    let ret = unsafe {
        syscall4(
            nr::PTRACE,
            PTRACE_SETREGSET as usize,
            pid as usize,
            NT_PRSTATUS as usize,
            &mut iov as *mut RegSetIoVec as usize,
        )
    };
    from_ret(ret).map(|_| ())
}

/// True if `status` (as returned by [`crate::wait::waitpid`]) reports a
/// [`ptrace_syscall`]-driven syscall-stop, as opposed to an ordinary
/// signal-stop. Requires [`PTRACE_O_TRACESYSGOOD`] to have been set on the
/// tracee first (via [`ptrace_setoptions`] or [`ptrace_seize`]'s `options`)
/// — without it, a syscall-stop and a real `SIGTRAP` the tracee raised
/// itself both report as plain `SIGTRAP`, and this always returns `false`.
pub fn is_syscall_stop(status: i32) -> bool {
    crate::wait::wifstopped(status) && crate::wait::wstopsig(status) == (SIGTRAP | 0x80)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::signal::SIGSTOP;
    use crate::{fd, process, wait};
    use process::{exit_group, fork};

    #[test]
    fn traceme_stop_getregset_setregset_cont_roundtrip() {
        // Classic (non-SEIZE) attach: the child calls TRACEME, then
        // self-signals SIGSTOP to enter a deterministic first ptrace-stop
        // the parent can wait on -- unlike a real external SIGSTOP to a
        // seized tracee (a group-stop with its own persistence rules), a
        // classic tracee's SIGSTOP becomes an ordinary, fully resumable
        // ptrace signal-stop.
        // Sentinel exit code the child uses in place of a panic if TRACEME
        // itself fails -- panicking across a raw `fork()` inside the test
        // harness is its own hazard, so the child reports failure by exit
        // status instead and lets the parent decide what it means.
        const TRACEME_UNSUPPORTED: i32 = 90;

        match unsafe { fork() }.expect("fork") {
            0 => match ptrace_traceme() {
                Ok(()) => {
                    process::kill(process::getpid(), SIGSTOP).ok();
                    exit_group(0);
                }
                Err(_) => exit_group(TRACEME_UNSUPPORTED),
            },
            pid => {
                let (reaped, status) = wait::waitpid(pid, 0).expect("waitpid stop");
                assert_eq!(reaped, pid);

                if wait::wifexited(status) && wait::wexitstatus(status) == TRACEME_UNSUPPORTED {
                    // qemu-user's ptrace emulation does not support
                    // PTRACE_TRACEME here (confirmed empirically: works
                    // natively on x86_64, same as the process_vm_readv/
                    // writev ENOSYS gap already caught in #102) -- a
                    // limitation of the emulator this crate's own CI uses
                    // for aarch64, not real aarch64 hardware behavior.
                    return;
                }

                assert!(wait::wifstopped(status));
                assert_eq!(wait::wstopsig(status), SIGSTOP);

                let regs = ptrace_getregset(pid).expect("getregset");
                // The child is stopped mid-syscall (its own kill()), so the
                // instruction pointer is somewhere in valid code, never 0.
                assert_ne!(regs.instruction_pointer(), 0);
                assert_ne!(regs.stack_pointer(), 0);

                // Round-trip with no changes; must still succeed.
                ptrace_setregset(pid, &regs).expect("setregset");

                ptrace_setoptions(pid, PTRACE_O_EXITKILL).expect("setoptions");

                ptrace_cont(pid, 0).expect("cont");

                let (reaped, status) = wait::waitpid(pid, 0).expect("waitpid exit");
                assert_eq!(reaped, pid);
                assert!(wait::wifexited(status));
                assert_eq!(wait::wexitstatus(status), 0);
            }
        }
    }

    #[test]
    fn seize_attaches_to_a_running_child_without_stopping_it() {
        let (block_r, block_w) = fd::pipe2(0).unwrap();
        match unsafe { fork() }.expect("fork") {
            0 => {
                fd::close(block_w).ok();
                let mut byte = [0u8; 1];
                let _ = fd::read(block_r, &mut byte);
                exit_group(0);
            }
            pid => {
                fd::close(block_r).ok();
                match ptrace_seize(pid, 0) {
                    Ok(()) => {
                        // SEIZE attaches without stopping the tracee --
                        // confirm nothing reportable happened yet.
                        let (reaped, _status) =
                            wait::waitpid(pid, wait::WNOHANG).expect("waitpid nohang");
                        assert_eq!(reaped, 0, "SEIZE should not have stopped the tracee");
                    }
                    // See the qemu-user note in
                    // traceme_stop_getregset_setregset_cont_roundtrip.
                    Err(Errno::ENOSYS) => {}
                    Err(e) => panic!("unexpected seize error: {e:?}"),
                }

                fd::close(block_w).ok();
                let (reaped, status) = wait::waitpid(pid, 0).expect("waitpid exit");
                assert_eq!(reaped, pid);
                assert!(wait::wifexited(status));
            }
        }
    }

    #[test]
    fn getregset_on_an_untraced_pid_fails() {
        // This process is never a valid ptrace target for itself (a process
        // cannot trace itself), so this always fails regardless of tracer
        // state -- confirms the error path without needing a real tracee.
        assert!(ptrace_getregset(process::getpid()).is_err());
    }

    #[test]
    fn cont_on_a_pid_not_in_a_ptrace_stop_fails() {
        let (block_r, block_w) = fd::pipe2(0).unwrap();
        match unsafe { fork() }.expect("fork") {
            0 => {
                fd::close(block_w).ok();
                let mut byte = [0u8; 1];
                let _ = fd::read(block_r, &mut byte);
                exit_group(0);
            }
            pid => {
                fd::close(block_r).ok();
                // Never traced at all, so definitely not in a ptrace-stop.
                assert!(ptrace_cont(pid, 0).is_err());

                fd::close(block_w).ok();
                let (reaped, status) = wait::waitpid(pid, 0).expect("waitpid exit");
                assert_eq!(reaped, pid);
                assert!(wait::wifexited(status));
            }
        }
    }

    #[cfg(target_arch = "x86_64")]
    fn syscall_nr(regs: &GpRegs) -> u64 {
        regs.orig_rax
    }
    #[cfg(target_arch = "aarch64")]
    fn syscall_nr(regs: &GpRegs) -> u64 {
        regs.regs[8]
    }

    #[test]
    fn ptrace_syscall_reports_entry_and_exit_stops() {
        // Same TRACEME + self-SIGSTOP handshake as the lifecycle test, but
        // once stopped, drives the child through one full getpid() syscall
        // under PTRACE_SYSCALL instead of just letting it run free -- this
        // exercises the entry-stop/exit-stop pair ptrace_syscall produces,
        // and is_syscall_stop's SIGTRAP|0x80 disambiguation.
        const TRACEME_UNSUPPORTED: i32 = 90;

        match unsafe { fork() }.expect("fork") {
            0 => match ptrace_traceme() {
                Ok(()) => {
                    process::kill(process::getpid(), SIGSTOP).ok();
                    // The syscall the parent will observe entry/exit for.
                    let _ = process::getpid();
                    exit_group(0);
                }
                Err(_) => exit_group(TRACEME_UNSUPPORTED),
            },
            pid => {
                let (reaped, status) = wait::waitpid(pid, 0).expect("waitpid stop");
                assert_eq!(reaped, pid);

                if wait::wifexited(status) && wait::wexitstatus(status) == TRACEME_UNSUPPORTED {
                    // See the qemu-user note in
                    // traceme_stop_getregset_setregset_cont_roundtrip.
                    return;
                }
                assert!(wait::wifstopped(status));
                assert_eq!(wait::wstopsig(status), SIGSTOP);

                ptrace_setoptions(pid, PTRACE_O_TRACESYSGOOD).expect("setoptions");

                // Step through syscall-stops via PTRACE_SYSCALL until the
                // child's explicit getpid() call shows up. Exactly how many
                // stops occur between the self-SIGSTOP handshake and that
                // point isn't asserted -- the self-signal itself is a
                // syscall the tracee was still inside of when it stopped,
                // so whether its own exit surfaces as a distinct
                // syscall-stop first is a kernel-internal detail this test
                // doesn't need to pin down, only that a syscall-stop
                // reporting GETPID eventually does, exactly once, followed
                // immediately by its matching exit-stop.
                let mut steps = 0;
                let entry_regs = loop {
                    steps += 1;
                    assert!(steps <= 8, "did not observe a getpid() entry-stop in time");
                    ptrace_syscall(pid, 0).expect("ptrace_syscall");
                    let (reaped, status) = wait::waitpid(pid, 0).expect("waitpid stop");
                    assert_eq!(reaped, pid);
                    assert!(is_syscall_stop(status), "expected a syscall-stop");
                    let regs = ptrace_getregset(pid).expect("getregset");
                    if syscall_nr(&regs) == nr::GETPID as u64 {
                        break regs;
                    }
                };
                assert_eq!(syscall_nr(&entry_regs), nr::GETPID as u64);

                // Resume to that same syscall's exit.
                ptrace_syscall(pid, 0).expect("ptrace_syscall to exit");
                let (reaped, status) = wait::waitpid(pid, 0).expect("waitpid exit-stop");
                assert_eq!(reaped, pid);
                assert!(is_syscall_stop(status), "expected a syscall-exit-stop");
                let exit_regs = ptrace_getregset(pid).expect("getregset at exit");
                assert_eq!(syscall_nr(&exit_regs), nr::GETPID as u64);

                // Let it run free the rest of the way to its own exit_group.
                ptrace_cont(pid, 0).expect("cont");
                let (reaped, status) = wait::waitpid(pid, 0).expect("waitpid final exit");
                assert_eq!(reaped, pid);
                assert!(wait::wifexited(status));
                assert_eq!(wait::wexitstatus(status), 0);
            }
        }
    }

    #[test]
    fn is_syscall_stop_is_false_for_an_ordinary_signal_stop() {
        // A plain signal-stop (this test's own SIGSTOP handshake, with
        // TRACESYSGOOD never set) must not be misreported as a
        // syscall-stop -- the two share the same base SIGTRAP-or-not
        // ambiguity is_syscall_stop exists to resolve.
        const TRACEME_UNSUPPORTED: i32 = 90;

        match unsafe { fork() }.expect("fork") {
            0 => match ptrace_traceme() {
                Ok(()) => {
                    process::kill(process::getpid(), SIGSTOP).ok();
                    exit_group(0);
                }
                Err(_) => exit_group(TRACEME_UNSUPPORTED),
            },
            pid => {
                let (reaped, status) = wait::waitpid(pid, 0).expect("waitpid stop");
                assert_eq!(reaped, pid);

                if wait::wifexited(status) && wait::wexitstatus(status) == TRACEME_UNSUPPORTED {
                    return;
                }
                assert!(wait::wifstopped(status));
                assert!(!is_syscall_stop(status));

                ptrace_cont(pid, 0).expect("cont");
                let (reaped, status) = wait::waitpid(pid, 0).expect("waitpid exit");
                assert_eq!(reaped, pid);
                assert!(wait::wifexited(status));
            }
        }
    }
}
