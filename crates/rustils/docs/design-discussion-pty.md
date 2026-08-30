# Design discussion — PTY surface (Phase 7, D13)

Not a decision record — this is the design pass `docs/convergence-roadmap.md`'s
Phase 7 entry says to hold before writing any code ("the largest, most
design-sensitive piece" language it borrows from the Sandbox precedent applies
here too, for the same reason: two structurally different donor shapes need
reconciling into one portable trait). Neither donor repo (shh, rusty_term) is
in this session's scope, so this builds on `docs/extraction-map.md`'s D13
record (written by a session that did have donor access) rather than
re-verifying source directly — flagged here so the gap is visible, not hidden.

## Outcome

**Landed 2026-07-23** — the owner's explicit call, same posture as
`CredentialStore` and `Sandbox`'s confinement half: built without a confirmed
live consumer (Phase 7's two "real candidates" — a job-control rush-interactive,
or rusty_naner hosting rusty_term through a PAL PTY — are both still
hypothetical), accepting the same speculative-build risk those two slices were
held for explicitly. Trait shape and backend split below as proposed, with one
refinement made during #83's actual implementation (see the Windows bullet
below): no always-on background thread turned out to be needed after all,
just a bounded drain at teardown — see that bullet for why.

## What D13 already established

Two donors, structurally different:

