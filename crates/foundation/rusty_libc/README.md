# rusty_libc

A `#![no_std]`, zero-dependency, Linux-only raw-syscall crate that replaces the
`libc` FFI-bindings crate in [rush](https://github.com/baileyrd/rush). It issues
syscalls via inline asm instead of linking prototypes against glibc, and models
every struct with the **kernel's** layout (checked by `const` size/offset
assertions), not glibc's.

- **Targets:** `x86_64` and `aarch64` Linux (a per-arch syscall table each).
- **No dependencies.** Safe wrappers return `Result<T, Errno>`; a raw `-errno`
  never escapes the crate.
- **Optional `std` feature** adds `From<Errno> for std::io::Error` for callers
  that work in `std::io::Result`.
- **MSRV 1.88** (the crate uses naked functions for the x86_64 signal-return
  trampoline).

## Coverage

| Module | What it provides |
|---|---|
| `arch` | `syscall0`..`syscall6`, the `Errno` newtype (named constants, `Display`, `Error`), and `-errno` decoding |
| `fd` | `read`/`write`(`_all`), `pread`/`pwrite`, `readv`/`writev`/`preadv`/`pwritev` (`IoSlice`/`IoSliceMut`), `open`/`openat`, `poll`, `pipe2`, `dup`/`dup2`/`dup3`, `close`/`close_range`, `fcntl`, `memfd_create`, `lseek`, `ftruncate`, `fsync`/`fdatasync`; `O_*`/`F_*`/`POLL*` constants |
| `fs` | `statx`/`stat`/`lstat`/`fstat`, `faccessat`/`access`, `unlinkat`/`mkdirat`/`renameat2`/`symlinkat`/`readlinkat`/`linkat` (+ `unlink`/`mkdir`/`rename`/`link`/… shorthands), `fchmodat`/`chmod`, `fchownat`/`chown`/`lchown`, `utimensat`/`utimens`, `truncate` |
| `process` | pids/uids/gids (real & effective), `gettid`, `ngroups`/`getgroups`, `setuid`/`setgid`/`seteuid`/`setegid`/`setresuid`/`setresgid`/`getresuid`/`getresgid`/`setgroups`, `getpriority`/`setpriority`/`nice`, `prctl`/`set_pdeathsig`/`get_pdeathsig`, `uname`, `setpgid`/`setsid`/`getpgid`/`getsid`/`getpgrp`, `kill`/`killpg`, `chdir`/`fchdir`/`getcwd`, `exit_group`, raw `fork`, `pidfd_open`, `fork_with_pidfd` (atomic `clone3`/`CLONE_PIDFD`), `pidfd_send_signal`, `execve`/`execveat`, `vfork_exec` (multithread-safe fork-then-exec via `CLONE_VFORK`\|`CLONE_VM`), `vfork_exec_redirected` (same, plus `dup2`-shaped fd redirection before `exec`), `process_vm_readv`/`process_vm_writev` (cross-process memory scatter/gather, `RemoteIoVec`), `CStrArray` (`std` feature) |
| `signal` | `signal`/`sigaction` (+ `SA_*`), `sigprocmask`/`sigpending`/`sigsuspend`, the x86_64 `SA_RESTORER` trampoline, `SIG*` constants, `signalfd` + `SignalfdSiginfo` (recommended for async signal handling — see [ADR-0002](docs/adr/0002-signalfd-as-primary-event-driven-signal-path.md)), `sigqueue` (send a signal with a payload) |
| `socket` | TCP/UDP over `AF_INET`/`AF_INET6`: `socket`/`bind`/`connect`/`listen`/`accept`/`accept4`/`send`/`recv`/`sendto`/`recvfrom`/`shutdown`/`getsockname`/`getpeername`, `SockAddrIn`/`SockAddrIn6` (kernel `sockaddr_in`/`sockaddr_in6` layouts) — see [ADR-0003](docs/adr/0003-add-sockets-tcp-udp-and-dns-resolution.md) |
| `dns` | `resolve_a`/`resolve_aaaa` — a minimal stub resolver (RFC 1035 A/AAAA over UDP port 53, `/etc/resolv.conf`), no `getaddrinfo`/libc involved |
| `eventfd` | `eventfd` (`EFD_SEMAPHORE`/`EFD_CLOEXEC`/`EFD_NONBLOCK`) — a pollable kernel counter, the third `*fd` primitive alongside `signalfd`/`timerfd_create`; drive it with `fd::read`/`fd::write` |
| `epoll` | `epoll_create1`/`epoll_ctl`/`epoll_wait`, `EpollEvent` (kernel `struct epoll_event` — packed on x86_64, natural alignment on aarch64) — a scalable successor to `fd::poll` for many-fd event loops |
| `inotify` | `inotify_init1`/`inotify_add_watch`/`inotify_rm_watch`, `inotify_events` (zero-copy iterator over a raw read buffer, `IN_*` masks) — filesystem change notification |
| `sysinfo` | `sysinfo` (kernel `struct sysinfo`, 112 bytes) — uptime, 1/5/15-minute load averages, RAM/swap totals in one syscall |
| `wait` | `waitpid`/`waitpid_rusage` (via `wait4`), `waitid`/`waitid_rusage` (with `WNOWAIT` and a `Siginfo`), `getrusage` (`RUSAGE_SELF`/`RUSAGE_CHILDREN`), `Rusage`, and the `W*` status decoders |
| `rand` | `getrandom` (`GRND_NONBLOCK`/`GRND_RANDOM`) |
| `mmap` | `mmap`/`munmap`/`mprotect`/`madvise`/`mlock`/`munlock`/`msync`; `PROT_*`/`MAP_*`/`MADV_*`/`MS_*` constants |
| `ptrace` | `ptrace_traceme`/`ptrace_seize`/`ptrace_cont`/`ptrace_singlestep`/`ptrace_detach`/`ptrace_setoptions`/`ptrace_getregset`/`ptrace_setregset`/`ptrace_syscall`, `is_syscall_stop`, `GpRegs` (kernel `struct user_regs_struct`/`user_pt_regs`) — tracer/tracee lifecycle, portable register access, and syscall-entry/exit tracing, scoped by [ADR-0004](docs/adr/0004-ptrace-scope-and-shape.md) |
| `termios` | kernel `Termios`, `tcgetattr`/`tcsetattr`(`_with`), `make_raw`, `tcflush`/`tcdrain`/`tcflow`/`tcsendbreak`, tty pgrp queries, `tcgetsid`, `isatty`, `ttyname` (via `/proc/self/fd/N`, same as glibc), full `c_cc`/flag constants |
| `tty` | `Winsize` window-size query, `openpty` (pty pair allocation) |
| `rlimit` | `prlimit`/`getrlimit`/`setrlimit`, `RLIMIT_*` |
| `time` | `Timespec`, `clock_gettime` (vDSO fast path, no syscall) incl. the `*_COARSE`/`BOOTTIME` clocks, `clock_getres`, `nanosleep`, `clock_nanosleep` (absolute/explicit-clock, `TIMER_ABSTIME`), `Itimerspec`, `timerfd_create`/`timerfd_settime`/`timerfd_gettime` |
| `umask` | `umask` |

