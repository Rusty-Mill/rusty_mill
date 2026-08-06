# Extraction map: rush → rustils

Recorded 2026-07-19. Supersedes the earlier `r2-hoist-donor.md` (which
covered `rusty_win32` only) after a full review of
[`baileyrd/rush`](https://github.com/baileyrd/rush) and its satellite
crates. Companion to **RFC v2 Amendment A1** (see `rfc-v2.md` §7), which
re-grounds the R2 "hoist" on the facts below.

## Why this document exists

rush predates rustils. The RFC's §7 was written against a planning
document for an *alternate* rush that was never built; the real rush took
its own path and, in doing so, built — and battle-tested on real CI —
nearly everything rustils's roadmap planned to receive at R2/R3/R4. The
hoist is therefore not "wait for rush's Phase 2 gate"; it is an
**extraction project this repo can start at any time**, porting semantics
and tests (not linking code) from the donors below, re-floored on this
repo's tier doctrine (§2) and API standards (§5), with rush's suites as
the conformance oracle.

## Donor inventory

### D1 — `rush/src/job.rs`: Unix process groups & terminal control

A glibc-style job-control implementation with the hard invariants learned
and commented in place:

- Double-`setpgid` (parent *and* child) so terminal hand-off cannot race
  group creation.
- `tcsetpgrp` give/reclaim, sound only because SIGTTOU is ignored in the
  shell — a precondition the rustils API must encode, not assume.
- Exit-status decode through the full `WIFEXITED`/`WIFSIGNALED`/
  `WIFSTOPPED`/`WIFCONTINUED` set (this repo's B-5 sentinel, live).
- A strictly async-signal-safe `pre_exec` set (`setpgid` + `signal` only)
  for exec'd children; forked in-process stages are sound only under a
  single-threaded-at-fork invariant (see D4).

Stays rush-side (policy): job tables, `%n` specs, `$!` = last-stage pid,
`128+sig` conventions, `jobs`/`fg`/`bg`/`wait`/`disown` builtins.

**Landed (job-control slice) 2026-07-21** — forced by `nexus-rush/src/job.rs`
converging onto `platform::process`/`platform::term` (`baileyrd/nexus#454`,
split into rustils#43–#46): `GroupSpec::JoinGroup` (the pipeline-stage
group-join half, shared with D2 below), `Signal` +
`kill_tree`/`kill_single(sig)`, `ExitStatus::Stopped`/`Continued` +
`Child::wait_job`/`try_wait_job` (D10, below), and
`platform::term::JobControlTerminal::give_terminal` (`tcsetpgrp`,
`docs/behavior/term.md`) gated on `ignore_sigttou`. Unix-only throughout;
`docs/divergences.md` #008 records the Windows gaps.

### D2 — `rush/src/winjob.rs` + `rusty_win32`: Windows jobs & spawn

- Suspended-spawn → assign-to-Job-Object → resume: membership guaranteed
  before the child executes one instruction. The `GroupSpec` mechanism.
- Kill-on-close semantics with the `disown` lesson: the limit must be
  reversed *before* dropping the handle, or the process dies anyway.
- `wait_any` over `WaitForMultipleObjects` with a documented fallback at
  the 64-handle cap — the limit §5.6 requires the reactor to absorb.
- Explicit environment-block spawn (a shell tracking its own variable
  table cannot rely on inheritance).
- The std-slot swap spawn-inheritance model: mark inheritable only the
  slots this spawn touched; restore immediately (CreateProcessW snapshots
  at spawn). Alternative at extraction time: STARTUPINFO handle lists —
  decide deliberately, with rush's comments as the record of why the swap
  model works and where it is fragile.

**Landed (adopt-by-pid slice) 2026-07-23** — rustils#47:
`nexus-terminal/src/job_object.rs`'s `JobObject::assign_pid(pid)`
attaches a Job Object to a pid `nexus` didn't spawn (obtained from
`portable-pty::Child::process_id()`) — a shape D2's own donor material
doesn't cover, since `winjob.rs` always creates its job as part of its
own suspended-spawn. `Spawner::adopt(pid) -> Result<Box<dyn
GroupHandle>>` closes that gap: `OpenProcess` + the same
`create_kill_on_close_job`/`AssignProcessToJobObject` sequence D2's
suspended-spawn already uses, applied to a pid obtained after the fact
rather than one this backend just created suspended. Unix has no sound
equivalent — `docs/divergences.md` #010 — so this landed as a
Windows-real / Unix-`Unsupported` pair, not a symmetric capability.

### D3 — `winjob.rs::build_command_line`/`quote_arg`: the winargv seed

A tested reimplementation of the std library's MSVCRT quoting algorithm
(2n+1 backslashes before an embedded quote; trailing-backslash doubling;
quote-if-empty-or-whitespace). Direct seed for §5.4's `winargv` module.

**Known gap rustils must close — and hand back:** rush resolves `.BAT`/
`.CMD` via PATHEXT and quotes them with MSVCRT rules, but cmd.exe parses
batch arguments under different rules (the BatBadBut class). rush's
*foreground* path is protected by `std::process::Command`'s own guard;
its *background* path is not. rustils's contract — escape under cmd rules
or **refuse** unrepresentable arguments — is stronger than what exists.
This is the clearest case where extraction pays the donor back: build it
here, fuzz it against an argv-echo oracle on Windows CI (§9.5), and rush
adopts it.

### D4 — `rush/src/sys.rs` + `rusty_libc` + `docs/LIBC_DEPENDENCY_ANALYSIS.md`: the Track P blueprint (R4)

Track P already exists in prototype. The facade pattern (identical
surface, per-target backend selection, compile-error on no backend) is
the shape; the analysis doc is the map: ~25-syscall surface, and the
soundness landmines with resolutions —

- x86_64 `SA_RESTORER` signal-return trampoline (hand-written asm; wrong
  = crash on first delivered signal; aarch64 uses the vDSO instead).
- Kernel vs glibc `termios` layout (NCCS 19 vs 32) — silent stack
  corruption if the glibc shape is assumed.
- aarch64's removed syscalls (`fork`/`dup2`/`pipe`/`poll` →
  `clone`/`dup3`/`pipe2`/`ppoll`).
