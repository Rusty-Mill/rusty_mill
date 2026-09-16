# rusty_libc — design & requirements

Goal: a `#![no_std]`, zero-dependency, Linux-only Rust crate that replaces
the `libc` FFI-bindings crate for [rush](https://github.com/baileyrd/rush),
issuing raw syscalls via inline asm instead of linking prototypes against
glibc.

> **Status:** this document captures the *original* requirements and phasing
> that kicked off the crate; all five phases below are complete, and several
> rounds of implementation review since have gone well past the original
> ~25-syscall/~100-symbol scope (raw `execve`, `fs`/`time` modules, `waitid`,
> a vDSO fast path for `clock_gettime`, …). See [RELEASE_NOTES.md](RELEASE_NOTES.md)
> for the full history and [REVIEW.md](REVIEW.md) for the review log. It is
> kept as-is below for the rationale, not as a live status page — in
> particular, the "`fork` stays on glibc" ground rule was the original,
> deliberately cautious starting position; Phase 4 implemented a raw `fork`
> and a later round added raw `execve`, both with the safety caveats spelled
> out in their own doc comments (`process::fork`'s safety note is the current
> word on when it's safe to call).

Scope and rationale are established in rush's
`docs/LIBC_DEPENDENCY_ANALYSIS.md` (same-named branch there). Summary of the
ground rules that fall out of that analysis:

- **Linux only** (`cfg(target_os = "linux")`): Linux is the only major
  kernel with a stable public syscall ABI. macOS/BSD must keep going through
  their system libc.
- **This does not remove glibc from rush's binary** — Rust std links it on
  `*-linux-gnu`. It removes the third-party `libc` crate and gives us our
  own kernel interface. (A truly glibc-free binary is a separate goal; use
  static musl for that.)
- **`fork` and `std::process::Command` stay on glibc** (at least initially):
  glibc's `fork()` resets its internal malloc/stdio locks in the child;
  rush runs helper threads, and its forked children keep executing Rust
  without exec'ing, so a raw `SYS_clone(SIGCHLD)` risks a child deadlocked
  on an inherited heap lock. Revisit only with a plan for thread quiescence.

## Required API surface (rush's exact needs)

~25 syscalls, ~100 symbols. From the inventory in rush's analysis doc:

| Module | Exports | Backing syscalls (x86_64 / aarch64) |
|---|---|---|
| `arch` | `syscall0`..`syscall6`, `Errno` decode (`-4095..-1`) | `core::arch::asm!` per arch |
| `process` | `getpid`, `getppid`, `getuid`, `setpgid`, `kill`, `killpg`, `exit_group`; later `fork` | direct; `killpg(pg)` = `kill(-pg)`; `fork` = `SYS_fork` / `SYS_clone(SIGCHLD)` |
| `wait` | `waitpid` + `WNOHANG`/`WUNTRACED`/`WCONTINUED`; `wifexited`, `wexitstatus`, `wifsignaled`, `wtermsig`, `wifstopped`, `wstopsig`, `wifcontinued` | `SYS_wait4`; status fns are bit tests |
| `signal` | `sigaction`-backed `signal(sig, handler)`, `SIG_DFL`/`SIG_IGN`, `SIG*` constants (rush uses ~20), `sighandler_t` | `SYS_rt_sigaction` with `sigsetsize = 8`; **x86_64 needs our own `SA_RESTORER` trampoline executing `SYS_rt_sigreturn`** (never-inline naked asm); aarch64 kernel default suffices |
| `termios` | kernel-layout `Termios` (**NCCS = 19**, no glibc speed fields), `tcgetattr`, `tcsetattr(TCSADRAIN)`, `tcgetpgrp`, `tcsetpgrp`, `isatty`; `ICANON ECHO ISIG IEXTEN IXON ICRNL INLCR VMIN VTIME` | `ioctl(TCGETS)`, `ioctl(TCSETSW)`, `ioctl(TIOCGPGRP/TIOCSPGRP)`; `isatty` = `TCGETS` succeeds |
| `tty` | `Winsize`, window-size query | `ioctl(TIOCGWINSZ)` |
| `fd` | `read`, `poll(&mut [PollFd], timeout)`, `pipe2`, `dup`, `dup2`, `close`, `fcntl` (`F_GETFD`/`F_SETFD`/`FD_CLOEXEC`), `POLLIN` | `SYS_read`, `SYS_poll` / `SYS_ppoll` (aarch64 has no `poll`), `SYS_pipe2`, `SYS_dup`, `SYS_dup2` / `SYS_dup3` (aarch64 has no `dup2`), `SYS_close`, `SYS_fcntl` |
| `rlimit` | `prlimit`, `getrlimit`, `setrlimit`, `Rlimit { cur, max }` (always u64), `RLIM_INFINITY`, `RLIMIT_*` consts (`CORE DATA FSIZE NICE SIGPENDING MEMLOCK RSS NOFILE MSGQUEUE RTPRIO STACK CPU NPROC AS LOCKS RTTIME`) | `SYS_prlimit64` for both directions (`getrlimit`/`setrlimit` = pid 0). Pipe capacity is **not** an rlimit — it's `fcntl(F_SETPIPE_SZ)` in `fd` |
| `umask` | `umask(mode)` | `SYS_umask` |

Design conventions:

- Safe wrappers return `Result<T, Errno>`; raw `-errno` never escapes.
  Callers needing `std::io::Error` use `Error::from_raw_os_error(errno)` —
  we do **not** write glibc's TLS `errno`, so `last_os_error()` is not
  meaningful after our calls.
- `unsafe` confined to `arch` (asm) and pointer-taking ioctl internals;
  public API is safe except where inherently not (`signal`, `fork`).
- All struct layouts are the **kernel's**, checked by `const _:` size/offset
  assertions, never copied from glibc headers.

## Phasing (mirrors rush's migration plan)

1. **Core + easy syscalls** — asm stubs, errno, fds, `poll`/`read`,
   `TIOCGWINSZ`, `umask`, `prlimit64`, pids, `setpgid`, `tcsetpgrp`,
   `isatty`. Low risk; ~700–1000 LOC with tests.
2. **termios raw mode** — kernel struct + `TCGETS`/`TCSETSW`; validate
   against rush's line editor under its PTY tests.
3. **Signals** — `rt_sigaction` + x86_64 restorer trampoline; signal-storm
   stress test (deliver thousands of SIGTERM/SIGCHLD, assert no crash, no
   missed count).
4. **fork/wait4** — only per the ground rule above, or keep on glibc.
5. **aarch64 + CI matrix** — per-arch syscall tables; run rush's full suite
   `--features rusty-libc` and without, on both arches.

Integration on the rush side goes through a thin `sys` facade module gated
by a `rusty-libc` cargo feature, so the `libc` crate remains the default
until both configurations pass the full test suite.

## Testing strategy

- Unit tests per wrapper (pipe/dup/fcntl round-trips, rlimit get/set/restore,
  umask save/restore, poll timeout behavior).
- Layout assertions compiled on every target (`size_of`, `offset_of`).
- The real gate: rush's existing PTY integration suite (job control, raw
  mode, traps) running entirely on rusty_libc.
- Stress: signal storms (phase 3); fork-under-allocation-load (phase 4, if
  ever taken off glibc).
