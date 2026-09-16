# Release Notes

Stay up to date with the latest changes to rusty_libc.

> rusty_libc has not yet been published to crates.io — it is still `0.0.1` and
> consumed by [rush](https://github.com/baileyrd/rush) directly off `main`.
> Everything below is merged; there is no unreleased/pending section because
> `main` **is** the release. Once the API is considered stable for outside
> consumers we'll cut a tagged `0.x` release and this section will split into
> "Unreleased" vs. tagged versions, the way most changelogs do.

---

## Round 3, batch 4: timerfd, atomic pidfd via clone3, and a safe vfork — [PR #56](https://github.com/baileyrd/rusty_libc/pull/56)–[#58](https://github.com/baileyrd/rusty_libc/pull/58)

**July 23, 2026 • [Compare changes](https://github.com/baileyrd/rusty_libc/compare/fa43cfc...4c561a1)**

The last three items on REVIEW.md's Round 3 list to actually ship code (the fourth, a `signalfd` design question, is a decision writeup — see below).

**Added**
- `time::timerfd_create`/`timerfd_settime`/`timerfd_gettime` + `Itimerspec` — a timeout that composes directly with `poll` instead of an asynchronously delivered `SIGALRM` with its own signal-safety rules. ([#30](https://github.com/baileyrd/rusty_libc/issues/30), [PR #56](https://github.com/baileyrd/rusty_libc/pull/56))
- `process::fork_with_pidfd` via `clone3`/`CLONE_PIDFD` — closes the pid-reuse race in `fork()` + `pidfd_open()`: between those two separate syscalls the child can exit and, under a busy pid space, have its pid recycled before `pidfd_open` runs. `clone3` hands back the pidfd atomically as part of process creation. `fork` + `pidfd_open` remains available for kernels without `clone3` (Linux 5.3+). ([#20](https://github.com/baileyrd/rusty_libc/issues/20), [PR #57](https://github.com/baileyrd/rusty_libc/pull/57))
- `process::vfork_exec` via `CLONE_VFORK`\|`CLONE_VM` — a narrower `fork` for the fork-then-exec case, safe to call from a multithreaded parent (no copy-on-write duplicate of the parent's address space for a stray allocator lock to be held against). ([#21](https://github.com/baileyrd/rusty_libc/issues/21), [PR #58](https://github.com/baileyrd/rusty_libc/pull/58))

**Notes from this batch worth keeping in mind**
- `vfork_exec` needed real hand-written asm (`arch::vfork_execve`, one implementation per architecture), not ordinary Rust code after the `clone` syscall. `CLONE_VM` gives the child *actual* shared memory with the parent, not fork's copy-on-write duplicate, so the compiler's usual (and normally correct) freedom to reuse a stack slot between "this local is only live in the child branch" and "this local is only live in the parent's continuation" becomes unsound: both branches really do run, sequentially, against the same physical memory. Caught this locally in two stages — a shared-stack version corrupted the parent's own `pid` variable, and a version with a separate child stack then SIGSEGV'd, because raw `clone()` has no notion of giving the child a fresh call frame, only of resuming both parent and child from the same mid-flight point. The shipped fix does the entire clone-then-`execve` sequence as one asm block per arch, so the child never executes a single instruction of compiler-generated code.
- The `clone3` syscall (used by `fork_with_pidfd`) is unavailable in this crate's own dev sandbox — denied outright via what looks like a seccomp filter, to the point that even the Rust test runner's own thread-spawn `clone3` call fails the same way. `fork_with_pidfd`'s test tolerates this (skips rather than fails) since it can't be told apart locally from a real regression, but the real target CI (`ubuntu-latest`, unrestricted) exercises the full success path — confirmed green there.

---

## Round 3, batch 3: uname, readv/writev, rusage, and a test-race fix — [PR #51](https://github.com/baileyrd/rusty_libc/pull/51)–[#54](https://github.com/baileyrd/rusty_libc/pull/54)

**July 23, 2026 • [Compare changes](https://github.com/baileyrd/rusty_libc/compare/d6e4c83...946ad5c)**

**Fixed**
- `process::tests::set_pdeathsig_kills_child_when_parent_exits` (added in #49) was flaky — roughly 1 in 5 full-suite runs — because the intermediate process could exit before the grandchild's `prctl(PR_SET_PDEATHSIG)` call had actually run, so `pdeathsig` never armed and the grandchild just survived as an ordinary orphan. Fixed with a handshake pipe: the grandchild confirms `pdeathsig` is armed before the intermediate is allowed to exit. Found by stress-testing (10 consecutive full-suite runs) before starting the next PR in this round, not by CI catching it. ([PR #51](https://github.com/baileyrd/rusty_libc/pull/51))

**Added**
- `process::uname`/`Utsname` — system identification (kernel name, hostname, release, version, machine, domain), the primitive behind `$OSTYPE`/`$MACHTYPE`-style shell variables. ([#31](https://github.com/baileyrd/rusty_libc/issues/31), [PR #52](https://github.com/baileyrd/rusty_libc/pull/52))
- `fd::readv`/`writev` + `IoSlice`/`IoSliceMut` — scatter-gather I/O; `fd` had `read`/`write`/`pread`/`pwrite` but no vectored form. ([#32](https://github.com/baileyrd/rusty_libc/issues/32), [PR #53](https://github.com/baileyrd/rusty_libc/pull/53))
- `wait::Rusage`/`Timeval` + `waitpid_rusage`/`waitid_rusage` — `waitpid`/`waitid` discarded the kernel's resource-usage output by passing null; a `time` builtin needs exactly this. Added as siblings so existing `waitpid`/`waitid` callers are unaffected. ([#22](https://github.com/baileyrd/rusty_libc/issues/22), [PR #54](https://github.com/baileyrd/rusty_libc/pull/54))

**Notes from this batch worth keeping in mind**
- `waitid`'s rusage out-parameter reads back all-zero under the aarch64 qemu-user CI job even when the sibling `wait4`-based path (identical CPU-burning test child) reports real data on both arches — a gap in qemu-user's syscall translation for `waitid` specifically, not a crate bug or a real aarch64 kernel limitation. The content assertion in `waitid_rusage`'s test is x86_64-only for this reason (same reasoning already applied to `vdso_resolves_on_native_x86_64`).
- Clippy is genuinely per-target when code is `cfg`-gated by arch: PR #54 shipped a local-clean commit that CI caught as an aarch64-only "unused variable" (a variable read only inside an `x86_64`-gated assertion). Fixed immediately, and `cargo clippy --target aarch64-unknown-linux-gnu --all-targets -- -D warnings` is now run explicitly alongside the default-target check for every change in this round, not just when `cfg`-gating is visibly in play.

---

## Round 3, batch 2: utimensat, linkat, getgroups, priority control, pdeathsig — [PR #45](https://github.com/baileyrd/rusty_libc/pull/45)–[#49](https://github.com/baileyrd/rusty_libc/pull/49)

**July 22, 2026 • [Compare changes](https://github.com/baileyrd/rusty_libc/compare/f566d12...d6e4c83)**

Continuing straight through REVIEW.md's Round 3 list.

**Added**
- `fs::utimensat`/`utimens` + `UTIME_NOW`/`UTIME_OMIT` — set atime/mtime; nothing existed to back `touch`. ([#24](https://github.com/baileyrd/rusty_libc/issues/24), [PR #45](https://github.com/baileyrd/rusty_libc/pull/45))
- `fs::linkat`/`link` + `AT_SYMLINK_FOLLOW` — hard links; `fs` covered `symlinkat` but not `ln` (without `-s`). ([#25](https://github.com/baileyrd/rusty_libc/issues/25), [PR #46](https://github.com/baileyrd/rusty_libc/pull/46))
- `process::ngroups`/`getgroups` — supplementary group IDs for `id`/`groups`-style builtins. ([#27](https://github.com/baileyrd/rusty_libc/issues/27), [PR #47](https://github.com/baileyrd/rusty_libc/pull/47))
- `process::getpriority`/`setpriority`/`nice` — no priority control existed at all. ([#28](https://github.com/baileyrd/rusty_libc/issues/28), [PR #48](https://github.com/baileyrd/rusty_libc/pull/48))
- `process::prctl`/`set_pdeathsig`/`get_pdeathsig` — the standard fix for orphaned children surviving a crashed/killed job-control shell. ([#29](https://github.com/baileyrd/rusty_libc/issues/29), [PR #49](https://github.com/baileyrd/rusty_libc/pull/49))

**Fixed**
- The priority-control PR's own test assumed the crate's test suite always runs as root (true in this repo's dev sandbox, not guaranteed for a consumer's CI): restoring a raised nice value back down needs `CAP_SYS_NICE`, which an unprivileged runner doesn't have. GitHub's own hosted CI runner caught it immediately after merge. Fixed by isolating the whole scenario in a forked child that just exits — carrying the mutation away with it — instead of requiring a privileged restore step. Verified this round and going forward by actually running the test binaries as an unprivileged user (`setpriv --reuid=nobody`) locally, on both arches, not just re-reading the code.

Every syscall number and constant in this batch was checked directly against `/usr/include/{x86_64-linux-gnu/asm,asm-generic}/unistd.h` and `linux/prctl.h` before use, including one genuinely arch-order-swapped pair (`GETPRIORITY`/`SETPRIORITY` are `140`/`141` on x86_64 but `141`/`140` on aarch64) that a memory-only recall would have been an even-odds coin flip on.

---

## Round 3, batch 1: an aarch64 correctness fix, and five REVIEW.md items — [PR #39](https://github.com/baileyrd/rusty_libc/pull/39)–[#43](https://github.com/baileyrd/rusty_libc/pull/43)

**July 22, 2026 • [Compare changes](https://github.com/baileyrd/rusty_libc/compare/dfa4e8c...f566d12)**

The first working batch off REVIEW.md's Round 3 list, plus one bug found while verifying syscall numbers for it.

**Fixed**
- `process::execveat` was silently broken on aarch64 — `arch::aarch64::nr::EXECVEAT` was `387`, an unallocated number on the generic syscall table aarch64 actually uses, instead of the real, permanently-fixed `281`. No test called `execveat` directly (only `execve`, a different syscall number), so this went uncaught, including by the aarch64 qemu-user CI job. Fixed, with a regression test that exercises `execveat`'s own `ENOENT` path on both arches. ([#38](https://github.com/baileyrd/rusty_libc/issues/38), [PR #39](https://github.com/baileyrd/rusty_libc/pull/39))

**Added**
- `Errno` named constants for `ENXIO`, `E2BIG`, `ENOEXEC`, `EXDEV`, `ENODEV`, `ENFILE`, `ETXTBSY`, `EFBIG`, `ENOSPC`, `EROFS`, `EMLINK`, `EDOM`, `ERANGE`, `ENAMETOOLONG`, `ENOTEMPTY`, `ELOOP`, `ETIMEDOUT`, `ECONNREFUSED` — everything `name()` already recognized but had no `pub const` for. Fixed `process::getcwd`'s own test, which had hardcoded `Err(Errno(34)) // ERANGE` for want of the const. ([#34](https://github.com/baileyrd/rusty_libc/issues/34), [PR #40](https://github.com/baileyrd/rusty_libc/pull/40))
- `process::CStrArray` (`std` feature): an owned builder for the null-terminated `argv`/`envp` arrays `execve`/`execveat` need, so callers no longer have to hand-roll pointer arrays over self-managed `CString`s. ([#35](https://github.com/baileyrd/rusty_libc/issues/35), [PR #41](https://github.com/baileyrd/rusty_libc/pull/41))
- `fd::ftruncate` — resize an already-open descriptor; `O_TRUNC` only truncates at `open` time. ([#26](https://github.com/baileyrd/rusty_libc/issues/26), [PR #42](https://github.com/baileyrd/rusty_libc/pull/42))
- `fs::fchmodat`/`chmod` and `fs::fchownat`/`chown`/`lchown`, plus a `DONT_CHANGE` sentinel constant for `fchownat`'s per-field "leave this id alone" — `fs` had full path mutation but no way to change permissions or ownership at all. ([#23](https://github.com/baileyrd/rusty_libc/issues/23), [PR #43](https://github.com/baileyrd/rusty_libc/pull/43))

All five REVIEW.md-derived changes verified on both x86_64 and aarch64 via a local cross toolchain + qemu-user (matching the CI matrix exactly), not just the CI run itself — every new syscall number was checked directly against `/usr/include/{x86_64-linux-gnu/asm,asm-generic}/unistd.h` rather than recalled from memory, which is exactly the discipline that caught the `execveat` bug above.

---

## Capabilities assessment + standard governance scaffolding — [PR #36](https://github.com/baileyrd/rusty_libc/pull/36)

**July 22, 2026 • [Compare changes](https://github.com/baileyrd/rusty_libc/compare/dfa4e8c...50bf31b)**

**Added**
- `REVIEW.md` Round 3: a fresh line-by-line capabilities assessment against
  every module, done after Round 2 and the Track P work landed. Filed as
  [issues #20–#35](https://github.com/baileyrd/rusty_libc/issues?q=is%3Aissue):
  `clone3(CLONE_PIDFD)` to close the `fork`+`pidfd_open` pid-reuse race, a
  `vfork`-style clone for the fork-then-exec path, `rusage` reporting on
  `waitpid`/`waitid`, `chmod`/`chown`/`utimensat`/`linkat`/`ftruncate` in
  `fs`, `getgroups`/`nice`-family/`prctl(PR_SET_PDEATHSIG)` in `process`, a
  timeout primitive, `uname`, `readv`/`writev`, plus an open design question
  on `signalfd` vs. `sigaction` and two `Errno`/`execve`-ergonomics nits.
  Assessment only — none of the items are implemented yet.
- Standard governance scaffolding via the `repo-config` skill: PR templates
  (feature/bug_fix/docs/chore), issue templates, `CONTRIBUTING.md`,
  `CODE_OF_CONDUCT.md`, `SECURITY.md`, `ARCHITECTURE.md` (filled in for this
  crate's actual arch-vs-syscall-subsystem boundary and non-goals, not left
  as template scaffolding), and a seed ADR log at `docs/adr/`.

**Changed**
- README now links `ARCHITECTURE.md` and the new `CONTRIBUTING.md`/
  `CODE_OF_CONDUCT.md`/`SECURITY.md`.

Deliberately **not** added: a generic `ci-rust.yml` (the existing
`.github/workflows/ci.yml` is more thorough and its `--test-threads=1`
requirement is load-bearing for the fork/signal tests — a generic template
would have both duplicated and broken it) and `CHANGELOG.md` (this file
already serves that role).

---

## `getdents64` + `pidfd_open` — [PR #19](https://github.com/baileyrd/rusty_libc/pull/19)

**July 19, 2026 • [Compare changes](https://github.com/baileyrd/rusty_libc/compare/40a8bd1...a373607)**

**Added**
- `fs::getdents64` + `fs::dirents` — the last syscall standing between
  `rush`'s directory listing and Track P (`readdir`/`DIR*` was the one
  remaining glibc-only path). `getdents64` fills a caller-owned buffer with
  packed kernel `linux_dirent64` records, no per-entry allocation or hidden
  internal buffering, matching this crate's `read`-style bring-your-own-buffer
  convention; `dirents` parses that buffer into a zero-allocation
  `Iterator<Item = RawDirent>` by walking the kernel's own `d_reclen` chain.
  `fs::DT_REG`/`DT_DIR`/`DT_LNK`/`DT_UNKNOWN` type tags included.
- `process::pidfd_open` — a stable process-identity handle immune to pid
  reuse, closing Track P's other remaining gap (`SYS_pidfd_open` was a raw
  `syscall()` escape hatch on the consumer side until now). Poll it for
  readability (readable once the process exits) or feed it to
  `wait::waitid(P_PIDFD, ...)`.

---

## Inline the hot path — [PR #16](https://github.com/baileyrd/rusty_libc/pull/16)

**July 17, 2026 • [Compare changes](https://github.com/baileyrd/rusty_libc/compare/97c80c2...0fb52f4)**

**Performance**
- Marked 15 thin convenience wrappers `#[inline]` — `open`, `access`,
  `stat`/`lstat`/`fstat`, `unlink`/`rmdir`, `mkdir`, `rename`, `symlink`,
  `readlink`, `killpg`, `getpgrp`, `getrlimit`/`setrlimit`, `tcsetattr`,
  `isatty`, `signal`/`sigaction`. Verified by disassembling a downstream
  consumer built **without** LTO: these functions previously survived as real,
  separate symbols reached through an indirect call, because rustc's automatic
  cross-crate inlining only reaches trivial single-hop leaves (like `getpid`)
  and not functions that forward to another wrapper. After the annotation they
  fully collapse into the caller.

---

## Coarse clocks + `clock_gettime` tuning — [PR #15](https://github.com/baileyrd/rusty_libc/pull/15)

**July 17, 2026 • [Compare changes](https://github.com/baileyrd/rusty_libc/compare/df46fdd...97c80c2)**

**Added**
- `time::CLOCK_REALTIME_COARSE`, `CLOCK_MONOTONIC_COARSE`, `CLOCK_BOOTTIME`.
  The `*_COARSE` clocks skip the vDSO's fine-grained interpolation, trading
  millisecond-ish resolution for speed — about 4x faster than the precise
  clock in this crate's own benchmark. The right choice for `$SECONDS`-style
  counters, throttling, or any hot-path timestamp that doesn't need sub-ms
  accuracy.

**Performance**
- `clock_gettime` now fills a `MaybeUninit<Timespec>` instead of
  zero-initializing, and is marked `#[inline]`.
- The vDSO function-pointer cache now uses `Ordering::Relaxed` instead of
  `Acquire`/`Release` — the cached value is a fixed vDSO code address the
  kernel already mapped executable before the process ran any code, so there
  was nothing for the fence to synchronize.

---

## Benchmark: add a musl comparison — [PR #14](https://github.com/baileyrd/rusty_libc/pull/14)

**July 17, 2026 • [Compare changes](https://github.com/baileyrd/rusty_libc/compare/7da292e...df46fdd)**

**Added**
- `bench/` now builds and runs against **musl** as well as glibc
  (`--target x86_64-unknown-linux-musl`, or `bench/run.sh` for both). The
  harness self-labels its output (`glibc` vs `musl`) from `target_env`, and
  the musl build is a static binary via Rust's self-contained musl linking —
  no `musl-gcc` required.

---

## A reproducible benchmark harness — [PR #13](https://github.com/baileyrd/rusty_libc/pull/13)

**July 17, 2026 • [Compare changes](https://github.com/baileyrd/rusty_libc/compare/dc466d3...7da292e)**

**Added**
- `bench/`: a standalone crate (its own `[workspace]`, kept out of the
  library's build) comparing rusty_libc against the `libc` crate on the same
  syscalls — `getpid`, `getuid`, `read`, `write`, `clock_gettime`. `libc` never
  enters the library's own dependency graph, so the zero-dependency guarantee
  holds.

---

## A vDSO fast path for `clock_gettime` — [PR #12](https://github.com/baileyrd/rusty_libc/pull/12)

**July 17, 2026 • [Compare changes](https://github.com/baileyrd/rusty_libc/compare/f0d08d1...dc466d3)**

**Performance**
- `time::clock_gettime` now resolves and calls the process vDSO's
  `clock_gettime` (reading `AT_SYSINFO_EHDR` from `/proc/self/auxv` and parsing
  the vDSO's ELF symbol table — no libc), matching glibc's speed instead of
  paying a syscall trap on every call. About 11x faster
  (~287ns → ~26ns), landing within a few ns of glibc's own vDSO call. Falls
  back to the raw syscall whenever the vDSO is unavailable, so it is a pure
  optimization, never a correctness dependency.

---

## More syscalls: `waitid`, `pread`/`pwrite`, `getpgrp`, errno aliases — [PR #11](https://github.com/baileyrd/rusty_libc/pull/11)

**July 17, 2026 • [Compare changes](https://github.com/baileyrd/rusty_libc/compare/d058f1f...f0d08d1)**

**Added**
- `wait::waitid` — peek a child's status via `WNOWAIT` **without** reaping it,
  returning a kernel `Siginfo`; plus `P_ALL`/`P_PID`/`P_PGID`,
  `WEXITED`/`WSTOPPED`, and the `CLD_*` reason codes.
- `fd::pread`/`fd::pwrite` — positioned I/O that doesn't move the file offset.
- `process::getpgrp()` — convenience for `getpgid(0)`.
- `Errno::EWOULDBLOCK` (alias of `EAGAIN`) and `Errno::EINPROGRESS`.

---

## Round 2: filesystem metadata, faccessat, time, signal flags, terminal control — [PR #10](https://github.com/baileyrd/rusty_libc/pull/10)

**July 17, 2026 • [Compare changes](https://github.com/baileyrd/rusty_libc/compare/4f04cc1...d058f1f)**

**Added**
- `fs` module: `statx` + `stat`/`lstat`/`fstat` (kernel-layout `Statx` with
  `is_dir`/`is_file`/`is_symlink`), `faccessat`/`access`
  (`F_OK`/`R_OK`/`W_OK`/`X_OK`), and the path mutators `unlinkat`/`mkdirat`/
  `renameat2`/`symlinkat`/`readlinkat` with `unlink`/`rmdir`/`mkdir`/`rename`/
  `symlink`/`readlink` shorthands.
- `time` module: `Timespec`, `clock_gettime`, `nanosleep`,
  `CLOCK_REALTIME`/`CLOCK_MONOTONIC`.
- `process::geteuid`/`getgid`/`getegid` — effective/real credential getters
  for privilege-aware prompts and permission checks.
- `signal::sigaction(sig, handler, flags)` with the full `SA_*` constant set
  (`signal()` keeps its simpler `SA_RESTART`-only BSD semantics), plus
  `sigpending` and `sigsuspend` — the race-free "wait for a signal" primitive.
- `termios`: the full `c_cc` index set (`VINTR`..`VEOL2`), `tcflush`/`tcdrain`.
- `fd::dup3(oldfd, newfd, flags)`, public, for atomic-`O_CLOEXEC`
  redirections.

**Changed**
- README rewritten: status, a per-module coverage table, a usage example, the
  `std` feature, and MSRV.

---

## Fix nits: `prlimit(pid, …)`, a `killpg` comment, pipe-size `fcntl` — [PR #9](https://github.com/baileyrd/rusty_libc/pull/9)

**July 17, 2026 • [Compare changes](https://github.com/baileyrd/rusty_libc/compare/92ab59b...4f04cc1)**

**Added**
- `rlimit::prlimit(pid, resource, new, old)` — the full `prlimit64` primitive
  (get/set/atomic-swap on any pid); `getrlimit`/`setrlimit` are now thin
  `pid = 0` wrappers over it.
- `fd::F_SETPIPE_SZ`/`F_GETPIPE_SZ` — the real pipe-capacity mechanism (Linux
  has no `RLIMIT` for it).

**Fixed**
- `killpg`'s `pgrp.wrapping_neg()` now has a comment explaining why it's
  correct (including the `pgrp == 0` → "caller's own group" case) instead of
  reading like an accident.

---

## Implementation review round 1: exec, filesystem, signals, session control, ergonomics, CI — [PR #8](https://github.com/baileyrd/rusty_libc/pull/8)

**July 17, 2026 • [Compare changes](https://github.com/baileyrd/rusty_libc/compare/65f467f...92ab59b)**

The first pass of a full implementation review (see
[REVIEW.md](REVIEW.md)) — sixteen additions closing the gap between "raw
syscalls exist" and "a shell can actually use this."

**Added**
- `process::execve`/`execveat` — raw exec, the piece that made a raw `fork`
  with no way to launch external commands.
- `fd::open`/`openat` + the `O_*` flag set — file redirections.
- `signal::sigprocmask` + `sigmask()` — block `SIGCHLD` around job-control
  critical sections.
- `process::chdir`/`fchdir`/`getcwd` — the `cd` builtin and `$PWD`.
- `process::setsid`/`getpgid`/`getsid` — session and process-group control.
- Named `Errno` constants (`EBADF`, `EINVAL`, `EAGAIN`, …) plus `Errno::name()`,
  `Display`, and `core::error::Error`; an opt-in `std` feature adds
  `From<Errno> for std::io::Error`.
- The full `POLL*` flag set and `PollFd` helper methods
  (`is_readable`/`is_hup`/…).
- `fcntl` `F_GETFL`/`F_SETFL`; `O_CLOEXEC`/`O_NONBLOCK` constants.
- `fd::read_all`/`write_all` — loop over short I/O, retrying on `EINTR`.
- `termios::tcsetattr_with` + `TCSANOW`/`TCSADRAIN`/`TCSAFLUSH`.

**Changed**
- MSRV declared (`rust-version = "1.88"`, the `naked_asm!` floor) with a CI
  job to catch regressions.
- `unsafe_op_in_unsafe_fn` and `missing_docs` promoted from warn to deny;
  `cargo doc -D warnings` added to CI.
- CI now builds `no_std` explicitly (`--no-default-features`) and the `std`
  feature separately, and runs tests single-threaded so the process is
  effectively single-threaded at each `fork` point.

---

## Foundational build — Phases 1–5

**July 10–11, 2026**

The initial implementation, before the review process above began.

- **Phase 1** — raw-syscall core: `syscall0`..`syscall6`, `Errno` decode,
  `fd` primitives (`read`/`poll`/`pipe2`/`dup`/`dup2`/`close`/`fcntl`),
  process ids and `setpgid`, `tcsetpgrp`/`isatty`, `rlimit`, `umask`.
- **Phase 2** — kernel-layout `Termios`, raw-mode transform, validated against
  a real PTY.
- **Phase 3** — signals via `rt_sigaction`, including the x86_64
  `SA_RESTORER` trampoline (a naked function issuing `rt_sigreturn`) — the
  hardest problem in the crate, stress-tested with a 5,000-signal delivery
  storm.
- **Phase 4** — raw `fork` (`clone(SIGCHLD)`), `memfd_create`, `write`,
  `lseek`.
- **Phase 5** — the aarch64 syscall table and a two-arch CI matrix
  (`x86_64` native, `aarch64` cross-compiled under `qemu-user`).
- `RLIMIT_RTTIME` constant.