- The errno contract: raw syscalls must not write glibc's TLS errno;
  rush's `LAST_ERRNO` thread-local stash is the realized pattern.
- Fork vs malloc-lock deadlock, solved beyond the doc's minimum: memfd-
  backed here-docs remove the helper thread entirely, making a raw
  `clone(SIGCHLD)` fork sound (single-threaded at every fork point).

At R4, evaluate adopting `rusty_libc` as the Track P backend outright
versus re-deriving it; either way this material is the curriculum (M1).

> **Landed (first slice, D-12 supersedes the R4 wait):** O-2 resolved the
> evaluation early — `rusty_libc` IS the backend, as a rev-pinned git
> dependency behind platform-linux's `track-p` feature (off by default;
> rusty_libc's 1.88 MSRV sits above the workspace's 1.75 floor, so only
> the ubuntu+stable CI leg turns it on). First replaced family:
> `sys::fdio::read`/`write`, with the whole platform-linux suite re-run
> under `--features track-p` as the equivalence test and the errno-contract
> lesson written up (`docs/learning/002-…`). Remaining families migrate
> call-by-call in later slices.

### D5 — `rush/src/exec.rs` + `winstdio.rs`: stdio & fd mechanisms

- The `FdAction`/`pre_exec` fd-surgery engine: fd 3+ wiring applied in
  source order so later actions may reference earlier ones (`3>f 4>&3`).
- `winstdio`'s swap-save-restore-drop guard, with the startup-stdin
  snapshot distinguishing "fd 0 redirected" from "shell's own stdin".
- Parent-side pipe-end lifetime lessons (a lingering write end starves
  the reader of EOF — the deadlock class, documented at each site).

