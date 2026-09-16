# Behavior Spec — pty (Pty, PtyMaster)

RFC v2 R5+, decision D13, convergence roadmap Phase 7. Built without a
confirmed live consumer, the owner's explicit call — the same posture
`security::CredentialStore`/`security::Sandbox`'s confinement half were
built under (`docs/behavior/security.md`). Design pass held first per
the roadmap's own "expect an RFC-level discussion" instruction for this
phase; see `docs/design-discussion-pty.md` for the full donor-shape
reconciliation (shh's `connect/pty.rs`, rusty_term's `backend/`) this
trait's shape is drawn from.

One atomic `Pty::spawn(cmd, size)` opens a fresh pty pair and spawns
`cmd` attached to its slave side in one call — not a separate open/
attach pair. Windows's ConPTY (issue #83, not yet landed) structurally
cannot attach an already-created pseudo console to an already-running
process, so this trait never offers a Unix-only step a Windows backend
would have to leave permanently `Unsupported`.

**No raw `fork`.** The Linux backend
(`platform_linux::sys::pty::spawn_attached`) reaches shh's donor
`fork`+`TIOCSCTTY` outcome — the child ends up a session leader with the
pty slave as its controlling terminal — entirely through `posix_spawn`:
`POSIX_SPAWN_SETSID` (a glibc extension) makes the child call `setsid()`
before its file actions run, then a file action opens the slave **by
pathname** (not a `dup2` of an already-open fd) for fd 0. Opening a
terminal device by path, without `O_NOCTTY`, from a session leader with
no controlling terminal yet is standard POSIX/Linux behavior that
assigns it as the controlling terminal automatically. Raw `fork`+
`pre_exec` (shh's own mechanism) was deliberately not used: it reopens
the async-signal-safety hazard `sys::spawn`'s existing `posix_spawn`-only
design was built to close, and that tradeoff is parked behind its own
separate, still-undecided roadmap item ("Parked: fork/execve vs
posix_spawn") — not this issue's call to reopen. See
`docs/design-discussion-pty.md`'s "The `posix_spawn` substitute for
`fork`+`TIOCSCTTY`" section for the full reasoning.

Live-verified on Linux (`crates/platform-linux/tests/pty.rs`), including
the one claim that's easy to get wrong by inspection alone: a real
spawned child's own `/proc/<pid>/stat` is read back — kernel ground
truth, not this crate's own reporting — to confirm `sid == pid` (session
leader) and `tty_nr != 0` (a controlling terminal is actually set), not
just that `posix_spawn` returned success. Also covered: master↔child I/O
round-tripping through the pty's line discipline (write, read back local
echo plus the child's own output), `Ok(0)` at EOF after the child exits,
and `resize` visible to the child's own `stty size` query.

The Windows backend (`crates/platform-windows/tests/pty.rs`) is
CI-verified only — this crate's whole backend is developed from a Linux
host against `cargo check --target x86_64-pc-windows-gnu`
(`platform-windows/src/lib.rs`'s own module doc), so nothing in it has
run outside CI's `windows-latest` leg. Covered there: a real spawned
child's output arriving on the master, master→child input round-tripping
through `cmd`'s own `set /p`, `Ok(0)` at EOF, a real `ResizePseudoConsole`
call, and — the load-bearing test — dropping a master that was never
drained, against a child producing far more output than a pipe's default
buffer holds, to exercise the teardown-deadlock fix for real rather than
just by inspection.

## Specified

- `Pty::spawn(cmd, size)` opens a fresh pty pair and spawns `cmd`
  attached to the slave side as fd 0/1/2, returning
  `(Box<dyn PtyMaster>, Box<dyn Child>)`. `cmd.argv`/`cmd.cwd`/`cmd.env`
  apply unchanged from `platform::process::Command`; `cmd.stdin`/
  `cmd.stdout`/`cmd.stderr` are ignored (the pty slave is all three,
  unconditionally).
- `cmd.group`: `GroupSpec::Inherit` and `GroupSpec::NewGroup` are both
  accepted (and behave identically — a pty-hosted child is
  unconditionally a fresh session leader, which makes it a fresh
  process-group leader too, by definition).
  `GroupSpec::JoinGroup(_)` is a real `Err(InvalidInput)`, checked
  before any OS call is attempted: there is no way to host a child on a
  fresh pty and also place it into an existing, different process
  group — the mechanism that gives it the pty as its controlling
  terminal (`setsid`) is exactly what rules that out.
- `PtyMaster::read`/`write` are blocking, matching a real pty master
  fd's own semantics. `read` returns `Ok(0)` at EOF (the slave side
  closed because the child exited) — never a raw `EIO`/broken-pipe
  error, matching `crate::fs::File::read`/`Terminal::read_chunk`'s
  existing convention. This is a real behavioral translation on Linux
  (the kernel reports `EIO`), not a passthrough.
- `PtyMaster::resize(size)` updates the pty's window size
  (`platform::term::WinSize` — rows/cols, no new size type), visible to
  the child the next time it queries its terminal size (`stty size`/
  `TIOCGWINSZ`), the same way a real terminal resize would be.
- Linux (`platform_linux::{LinuxPty, LinuxPtyMaster}`): real
  `posix_openpt`/`grantpt`/`unlockpt`/`ptsname_r` pty pair,
  `posix_spawn`-based attach per this doc's summary above,
  `ioctl(TIOCSWINSZ)` for resize. `LinuxPtyMaster` additionally provides
  `AsFd`/`AsRawFd` on the concrete (non-boxed) type — the same raw-fd
  escape hatch rustils#41/#42 established for `Net`/`Tun`, for a
  consumer that wants to register the master fd with its own reactor
  rather than drive I/O through this trait's blocking calls.
- Windows (`platform_windows::{WindowsPty, WindowsPtyMaster}`,
  rustils#83): `CreatePseudoConsole` + a `PROC_THREAD_ATTRIBUTE_LIST`
  carrying `PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE`, passed to
  `CreateProcessW` via `STARTUPINFOEXW`/`EXTENDED_STARTUPINFO_PRESENT`
  — the only way to wire a pseudo console to a child at all, since there
  is no Win32 call to attach one after the fact. Not grouped: no Job
  Object is created or assigned, so `kill_tree` on a pty-hosted `Child`
  is `Unsupported` on Windows — a deliberate scope reduction (a pty
  session is unconditionally its own session on Linux; this backend
  does not yet mirror that with `kill_tree` semantics). Deliberately
  **not** the suspended → assign → resume sequence `Spawner::spawn`'s
  `GroupSpec::NewGroup` path uses, matching Microsoft's own ConPTY
  sample, which creates the process running with no suspend step.
  `STARTUPINFOEXW.dwFlags` also sets `STARTF_USESTDHANDLES` (with null
  std handles): live CI testing found that without it, every child's
  real console output reached the *calling* process's own ambient
  console instead of the pseudo console's pipes — not a timing race,
  but 100% reproducible regardless of shell-vs-no-shell, Job Object
  presence, or `CREATE_SUSPENDED`. The actual cause (confirmed against
  `microsoft/terminal` discussion #15814): when the spawning process's
  own stdio is itself redirected — exactly `cargo test`'s situation
  under a CI runner — the kernel duplicates those redirected handles
  into the child by default even with `PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE`
  set; `STARTF_USESTDHANDLES` with null std handles suppresses that
  duplication. `resize` calls `ResizePseudoConsole`.
  `WindowsPtyMaster::read`/`write` are
  ordinary blocking `ReadFile`/`WriteFile` on the two pipe handles
  ConPTY's master side actually is (see divergence 011). `ERROR_BROKEN_PIPE`
  on read collapses to `Ok(0)`, the existing translation
  `sys::fileio::read` already performs for every other pipe in this
  backend, reused unchanged rather than re-implemented.

  Making that `Ok(0)` actually happen once the child exits — the
  portable contract `platform::pty::PtyMaster::read`'s own doc promises
  ("Windows's broken-pipe-on-child-exit" collapsing to it, matching
  Unix) — needs one background thread ConPTY doesn't give for free.
  Unlike a Unix pty slave, which the kernel closes automatically once
  its last holder (the child) exits, ConPTY's output pipe stays open
  until `ClosePseudoConsole` is explicitly called; live testing
  confirmed reads block indefinitely even after the child has
  genuinely already exited. `WindowsPty::spawn` installs
  `sys::pty::spawn_exit_watcher`, which waits on a *duplicated* process
  handle (independent of `WindowsChild`'s own, so the two owners don't
  fight over one handle's lifecycle) and calls bare `ClosePseudoConsole`
  once the child exits — deliberately *not* the draining close `Drop`
  itself uses (below): an earlier version had the watcher drain first
  too, and live CI caught the real cost of that — the watcher's own
  drain raced a caller's concurrent `read()` on the same handle for the
  same bytes, and *won* often enough to break three previously-passing
  tests down to only conhost's VT-negotiation bytes. Calling bare
  `ClosePseudoConsole` has no such race (the watcher never touches the
  output pipe at all — a pending or future `ReadFile` unblocks
  naturally once conhost's own write-side duplicate closes). A shared
  `closed` flag (compare-exchange) guards against a double-close race
  with `WindowsPtyMaster::drop` — whichever of "the child exits" or
  "the caller drops the master" happens first performs the real close;
  the loser is a no-op. The trade-off moves rather than disappears:
  `ClosePseudoConsole` can itself block if conhost's writer is stuck
  behind a full, unread pipe, so the watcher thread can stall in that
  case until the caller's own reads relieve the backpressure —
  acceptable since it's a detached, never-joined thread nothing else
  waits on.

  Dropping a `WindowsPtyMaster` drains its output pipe (a bounded,
  non-blocking `PeekNamedPipe` loop) before calling
  `ClosePseudoConsole`, avoiding a real deadlock: `ClosePseudoConsole`
  blocks until conhost's internal writer thread finishes, which can
  itself be blocked writing into a pipe nobody is reading. See
  `docs/design-discussion-pty.md` and divergence 011 for the full
  reasoning, including why the master here is two named handles
  (`input_handle`/`output_handle`) rather than a single `AsHandle`/
  `AsRawHandle` impl.
- `platform-mock`'s `MockPty`/`MockPtyMaster`: scriptable, not a real
  pty — `MockPtyMaster::queue_inbound` queues bytes for a future
  `read()` to hand back (standing in for "the child wrote this to the
  slave"), every `write()` is recorded (`written_chunks()`) for a test
  to assert against, and `resize` just updates an in-memory value
  (`current_size()`). The spawned `Child` is a trivial
  already-succeeded stand-in — this mock exists to exercise the master
  I/O contract, not process lifecycle. Enforces the same
  `GroupSpec::JoinGroup` rejection the real backends do, so consumer
  logic can be tested against it without a real OS.

## Deliberately unspecified

- macOS: no backend — no donor evidence (D13 only surveys shh/
  rusty_term, both Linux+Windows).
- Job-control terminal handoff (`tcsetpgrp` on the pty-hosted child) —
  already exists as `platform::term::JobControl` (D9); a consumer
  composes the two rather than this slice reinventing it.
- Resize *notification* (a SIGWINCH-stream analog) — an already-deferred
  `platform::term` facet (D9), not new scope here.
- Exact byte-for-byte terminal emulation behavior (what a specific
  escape sequence does, cursor semantics, scrollback) — this trait is a
  raw pty transport, not a terminal emulator; that's entirely a
  consumer's concern (e.g. a future rusty_term convergence).