- **shh** (`connect/pty.rs`): `openpty`, then `fork`; the child calls
  `TIOCSCTTY` in `pre_exec` (before `exec`, while still single-threaded) to
  become session leader and acquire the slave as its controlling terminal,
  then `exec`s. `TIOCSWINSZ` resizes. The parent's master fd is read
  asynchronously; the slave side closing surfaces as `EIO` on the master,
  which shh's own code translates to EOF rather than propagating the raw
  errno.

  **Not reusable as-is here**: shh's `fork`+`pre_exec` shape is raw
  `fork`/`execve`, which this repo's own roadmap has parked behind a
  separate, still-undecided owner call ("Parked: fork/execve vs
  posix_spawn" — `docs/convergence-roadmap.md`) precisely because it
  reopens the async-signal-safety hazard `sys::spawn`'s existing
  `posix_spawn`-only design was built to close by construction (RFC v2 §5.4,
  `sys/spawn.rs`'s own module doc). Raw `fork` for PTY hosting would be
  the same hazard reopened for one more slice — not something to
  reintroduce silently just because PTY is where a donor happens to use
  it. See "the posix_spawn substitute" below for what this crate does
  instead to reach the same outcome (session leader + controlling
  terminal) without `fork`.
- **rusty_term** (`backend/`): the same `openpty`/`fork`/`exec` shape on
  Unix. On Windows: `CreatePseudoConsole` + a `PROC_THREAD_ATTRIBUTE_LIST`
  carrying `PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE`, passed to `CreateProcessW`
  via `STARTUPINFOEX` — the pseudo-console is wired to the child at process
  *creation* time, not attached after the fact. D13 also flags a specific
  lesson already paid for: an **EOF-vs-exit teardown deadlock** — closing
  the pseudo console before the reader has drained the last of the child's
  output can hang, because `ClosePseudoConsole` blocks until its internal
  conhost pipe reader thread exits, which itself may be blocked on a read
  the caller hasn't serviced yet.

## The shape question this document resolves

**Is "open a PTY" and "attach a process to it" two separable steps, or one
atomic operation?**

Unix *can* separate them (a caller could `openpty` now, `fork`/`TIOCSCTTY`
later). Windows structurally cannot: `PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE`
only exists as an argument to `CreateProcessW` itself — there is no Win32 call
that attaches an already-created pseudo console to an already-running,
separately-created process the way [`Spawner::adopt`](../crates/platform/src/process.rs)
(rustils#47) adopts an already-running pid into a Job Object. A portable trait
offering a Unix-only "open" step and a separate "attach" step would either
leave the attach step `Unsupported` on Windows forever (a trait method every
consumer must special-case) or invite a caller to write Unix-only code against
a nominally-portable API.

**Decision: one atomic `spawn`.** `platform::pty::Pty::spawn(cmd, size)`
opens a fresh pty pair *and* spawns `cmd` attached to its slave side in one
call, returning `(Box<dyn PtyMaster>, Box<dyn Child>)`. This matches what both
donors actually do in practice — shh's `openpty`+`fork`+`TIOCSCTTY`-in-
`pre_exec` sequence is just as atomic as ConPTY's, in the sense that nothing
useful happens with the fds/handles in between — and it reuses
`platform::process`'s existing [`Command`]/[`Child`] types rather than
inventing parallel ones: `Command`'s `argv`/`cwd`/`env`/`group` all apply
unchanged to a PTY-hosted child, only its `stdin`/`stdout`/`stderr` fields are
moot (the pty slave *is* all three).

## Master I/O: one contract, two internal shapes

**Decision: `Ok(0)` at EOF, matching `crate::fs::File`/`Terminal::read_chunk`'s
existing convention** — not a raw errno, not an OS-specific sentinel. Unix's
`EIO`-on-slave-close and Windows's `ERROR_BROKEN_PIPE`-on-child-exit both
collapse to the same `Ok(0)`, exactly the translation shh's own code already
does by hand. A caller reading a PTY master should not need an `if cfg!
(unix) { ... } else { ... }` around its EOF check.

Internally the two backends stay different, and that's fine — divergence, not
a shared contract:

- **Linux**: the master fd is pollable (a real fd from `posix_openpt`/
  `/dev/ptmx`), so [`PtyMaster`] gets the same raw-fd escape hatch
  (`AsFd`/`AsRawFd` on the concrete `LinuxPtyMaster`, not the object-safe
  trait) that `Net`/`Tun` established (rustils#41/#42, #77-adjacent Tun
  precedent) — a consumer needing to register it with an async reactor can.
- **Windows**: ConPTY's master side is a pair of anonymous pipes
  (`CreatePipe`), which are **not** waitable/pollable the way a socket
  handle is — this is the "blocking-thread bridge" D13's own text
  flagged as the expected Windows divergence. **Refined during
  implementation**: `PtyMaster::read`/`write`'s own contract is already
  synchronous/blocking (matching Linux's shape exactly), so no
  background thread turns out to be needed for I/O itself —
  `WindowsPtyMaster::read`/`write` are ordinary blocking
  `ReadFile`/`WriteFile` calls, just like `LinuxPtyMaster`'s. The only
  place a thread was ever going to be *load-bearing* is teardown: closing
  a pseudo console (`ClosePseudoConsole`) blocks until conhost's internal
  writer thread finishes, which can deadlock against an un-drained output
  pipe. Rather than a permanent background reader thread for the whole
  session, `sys::pty::close` does a bounded, non-blocking drain
  (`PeekNamedPipe` to check for data without blocking, `ReadFile` only
  when there's something to read, a short overall time budget) right
  before calling `ClosePseudoConsole` — solving the actual hazard with no
  thread and no `Send`-across-threads handle-ownership complexity at all.
  `WindowsPtyMaster` has no single-handle `AsHandle`/`AsRawHandle` impl
  (there is no single handle — see divergence 011 in
  `docs/divergences.md`), just two named accessors
  (`input_handle`/`output_handle`), each documented non-pollable for the
  same anonymous-pipe reason.

## The `posix_spawn` substitute for `fork`+`TIOCSCTTY`

`sys::spawn` already reaches for `posix_spawn` specifically to keep every
allocation in the parent, before the call — `fork`+`pre_exec` is exactly the
async-signal-safety shape it was built to avoid, and reopening that gap for
one more slice isn't this issue's call to make unilaterally. The good news:
the CTTY-acquisition *outcome* shh's code gets from `fork`+`TIOCSCTTY` has a
`posix_spawn`-native path that needs no new attribute type and no `fork`:

- `posix_spawnattr_setflags(POSIX_SPAWN_SETSID)` (a glibc extension since
  2.24, `libc::POSIX_SPAWN_SETSID`) makes the child call `setsid()` — become
  a new session leader with no controlling terminal — before its file
  actions run and before `exec`, the same point in the sequence
  `POSIX_SPAWN_SETPGROUP` already runs at (`sys/spawn.rs`'s existing
  pgroup-at-spawn code, D1).
- `posix_spawn_file_actions_addopen` (already used in `sys/spawn.rs` for
  `Stdio::Null`) opens the slave pty's **pathname** — not a `dup2` of an fd
  the parent already has open — for the child's fd 0, then `adddup2`s it
  onto 1 and 2. Opening a terminal device by path, without `O_NOCTTY`, from
  a session leader that has no controlling terminal yet, is standard
  POSIX/Linux behavior that assigns it as the controlling terminal
  automatically — the exact effect `TIOCSCTTY` gives explicitly, reached
  here through `posix_spawn`'s own file-actions mechanism instead.

Net effect: the child ends up session-leader, with the pty slave as its
controlling terminal and its stdio, having gone through `sys::spawn`'s
existing `posix_spawn` call with one added attribute flag and a different
set of file actions — no new `fork` call anywhere in this crate, no
new async-signal-safety surface. This needs live verification, not just
inspection, before it's trusted: the implementing PR checks the spawned
child's actual session id and controlling terminal (e.g. spawning `sh -c
'tty'` and confirming the printed path matches the slave), not just that
`posix_spawn` returned success.

## Resize

`PtyMaster::resize(&self, size: WinSize)` reuses [`platform::term::WinSize`]
rather than inventing a parallel size type — `TIOCSWINSZ` on Linux,
`ResizePseudoConsole` on Windows. No resize *notification* (SIGWINCH-stream)
surface here, matching `platform::term`'s own documented scope split (D9):
that's a distinct, already-deferred facet, not folded into this slice either.

## What stays out of scope

- **macOS/BSD**: no `platform-bsd` PTY backend yet — no donor evidence for it
  (D13 only surveys shh/rusty_term, both Linux+Windows). Reports
  `ErrorKind::Unsupported`, the same posture `WindowsTun` took for its own
  missing-donor gap (Phase 8).
- **Job-control integration** (making the pty-hosted child's process group the
  terminal's foreground group via `tcsetpgrp`) — `platform::term::JobControl`
  already exists as its own trait (D9); a PTY-hosting shell consumer composes
  the two rather than this slice reinventing job-control handoff.
- **Resize notification** (SIGWINCH-stream) — as above, an already-deferred
  `platform::term` facet, not new scope here.
- **`platform-mock`'s `MockPty`**: scriptable like `MockTun`, not a real pty —
  a test queues master-side bytes to "arrive" and asserts on what the spawned
  mock child "received", since there's no real kernel pty to simulate over
  (mirrors `MockTun`'s own no-real-kernel-routing rationale, Phase 8).