## Example

```rust
use rusty_libc::{fd, fs, Errno};

fn read_hostname() -> Result<(), Errno> {
    // Test a PATH candidate the way a shell does, then read a file.
    if fs::access(c"/bin/sh", fs::X_OK).is_ok() {
        let f = fd::open(c"/etc/hostname", fd::O_RDONLY, 0)?;
        let mut buf = [0u8; 256];
        let n = fd::read_all(f, &mut buf)?;
        fd::close(f)?;
        let _ = &buf[..n];
    }
    Ok(())
}
```

## Benchmark

[`bench/`](bench/) is a standalone harness comparing rusty_libc against the
`libc` crate on the same syscalls — both **glibc** (default) and **musl**
(`cargo run --release --target x86_64-unknown-linux-musl`, or `./run.sh` for
both). It is its own workspace and never built by the library's own `cargo`
commands, so the zero-dependency guarantee holds. Summary: at parity with
both libcs for genuine syscalls, and `clock_gettime` matches their vDSO speed
via the fast path in [`src/vdso.rs`](src/vdso.rs), with `*_COARSE` clocks
faster still. See [bench/README.md](bench/README.md).

See [DESIGN.md](DESIGN.md) for the API surface, the hard problems (signal
restorer trampoline, fork-vs-threads, kernel-vs-glibc layouts), phasing, and
testing strategy; [ARCHITECTURE.md](ARCHITECTURE.md) for the module/port
boundary and data flow; [REVIEW.md](REVIEW.md) for the implementation-review
log (including tracked-but-not-yet-implemented gaps); and
[RELEASE_NOTES.md](RELEASE_NOTES.md) for a changelog of what shipped and
when. The dependency analysis that motivates the crate lives in rush's
`docs/LIBC_DEPENDENCY_ANALYSIS.md`.

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md) for the workflow and review
conventions, [CODE_OF_CONDUCT.md](CODE_OF_CONDUCT.md), and
[SECURITY.md](SECURITY.md) to report a vulnerability privately.