**Landed (`Stdio::File` slice) 2026-07-21** — rustils#51, filed
immediately after #43–#46 landed: converting `nexus-rush/src/job.rs`'s
`spawn_pipeline` onto `Spawner::spawn` (to actually reach
`GroupSpec::JoinGroup`/`Signal`-based `kill_tree`/`wait_job` — all
`Child`-trait methods, only reachable on a child rustils itself
spawned) hit a real blocker one level up: `Stdio` had no way to wire a
stage's stdin/stdout/stderr to an arbitrary already-open file at all,
which `nexus-rush/src/exec.rs::build_stage`'s shell redirects (`>
file`/`>> file`/`< file`/`2>&1`/`&> file`) need. Added
`Stdio::File(Box<dyn platform::fs::File>)` (mechanism: a spawn-time
`dup2`/`DuplicateHandle`-style wiring onto the child, borrowing rather
than consuming the caller's `File`) and `File::try_clone` (the
`2>&1`/`&> file` shape specifically — two `Stdio::File` slots sharing
one open-file description, position included, which two independent
`Dir::open` calls on the same path cannot substitute for). Forced
`Stdio` to give up `Copy`/`Clone`/`PartialEq`/`Eq` and `Command` to give
up `Clone` (an open OS handle has no honest value-type-copy meaning);
`platform-mock`'s spawn log moved from cloning `Command` to a
`SpawnRecord`/`StdioKind` snapshot type as the direct consequence. See
`docs/behavior/process.md` and `docs/behavior/fs.md` for the full
contract.

### D6 — `rush/src/trap.rs`: the signal-deferral core

Handler = one atomic store; consumption at safe points via `swap(0)`.
Plus the two-tier install policy (TERM/HUP always, others on trap
registration). The `events` domain's signal source starts here. Policy
that stays rush-side: `$?` preservation, re-entrancy guard, subshell
trap-visibility snapshots.

### D7 — rush test machinery: the parity regime's missing pieces

- Black-box conformance shape: `binary -c src` → assert only on
  (stdout, stderr, exit status). This is the form R2's exit criterion
  ("rush's conformance suite green on this layer") should take.
- Subprocess-per-test — sidesteps shared-fd races across test threads.
- Stdin fed from a thread — dodges the fixed-16KB pipe-buffer deadlock
  (macOS) that reads as a hung CI job, not a failed test.
- Instant-exit stand-ins (`cmd /c exit N`) + wall-clock budget as the
  assertion — race-free job-control tests.
- Fuzz only pure stages, no-panic contract; never stages that spawn or
  touch the filesystem.
- Document what is *deliberately not asserted* and why (see rush's
  Windows disown test for the exemplar).
- The PTY harness is reusable in shape; harden its fixed-sleep
  synchronization (prompt-string sync) rather than copying it.

### D8 — Ready-made divergence-registry entries

rush has already characterized, with OS-limitation citations and CI
evidence, entries `docs/divergences.md` is waiting for. When the process
domain lands here, record (with pinning tests): no `fork()` on Windows;
no `fg`/`bg`/Ctrl-Z (no `tcsetpgrp` equivalent); no fd table beyond the
three std slots; completion by polling (no SIGCHLD analog); the ambient-
Job-Object caveat that makes detach-from-job unreliable under CI runners.

### D9 — The Terminal cluster: five donors, one surface (second wave, 2026-07-19; full-ecosystem survey same day)

The strongest signal in the second-wave survey, then confirmed by the
full-ecosystem sweep: **five donors independently hand-rolled the same
terminal personality** — rusty_libc, rusty_win32, rush, then
**rusty_lines** (`src/term_sys.rs`: a three-backend facade over
rusty_libc/libc/rusty_win32 that is nearly a ready-made spec for the
trait) and **shh** (`src/tty/{unix,windows}.rs`). No surface has ever
had stronger consumer-gate evidence.

**rusty_term is the design oracle, not a backend.** It is a full
terminal *emulator* (~47k LOC) whose OS slice (`src/backend/`, ~1.4k
LOC) is already factored as a portable trait over two per-OS unsafe
files — CI-proven on both OSes. Build the rustils trait fresh and port
its *semantics + tests*; do not depend on it (tokio + edition-2024
would invert the layer stack). It converges later by swapping its
backend internals for the PAL trait. What it teaches beyond the
original sketch: raw mode on Windows is **two streams** (stdin VT-input
+ stdout VT-processing); the save/restore lifecycle IS the contract;
**resize notification is a divergence** (SIGWINCH stream vs Windows
timer-poll — no event exists); the pollable-pty-fd vs blocking-thread
asymmetry mirrors pidfd-vs-WFMO.

**Facets beyond raw-mode/winsize/isatty** (each lands when a consumer
forces it):
- *Line-editing support* (rusty_lines): bracketed-paste guard +
  envelope decode, cooked↔raw suspend/resume (`$EDITOR` handoff),
  `is_raw` re-assert + signal-free self-healing (200ms tick instead of
  SIGWINCH — a deliberate design alternative to record), VMIN=1/VTIME=0
  chunked reads, byte→Key decode (portable pure logic).
- *Console acquisition* (rusty_naner `naner-core/console.rs`): the
  GUI-subsystem attach-vs-alloc-vs-redirected personality —
  AttachConsole/AllocConsole/CONOUT$ reopen/enable-VT with a
  `ConsoleState` enum. A wholly separate facet the sketch missed.

- **Raw mode**: `rusty_libc/termios.rs` (`make_raw`, kernel-shape
  `Termios` NCCS=19, `tcgetattr`/`tcsetattr_with`, `tcflush`/`tcdrain`)
  and `rusty_win32/console.rs` (`Get/SetConsoleMode` with the
  `ENABLE_VIRTUAL_TERMINAL_INPUT`/`_PROCESSING` bits) — the two halves
  of one trait.
- **Window size**: `rusty_libc/tty.rs` (`TIOCGWINSZ`) and
  `rusty_win32/console.rs` (`GetConsoleScreenBufferInfo`, viewport not
  scrollback) — cleanly portable.
- **isatty**: `rush/sys.rs` + `builtins.rs` (`test -t`; Unix syscall vs
  Windows `IsTerminal`).
- **Job-control terminal handoff** (Unix): `rush/job.rs`
  `give_terminal`/`reclaim_terminal` over `tcsetpgrp`, sound only with
  SIGTTOU ignored — the *mechanism* half of D1 (the job tables stay
  policy); plus SIGTSTP/SIGCONT suspend-resume plumbing.
- **Test harness for free**: `rusty_win32/console.rs`
  `write_char_events`/`WriteConsoleInputW` — synthetic keystrokes, the
  Windows analog of writing into a pty master; drives a raw-mode reader
  end-to-end in CI. Not yet ported (no forcing consumer for it
  specifically) — the console-*acquisition* landing below is a distinct
  facet from this one.

Consumers per architecture.md: rusty_naner, rush interactive (Phase 5),
rusty_lines' host. Windows fg/bg absence is already characterized (D8).

**Landed (console acquisition slice) 2026-08-05** — the owner's explicit
call, same posture PTY hosting (D13) was built under: no confirmed live
consumer, built speculatively. `platform::term::{ConsoleState,
ConsoleAcquisition}` — `alloc_console`/`attach_console`/`free_console`
over `rusty_win32`'s real `AllocConsole`/`AttachConsole`/`FreeConsole`
primitives, plus a `CONIN$`/`CONOUT$` reopen (promoting
`rusty_win32`'s own test-only `open_console` helper to production code)
and a best-effort VT-processing enable so the rest of
`platform::term::Terminal` works immediately against a freshly acquired
console with no separate fixup step. `ConsoleAcquisition` is a
deliberately separate opt-in trait from `Terminal`, only implemented by
`WindowsTerminal` — the mirror-image asymmetry to `JobControlTerminal`
(Unix-only, no Windows implementor). `rusty_naner`'s own donor source
(the D9 entry above's actual attribution for this facet) wasn't in this
session's reach, so `ConsoleState`'s four variants
(`None`/`Inherited`/`Allocated`/`Attached`) are derived from
`rusty_win32`'s real primitives plus ordinary Win32 domain knowledge,
not ported verbatim — see `docs/design-discussion-console.md` for the
full reasoning and what's deliberately still out of scope (the
`write_char_events` test-harness facet above, `rusty_win32`'s
title/codepage/fill/cursor primitives, `platform-mock`).

### D10 — Wait-status completion: `waitid`/WNOWAIT + stopped/continued

- `rusty_libc/wait.rs`: `waitid` with `WNOWAIT` and a structured
  `Siginfo` (`P_ALL/P_PID/P_PGID`, `CLD_*`) — peek-without-reap,
  strictly richer than the adopted `wait4`; what a job table uses to
  inspect a child and still reap it later.
- `rush/sys.rs` + `job.rs`: `WUNTRACED`/`WCONTINUED` flags and
  `WIFSTOPPED`/`WIFCONTINUED` decode — the Ctrl-Z/fg/bg half of the
  status set; the landed `ExitStatus` covers exit+signal only.
  Unix-only (Windows divergence already in D8's list).

**Landed** 2026-07-21, with D1's job-control slice — see D1's landed note.
`ExitStatus::Stopped(sig)`/`Continued` plus `Child::wait_job`/
`try_wait_job` (`WUNTRACED`/`WCONTINUED`-aware wait, both blocking and
non-blocking, mirroring `wait`/`try_wait`'s pair). `waitid`+`WNOWAIT`
peek-without-reap remains unlanded — no consumer has forced it yet.

### D11 — Fs second wave: mutation layer, predicates, memfd

- `rusty_libc/fs.rs`: the directory-mutation and symlink layer —
  `renameat2` (`RENAME_NOREPLACE`/`EXCHANGE`), `symlinkat`/`readlinkat`,
  `faccessat` — only the read/stat side was adopted in Track P.
- `rush/builtins.rs`: `test`'s file-mode predicates (`-x/-u/-g/-k`,
  owner uid/gid, same-file by dev+ino) and the PATH-resolution logic
  duplicated across `command -v`/`type`/completion — a *unification*
  onto `Spawner::resolve`, not just an extraction.
- **`memfd_create`** (`rush/sys.rs` `memfd_heredoc` +
  `rusty_libc/fd.rs`): the thread-free here-doc — the load-bearing
  invariant that makes a raw `clone(SIGCHLD)` fork sound
  (single-threaded at every fork point). Cited as D4 *rationale*, never
  surfaced as an API. **Lands first if the fork/execve decision goes
  raw.**
- `rusty_libc/fd.rs`: `fcntl`/`dup` family (CLOEXEC and NONBLOCK
  toggling, `F_DUPFD_CLOEXEC`, per-fd pipe-capacity get/set),
  `pread`/`pwrite`; `rusty_win32/handle.rs` pipe/dup/inheritability as
  the Windows counterparts. Feeds D5's remaining fd-3+ engine.

> **Landed (first slice, convergence roadmap Phase 3, 2026-07-19):**
> `renameat2` (replace + `RENAME_NOREPLACE`) and `File::sync_all`
> (`fsync`/`FlushFileBuffers`), plus a default-provided
> `Dir::write_atomic` composed from the two.
>
> **Landed (symlink slice, 2026-07-19):** `symlinkat`/`readlinkat` as
> `Dir::symlink`/`read_link` — target stored verbatim, `read_link`
> round-trips it exactly. Windows needed the reparse-point construction
> this slice's own predecessor deferred (`FSCTL_SET_REPARSE_POINT`/
> `REPARSE_DATA_BUFFER`, a third ntdll-adjacent admission), plus a
> registered divergence for the file-vs-directory decision NT forces at
> creation time that POSIX doesn't (`docs/divergences.md` #004).
>
> **Landed (faccessat slice, 2026-07-19):** `Dir::access` — `faccessat`
> on Linux (real, not effective, uid/gid, matching what Track P's
> flags-less `rusty_libc::fs::faccessat` can support); a trial open on
> Windows. The design pass this slice's predecessor said it needed
> resolved to: Windows has no execute-permission bit on a regular file
> at all, so `execute` is granted unconditionally once existence is
> confirmed — a registered divergence (`docs/divergences.md` #005)
> rather than a forced-uniform answer.
>
> **Landed (test-predicates slice, 2026-07-19):** `Dir::unix_mode`/
> `file_id` — `test`'s `-u/-g/-k/-O/-G` (real mode bits + ownership on
> Linux, `Ok(None)` on Windows: no POSIX model there at all, registered
> divergence `docs/divergences.md` #006) and `-ef` (an opaque, both-
> backends-identical file identity via `fstatat`'s `(dev, ino)` /
> `GetFileInformationByHandle`'s `(volume serial, file index)`). `-x`
> needed nothing new (`Dir::access(rel, AccessMode::execute())` already
> covers it). The PATH-resolution half of this donor item turned out to
> already be done: `Spawner::resolve` has existed in `platform::process`
> since the process-surface work landed; what remains — rush swapping
> its own three duplicated PATH-walkers over to it — is ecosystem-side,
> out of scope here without `rush`'s code in hand.
>
> **Landed (third wave: nlink/mtime/permissions, 2026-07-21)** —
> coreutils gap backlog #63/#64/#65, forced by this repo's own
> `coreutils::ls -l` reference consumer. `Metadata::nlink`/`modified`
> (portable, no `Option`) and `UnixMode::permissions` (the standard
> `rwxrwxrwx` bits — read-only; a `chmod`-equivalent write path is
> #64's still-open remaining half). Live-verified per backend against
> a second, independent source (Linux: raw `libc::stat`; Windows:
> `std::fs::Metadata::modified()` + a raw
> `GetFileInformationByHandleEx(FileStandardInfo, ...)` call, since
> `MetadataExt::number_of_links` is nightly-only). Also added, outside
> `platform::fs` entirely: `platform_linux::{user_name, group_name}`
> (`getpwuid_r`/`getgrgid_r`) backing `coreutils::native`'s uid/gid
> display-name resolution — a directory-service lookup, not filesystem
> metadata, so it doesn't belong on `Dir`/`UnixMode` itself. The
> resulting `rls -l` diffs byte-for-byte identical against real `ls -l`
> (aside from the unduplicated `total N` header) across several stress
> cases. See `docs/behavior/fs.md` and the convergence roadmap's Phase
> 3 entry for the full contract and backend notes.
>
> **Landed (`memfd_create` slice, independent of the fork/execve
> question) 2026-08-06** — `platform::fs::AnonymousFile::create_memfd`,
> option 3 of `docs/decision-request-fork-execve.md`. This is the same
> `memfd_create` D4 cites as the load-bearing invariant for a raw
> `clone(SIGCHLD)` fork to be sound, landed on its own regardless of
> whether that larger, still-parked decision ever resolves toward raw
> — `rusty_libc/src/fd.rs` already had the wrapper (found while digging
> into the fork/execve decision), but this backend uses the plain
> `libc` crate instead (`memfd_create` has had a real libc wrapper
> since long before this workspace's MSRV floor, so no `track-p`
> gate, no rusty_libc dependency, matching `fsync`'s own "one
> implementation for both configurations" treatment). Deliberately not
> a `Dir` method — `memfd_create` takes no path and touches no
> directory capability, so a new, small, `Csprng`-shaped trait instead.
> Windows: `Unsupported`, no numbered divergence (matching
> `WindowsTun`'s own precedent — `CreateFileMappingW`'s anonymous
> mapping is a fixed-size shared-memory *mapping*, not a growable
> byte-stream `File`, so this is "no donor evidence for a shape that
> fits," not a hard OS limitation). See `docs/behavior/fs.md` for the
> full contract.

### D12 — Small process/events donors (each waits for its consumer)

- Single-fd `poll_readable` (`rush/sys.rs`, zero-timeout poll — the
  `read -t 0` primitive; distinct from the multiplexed reactor).
- `umask`, rlimit/ulimit (`rush/sys.rs`+`builtins.rs`,
  `rusty_libc/rlimit.rs`/`umask.rs`) — self-contained, park until a
  builtin-shaped consumer.
- uid/gid getters (`rusty_libc/process.rs`) — brushes the gated
  Security surface.
- `rusty_win32/process.rs` `environment_block` (double-NUL UTF-16,
  order-preserving) — reusable spawn primitive.
- **Time** (`rusty_libc/time.rs`+`vdso.rs`; `rusty_win32/time.rs`): the
  two donors deliberately share a `Timespec` shape — a portable Time
  trait is pre-aligned and nearly free; park until a consumer (e.g. a
  `time` builtin) arrives.

### D13 — PTY: a Process×Terminal capability (full-ecosystem survey)

Two donors: **shh** (`connect/pty.rs`: openpty + slave handoff +
`TIOCSCTTY` session setup in pre_exec + `TIOCSWINSZ` resize + async
master with EIO→EOF) and **rusty_term** (`backend/`: openpty/fork/exec
on Unix; **ConPTY** — CreatePseudoConsole + PROC_THREAD_ATTRIBUTE — on
Windows, incl. the EOF-vs-exit teardown deadlock lesson). Deliberately
NOT part of Terminal slice 1: PTY *hosting* is its own surface, gated
on an emulator/mux consumer. Divergences to register when it lands:
ConPTY vs openpty; pollable master fd vs blocking-thread bridge.

**Landed (part 1/2: trait + Linux backend), 2026-07-23** — built on the
owner's explicit call, still no confirmed consumer (rustils#82); see
`docs/design-discussion-pty.md` and `docs/convergence-roadmap.md`'s
Phase 7 entry for the full reasoning, including why the Linux backend
does *not* port shh's `fork`+`TIOCSCTTY` mechanism as-is (raw `fork`
stays parked behind its own separate roadmap decision).

**Landed (part 2/2: Windows ConPTY backend), 2026-07-23** — rustils#83,
`platform_windows::{WindowsPty, WindowsPtyMaster}`. Both PTY divergences
D13 anticipated (ConPTY vs openpty; pollable master fd vs blocking-thread
bridge) are now registered as `docs/divergences.md` #011, plus the
`posix_spawn`-vs-`fork` substitution #82 needed. CI-verified only — see
`docs/behavior/pty.md` and the roadmap's Phase 7 entry for the full
contract.

### D14 — Tun/virtual-link surface (rusty_tail)

rusty_tail is a Tailscale-style mesh VPN (not a log follower — the
architecture doc's original placement was wrong and is corrected). Its
`ts-tun/src/sys.rs` hand-rolls /dev/net/tun + TUNSETIFF + SIOCSIF*
ioctls behind an anticipated-but-unbuilt `TunDevice` trait (its own
comments defer a wintun backend). A new gated surface with its named
consumer already in hand.

**Landed 2026-07-21** — `platform::tun::{Tun, TunDevice}`, mirroring
`ts-tun/src/sys.rs`'s exact ioctl sequence on Linux (`/dev/net/tun` +
`TUNSETIFF`, then `SIOCSIFADDR`/`SIOCSIFNETMASK`/`SIOCSIFMTU`/bring-up).
Live-verified (not just cross-compile-checked) in this session's
sandboxed environment, which genuinely has `/dev/net/tun` and
`CAP_NET_ADMIN` — confirmed with a raw C probe first, the same
discipline the Sandbox slice's `ENOSYS` finding established: check
before assuming, and say so honestly either way. Ships the same raw-fd
escape hatch (`AsFd`/`AsRawFd`/`set_nonblocking` on the concrete
`LinuxTunDevice`) D16/rustils#41-#42 established for Net, since `ts-tun`
needs to hand its fd to tokio's reactor exactly like `ts-magicsock` did.
`WindowsTun` reports `Unsupported` explicitly — no Windows donor exists
yet, `ts-tun` itself being Linux-only. See `docs/behavior/tun.md` and
the convergence roadmap's Phase 8 entry for the full contract and
backend notes.

### D15 — Security surface donors (nexus, shh, rusty_rdp)

The gated Security surface now has concrete donor material:
- **nexus** `nexus-security/os_sandbox.rs`: Landlock fs confinement +
  seccomp inet-block + the helper-exec model dodging post-fork
  allocation (the D4 landmine again); `credential.rs`: OS-keyring
  vault trait with a disabled mode.
- **shh** `privsep.rs`: fork-before-runtime, socketpair monitor,
  `prctl(NO_NEW_PRIVS)`, setrlimit, credential drop with regain-root
  check.
- **rusty_rdp**: hand-rolled /dev/urandom entropy reads ×5 — a PAL
  CSPRNG/`fill_random` primitive would retire them.

**Landed (CSPRNG slice) 2026-07-20** — `platform::security::Csprng` +
`fill_random`, the first Security surface slice, forced by rusty_rdp's
five hand-rolled `/dev/urandom` reads. Deliberately narrow: one method,
no key derivation, no algorithm choice. Backends draw from the OS CSPRNG
directly rather than opening `/dev/urandom` as a file (Linux:
`getrandom(2)` raw syscall; Windows: `BCryptGenRandom` with the system
preferred RNG) — no `fd` for a future filesystem sandbox (this same
Phase 6's item 3) to have denied. See `docs/behavior/security.md` for
the full contract and `docs/convergence-roadmap.md`'s Phase 6 entry for
backend notes.

**Landed (Sandbox slice) 2026-07-20** — `platform::security::Sandbox`,
mirroring nexus's `os_sandbox.rs` shape exactly (`confine_filesystem` via
raw Landlock syscalls, `block_inet_sockets` via a hand-written seccomp-BPF
filter — two independently-degradable calls, not one, because that's
what nexus's own implementation proved necessary). Built without a
confirmed live consumer, an explicit owner call after
`docs/design-discussion-sandbox.md` surfaced that neither donor's shape
maps cleanly onto a single trait: nexus's need is process confinement,
shh's is privilege separation via fork+socketpair to protect a secret —
a different problem that doesn't fit `platform::process`'s current shape
and stayed out of scope. `CredentialStore` stayed held: nexus's
`CredentialVault` (a complete, working wrapper over the `keyring-rs`
crate) has no gap, no TODO pointing at rustils, and no expressed desire
to migrate — donor material only, the same conclusion
`docs/design-discussion-sandbox.md` reached for `Sandbox`'s own
consumer question before the owner chose to proceed anyway. See
`docs/behavior/security.md` and `docs/design-discussion-sandbox.md`'s
Outcome section for the full contract and reasoning.

### D16 — Net surface shape (shh, rusty_tail, rusty_rdp, rusty_llama)

Four consumers now define Net's shape without guessing: TCP
connect/listen + set_nodelay (shh, rdp, llama's optional server), UDP
datagram (rusty_tail magicsock), Unix sockets incl.
mode-0600-with-stale-cleanup bind (rusty_tail LocalAPI, shh agent).
**No TLS obligation**: shh and rusty_tail hand-roll wire crypto over
plain TCP; rdp's rustls is optional and injected. rusty_rdp converges
cheapest — its `net.rs` is already generic over `Read + Write`.
*Settled 2026-08-02 (rustils#70)*: the injected layer became a crate —
`rusty_tls` wraps a TLS engine behind one seam for the whole ecosystem,
with `rusty_request` and `rusty_rdp` migrated onto it. Net's TLS
obligation stays at zero permanently: no protocol crypto lands here.
Same day, the one non-crypto piece — OS trust-anchor access ("B1") —
gated in and is pending as a `security` slice with `rusty_tls` as
consumer of record, so it lands beside `Csprng`, not in Net. See
`design-discussion-tls.md`'s Resolution and *B1 reopened*.

**Landed (TCP slice) 2026-07-19** — `platform::net::{Net, TcpStream,
TcpListener}`, see `docs/behavior/net.md` and the convergence roadmap's
Phase 5 entry for the full contract and backend notes. UDP datagram and
Unix sockets remain future slices of this same decision.

**Landed (Unix sockets slice) 2026-07-20** — `platform::net::{UnixStream,
UnixListener}` + `Net::unix_connect`/`unix_listen`, the mode-0600-bind,
automatic-stale-cleanup-bind shape rusty_tail's LocalAPI and shh's agent
socket asked for; see the convergence roadmap's Phase 5 entry for the
full backend notes and `docs/behavior/net.md` for the full contract
(both updated for this slice). Unix sockets landed a shared
`net_parity.rs` cross-backend assertion (`assert_unix_behavior`) on
2026-07-20 as well, closing the gap this note originally flagged.

**Landed (UDP datagram slice) 2026-07-20** — `platform::net::UdpSocket`
+ `Net::udp_bind`, the last piece of this donor's original
four-consumer shape (rusty_tail's magicsock). No listener/stream split
unlike TCP/Unix — one connectionless socket, addressed per call via
`send_to`/`recv_from`. See the convergence roadmap's Phase 5 entry and
`docs/behavior/net.md` for the full contract; UDP's own assertion
function is in the shared `net_parity.rs` suite, strace-verified on
Linux including the fire-and-forget send-to-nobody behavior that has
no TCP/Unix equivalent. D16's Net surface is now fully landed across
all three named slices, with shared parity coverage for all three too.

**Landed (`platform-macos` backend, net-only) 2026-07-21** — rustils#48:
a third `Net` implementor, forced the same way D16 itself was — a real
consumer (`rusty_tail`'s `rusty_tokio` async runtime, building a kqueue
reactor backend for macOS/BSD) had no `platform-macos` to sit its socket
lifecycle on and hand-rolled `MacosTcpStream`/`MacosTcpListener`/
`MacosUdpSocket` directly against `libc` a second time — duplicating
`platform-linux`'s sockaddr packing, `SO_REUSEADDR`/`TCP_NODELAY`
handling, and error mapping instead of reusing it. `crates/platform-macos`
mirrors `platform-linux`'s `ffi`→`sys`→trait-impl layering exactly, with
the rustils#41 `AsFd`/`AsRawFd`/`From<OwnedFd>`/`set_nonblocking`/
concrete-constructor surface ported in from day one (the issue's own
request, not a follow-up). Three real BSD syscall differences, none of
them a behavioral divergence at the `platform::net` trait boundary (see
`docs/behavior/net.md`): no `SOCK_CLOEXEC`/`SOCK_NONBLOCK` type flags
(`fcntl(F_SETFD, FD_CLOEXEC)`/`fcntl(F_SETFL, O_NONBLOCK)` after
creation instead), no `accept4` (`accept` + the same post-creation
`fcntl`), and a leading `sin_len`/`sin6_len`/`sun_len` byte on every
sockaddr variant (built via `zeroed()` + field assignment so the extra
field never needs naming). Deliberately net-only — no `fs`/`process`/
`security`/`term`/`signals` slice, since nothing has forced one (RFC v2
§3); scoped down from a "full backend" option specifically to match what
the forcing consumer actually needs. Initially validated only via
`cargo check`/`clippy --target x86_64-apple-darwin` from a Linux host
(the same cross-compile discipline `platform-windows` was originally
developed under) — a dedicated `macos` CI job (real `macos-latest`
runner, scoped to `platform`/`platform-mock`/`platform-macos` rather
than `cargo test --workspace`, since `coreutils`'s bins aren't portable
to macOS yet either) now runs `clippy`/`test` on real hardware on every
push and PR, closing that gap. **Justified the gap immediately**: the
`macos` job's first-ever run found a genuine bug cross-compile-check
alone could never catch — `getpeername`/`getsockname` on an
anonymous/unbound `AF_UNIX` client does not shrink the returned `len`
back to the header-only size the way Linux does, so `from_sockaddr_un`
read the whole zeroed `sun_path` buffer as path content instead of
recognizing there was none, returning `Some("\0\0…")` instead of `None`
(`macos_unix_conforms`, `net_parity.rs`). Fixed by treating an
all-zero `sun_path` as unbound regardless of what `len` reports —
sound because `to_sockaddr_un` already refuses any path containing an
embedded NUL byte, so a real bound path can never come back all-zero.
No `docs/divergences.md` entry: this brings macOS behavior in line
with the documented `platform::net` contract, not a permanent
divergence from it.

**Widened (`platform-macos` → `platform-bsd`) 2026-08-02** — rustils#86,
the direct follow-up to the entry above. `rusty_tokio`#116 wanted the
same `kevent` reactor on FreeBSD/OpenBSD; the reactor was reusable but
the socket layer under it existed only for Darwin, so a *third*
hand-rolled `libc` socket lifecycle was about to be written — the
duplication #48 exists to prevent, one OS over. Crate and public types
renamed (`MacosNet` → `BsdNet`, …), gate widened to
`any(macos, freebsd, openbsd, netbsd, dragonfly)`, PAL group 0.20.0 →
0.21.0.

Zero implementation change, for a reason worth recording: #48's
Darwin-shaped code turns out to be the *intersection* of the BSDs, not
a Darwin special case. Only the `sin_len` family is universal; the
`SOCK_CLOEXEC`/`accept4` absences are Darwin's alone (the other four
BSDs have both), so writing for Darwin had already produced the
portable subset. `ffi::libc_surface` stays that intersection on
purpose. Real FreeBSD and OpenBSD VM CI legs now join the `macos` job
— #48's own bug above being the argument for why `cargo check
--target` alone wasn't acceptable here. NetBSD is compile-checked
only and DragonFly is unverified entirely; both are labeled as such in
`platform-bsd`'s `lib.rs` rather than folded into a green checkmark.

**Landed (`TcpStream::set_read_timeout`) 2026-07-20** — added while
starting the rusty_rdp convergence this entry names as cheapest;
rusty_rdp's `examples/connect.rs` idles a read loop out via
`std::net::TcpStream::set_read_timeout`, a capability this trait had
none for. See the convergence roadmap's Phase 5 entry for the full
backend notes, including a real pre-existing test-flake bug this work
caught and fixed along the way (unrelated to the timeout itself).

**Landed (raw-fd + non-blocking escape hatch) 2026-07-21** — filed as
rustils#41 by rusty_tail's own maintainer, mid-design on `rusty_tokio`
(a hand-rolled async runtime wanting to sit on this crate's
sockaddr-packing/stale-cleanup logic instead of reimplementing it).
`AsFd`/`AsRawFd`/`From<OwnedFd>`/`set_nonblocking` plus concrete
`connect`/`bind`/`accept` constructors on the five Linux socket types —
inherent-impl-only, the object-safe `Net`/`TcpStream`/etc. traits
themselves are unchanged. See the convergence roadmap's Phase 5 entry
for the full backend notes and why the constructors were necessary
(without them, the accessors would be unreachable — `Box<dyn Trait>`
erases the concrete type with no safe way back).

**Landed (Windows raw-socket + non-blocking escape hatch) 2026-07-21**
— rustils#59, the `platform-windows` half of the gap #41 left,
forced by the same `rusty_tokio` consumer scoping a Windows/IOCP
reactor (`rusty_tokio#6`). `AsRawSocket`/`set_nonblocking`
(`ioctlsocket(FIONBIO, ...)`) plus concrete `connect`/`bind`/`accept`
constructors on the five Windows socket types — inherent-impl-only,
same reasoning as the Linux slice. No `AsSocket`/ownership-transfer
interop (Windows's `sysnet::OwnedSocket` is this crate's own newtype,
not std's `std::os::windows::io::OwnedSocket`) — not asked for. See
the convergence roadmap's Phase 5 entry for the full backend notes.

### Cross-cutting notes from the full-ecosystem survey

- **nexus re-derived already-landed rustils work**: its
  `job_object.rs` (kill-on-close Job Objects) and `nexus-rush/job.rs`
  (setpgid×2/tcsetpgrp/SIG_IGN) duplicate D1/D2 — the cheapest, most
  valuable convergence is swapping them for `platform::Process`, which
  exists today.
- **winargv has a second live consumer**: rusty_naner's hand-quoted
  `raw_arg` command lines (launcher.rs) are the BatBadBut class D3
  exists to kill.
- **Atomic durable write** appears twice (nexus `storage/atomic.rs`:
  temp→fsync→rename→fsync-parent with retry classes; rusty_naner's
  download→tmp→rename staged install) — a ready Fs primitive, backed
  by renameat2 (D11) / MoveFileEx.
- **AsyncFd-over-raw-fd** (nonblocking fcntl → readiness reactor →
  guarded I/O with error→EOF) appears independently in rusty_tail's
  TUN and shh's PTY — a reactor-adoption primitive for the Events
  domain.
- **rusty_llama** memory-maps its model (`loader.rs`) — the one mmap
  in the ecosystem; an Fs read-only-map capability candidate (single
  consumer today, so parked).
- **rusty_lsp is the counter-example that validates the gate**: zero
  platform crates; it converges by doing essentially nothing.
- **rusty_whisper / rusty_regx confirmed** as classified: pure compute
  (no mmap, no audio-device capture — do NOT add an audio surface on
  whisper's account) and pure library respectively.

## Not extracted (shell policy, stays in rush)

Expansion, globbing, aliases, trap registry semantics, pipefail /
`PIPESTATUS`, 127/126 mapping, `$!`/`%n` conventions, the self-re-exec
subshell protocol. rustils makes these *expressible*; it does not own
them.

## Suggested sequence

1. **`winargv`** with cmd-rules escaping and refuse-unrepresentable
   (D3) — highest security value; fuzzed per §9.5; hand back to rush.
   **Landed:** `platform-windows/src/winargv.rs` (pure `&[u16]` core,
   tested on both CI legs + Miri) with a `CommandLineToArgvW` round-trip
   oracle incl. an exhaustive adversarial-alphabet sweep on the Windows
   leg (`tests/winargv_oracle.rs`).
   **§9.5 fuzz job landed:** `fuzz/` (own workspace) fuzzes arbitrary
   argv through the builder and an *independent model* of the MSVCRT
   splitting rules (differential — builder and parser cannot share a
   bug); the Windows oracle anchors the model to the real OS, and the
   model's deterministic tests replicate the oracle table. Nightly
   schedule in `.github/workflows/fuzz.yml`.
   **Standalone-reachability landed (convergence roadmap Phase 1c,
   2026-07-19):** moved to its own `crates/winargv` crate (depends only
   on `platform`), re-exported unchanged from `platform-windows`. A
   handback consumer no longer needs the rest of the Windows backend.
   Only the rush/rusty_naner handback PRs themselves remain from this
   step.
2. **Spawn + groups** behind the `Spawner` trait: Unix (D1) and Windows
   suspended-spawn/jobs (D2), with `behavior/process.md` grown to match
   and D8's divergence entries recorded.
   **First slice landed:** `LinuxSpawner` (`posix_spawn` — allocation
   entirely pre-call, no fork critical region in this crate) and
   `WindowsSpawner` (`CreateProcessW` with the command line built
   exclusively by `winargv`), consuming `wait` with decoded status
   (Signaled pinned on the Linux leg), mechanism-level `resolve`
   (PATH+execbit / PATH+PATHEXT), explicit-env and Stdio Null wiring,
   `rrun` as the gating consumer, parity tests on both legs.
   **Second slice landed:** `GroupSpec::NewGroup` + `Child::kill_tree`/
   `kill_single` — `POSIX_SPAWN_SETPGROUP` at-spawn placement on Linux;
   D2's suspended-spawn → assign-to-kill-on-close-Job → resume sequence
   on Windows — with divergence entries 001 (killed-status form) and 002
   (drop-unwaited semantics) recorded and parity-pinned. Remaining from
   D2 for later: disown-style detach (clear-kill-on-close), which waits
   for a consumer that needs it.
3. **Wait-any / reactor seed** (D2's `wait_any` + D6's signal source),
   absorbing the 64-handle limit internally per §5.6.
   **Seed landed:** `Child::try_wait` (WNOHANG / zero-timeout wait, with
   the reaped-status stash) and portable `wait_any` (poll-over-try_wait,
   10ms tick — the same coarser stand-in rush ran before adopting
   `WaitForMultipleObjects`, deliberately contract-first), consumed by
   `rpar` and parity-pinned on both legs.
   **R3 internals landed:** `Spawner::wait_any` (default = the portable
   loop) overridden natively — pidfd_open + `poll` on Linux (raw
   syscall; no libc wrapper at the MSRV baseline; `Unsupported` on
   pre-5.3 kernels falls back to the portable loop), and
   `WaitForMultipleObjects` on Windows with the 64-handle cap absorbed
   in `sys::proc::wait_many` (≤64: one true blocking wait; beyond:
   64-chunk zero-timeout sweeps on a 10ms tick). Parity sweeps 70
   children on both legs — past the Windows cap by construction.
   **Track P closed the raw-syscall gap (Phase 4, 2026-07-19):**
   `poll_pids`'s pidfd-opening step is a track-p-split helper now — the
   `track-p` arm calls `rusty_libc::process::pidfd_open` directly (a
   real wrapper as of rusty_libc PR #19), so the raw `c::syscall`
   escape hatch only remains in the non-track-p arm, where it's still
   unavoidable (no libc wrapper for this syscall at this workspace's
   MSRV either way).
   **D6 signal source landed, completing R3**: `platform::events::
   SignalSource` (single-slot, coalescing, take-at-safe-points — the
   donor's `PENDING_SIGNAL` shape verbatim in spirit), with
   `LinuxSignalSource` (signal(2) handler = one atomic store),
   `WindowsSignalSource` (`SetConsoleCtrlHandler` mapping console
   control events — divergence 003), and the mock. `rpar` assembles the
   §5.6 reactor from the pieces: multiplexed wait ∪ deferred signals ∪
   timeout tick, killing its children and exiting 130/143 on
   Interrupt/Terminate. Policy (trap tables, `$?` preservation,
   re-entrancy guards) stays rush-side per D6's own classification.
4. **Stdio/handle model** (D5) — decide std-slot-swap vs STARTUPINFO
   lists on the record.
   **Landed, with the decision recorded:** `Stdio::Pipe` +
   `Child::take_stdin/stdout/stderr` on all three backends, consumed by
   `rtee`. **Decision: STARTUPINFO handle lists, not rush's std-slot
   swap.** rush swapped the process-global std slots because
   `rusty_win32::spawn_suspended` exposes no per-spawn handle override;
   this backend owns its `CreateProcessW` call, so per-spawn
   `STARTF_USESTDHANDLES` avoids mutating process-global state and the
   restore ordering it forces (winstdio's swap-save-restore-drop guard).
   D5's transferable lessons are kept: only this spawn's handles are
   inheritable (fresh pipes created inheritable-child-end-only via
   `SetHandleInformation`; Inherit slots get inheritable *duplicates*
   closed after the snapshot), CLOEXEC/non-inheritance on every parent
   end so no leaked write end starves a reader of EOF, and
   `ERROR_BROKEN_PIPE`-is-EOF on pipe reads. Remaining from D5: fd-3+/
   extra-handle wiring (`FdAction`-style), which waits for a consumer.
5. **Track P at R4** via D4.

Each step: port semantics + tests, not code links; rush's suites are the
oracle; the consumer gate still applies — coreutils (or rush adapters)
must call what lands.
