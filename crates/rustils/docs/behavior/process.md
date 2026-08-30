# Behavior Spec — process (Command / Spawner / Child)

**Capability identity & maturity** (#114): `rustils.process.spawner` —
Stable; `rustils.process.child` — Stable; `rustils.process.wait_any` —
Trial (native reactor landed on Linux/Windows only, see this doc's Async
path section); `rustils.process.group_handle` (`adopt`) — Trial
(rustils#47, Windows-only real support, Unix always `Unsupported` by
design); `rustils.process.detach`/`.liveness` (`Command::detach`,
`Spawner::is_alive`/`is_zombie`) — Trial (`docs/decision-request-
detach-liveness.md`, no confirmed live consumer yet — landed ahead of
the RFC v2 §3 consumer gate by explicit owner override, see that
document's Outcome). See `docs/rfc-v2.md` §10 for the numbered decision
log (D-7).

The semantics the parity suite asserts for every backend implementing
`Spawner`/`Child`. Backends: `platform-mock` (scripted; unit tests),
`platform-linux` (`posix_spawn`), and `platform-windows`
(`CreateProcessW` over `winargv`) — the native pair extracted per
`../extraction-map.md` step 2 (first slice: spawn/wait/resolve; groups,
kill-tree, pipes, and wait-any follow). The parity tests live in each
backend crate's `tests/parity.rs` with OS-specific fixtures and mirrored
assertions.

## Specified

- `Command.argv` is a list of discrete arguments end to end. Any joining
  or quoting an OS requires is backend-internal and never caller-visible;
  a backend that cannot represent an argument list faithfully must refuse
  to spawn, not approximate (the BatBadBut class — RFC v2 §5.4).
- `cwd` is always explicit. There is no inherit-ambient-cwd variant;
  consumers own cwd policy.
- `EnvSpec::Inherit` passes the parent environment unchanged;
  `EnvSpec::Explicit` starts empty — nothing leaks from the parent.
- `ExitStatus` is decoded uniformly: `Code(n)` for a normal exit,
  `Signaled(n)` for signal termination. A raw `waitpid` status word never
  crosses the API boundary (pins scaffold bug B-5 — the parity suite's
  permanent sentinel). `Signaled` is never produced on Windows.
- `ExitStatus::success()` is true for `Code(0)` only.
- `Child::wait` consumes the child: double-wait — and with it the
  wait-after-close bug class (B-4) — is unrepresentable.
- `Spawner::resolve` applies mechanism-level lookup only (PATH + exec bit
  on unix; PATH + PATHEXT on Windows). Shell policy — builtin precedence,
  shebang emulation — lives in consumers.
- `resolve` of an unknown program fails `NotFound` with the program as
  path context.

- `GroupSpec::NewGroup` places the child in a fresh group *before its
  first instruction executes* (`POSIX_SPAWN_SETPGROUP`; suspended-spawn →
  Job-Object assign → resume) — there is no window in which the child or
  a fast-spawned grandchild escapes the group.
- `GroupSpec::JoinGroup(pgid)` places the child straight into an
  *existing* group, the same race-free way — D1's pipeline shape, where
  stage 2..n join the first stage's pgid instead of each leading its
  own. Unix only: Windows has no numeric process-group id a spawn can
  join (Job Objects are handle-based), so `Spawner::spawn` fails
  `Unsupported` for `JoinGroup` on that backend — divergence **008**.
- `Child::kill_tree` requires `NewGroup` or `JoinGroup` and fails
  `Unsupported` otherwise (the only alternative target is the parent's
  own group, which is never what the caller meant). `kill_single`
  always works (subject to the `Signal` restriction below).
- `Child::kill_tree`/`kill_single` take a portable `Signal`
  (`Term`/`Int`/`Hup`/`Quit`/`Kill`/`Stop`/`Cont`) — D1's `kill`/`fg`/
  `bg` builtins need more than a hardcoded SIGKILL on an already-running
  child. `Signal::Kill` is the only identity guaranteed on every
  backend; Windows fails every other `Signal` with `Unsupported` — there
  is no OS mechanism to deliver an arbitrary signal to a process here —
  per divergence **008**.
- A killed child must still be `wait`ed; the status it reports is
  OS-divergent — `Signaled(9)` vs `Code(1)` — per divergence **001**.
- Dropping an un-waited `NewGroup`/`JoinGroup` child: tree terminates on
  Windows (kill-on-close), keeps running on unix — divergence **002**.
- `Child::wait_job`/`try_wait_job` are the `WUNTRACED`/`WCONTINUED` half
  of wait (D10): they observe `ExitStatus::Stopped(sig)` (Ctrl-Z) and
  `ExitStatus::Continued` (`SIGCONT`) transitions in addition to the
  plain `Code`/`Signaled` pair — `wait`/`try_wait` never request those
  flags and so can never produce `Stopped`/`Continued`. Neither method
  consumes `self`: a `Stopped`/`Continued` result is not terminal, so
  the caller keeps the child and may call again; a terminal result IS
  stashed exactly like `try_wait` does, so a later `wait()`/`try_wait()`
  returns it directly. Unix only — `Unsupported` on Windows, which has
  no job-control stop/continue analog (already characterized as part of
  divergence-registry-adjacent D8).
- `Child::try_wait` never blocks and never loses a status: once it
  reports `Some`, every re-poll and the eventual consuming `wait` report
  the same status (unix `WNOHANG` reaps the zombie; the backend stashes
  the decoded result).
- `wait_any` returns the index of *a* terminated child or `None` on
  timeout; an empty set is `InvalidInput`. Which index wins when several
  have terminated is unspecified. The free function is the portable
  10ms poll-over-`try_wait` tick; `Spawner::wait_any` (same contract)
  is the backend multiplexer — pidfd+`poll` on Linux,
  `WaitForMultipleObjects` on Windows with the 64-handle limit absorbed
  internally — falling back to the portable loop for foreign children
  or a pre-pidfd kernel.
- `Stdio::Pipe` + `take_stdin`/`take_stdout`/`take_stderr`: each yields
  `Some` exactly once. Reads on a captured end return 0 at end-of-file —
  which arrives when every write-side copy has closed (on Windows,
  `ERROR_BROKEN_PIPE` on a pipe read *is* EOF and is decoded as 0, not
  an error). Dropping the stdin end delivers EOF to the child. **The
  deadlock contract**: drain captured output to EOF (or drop it) before
  blocking in `wait` — a child blocked writing a full pipe never exits —
  and never let a parent-side copy of a write end leak into another
  child (the backends guarantee their own ends don't: CLOEXEC on unix,
  explicit non-inheritance on Windows).
- `Stdio::File(file)` (D5, rustils#51 — forced by
  `nexus-rush/src/exec.rs::build_stage`'s shell redirects: `> file`,
  `>> file`, `< file`, `2>&1`, `&> file`): wires the slot to an
  already-open `File` the caller provides, ownership moving into the
  `Command`. Mechanism only — the backend duplicates the given file's
  fd/handle onto the child's stdin/stdout/stderr at spawn time (Unix:
  `posix_spawn_file_actions_adddup2`; Windows: an inheritable
  `DuplicateHandle` assigned via `STARTUPINFO`) without consuming or
  closing the caller's own `File`, which keeps working after `spawn`
  returns (or after `Command` itself drops, if the caller extracted no
  reference to it beforehand). `2>&1`/`&> file`-style duplication —
  stdout and stderr both landing correctly in one target rather than
  clobbering each other — needs [`crate::fs::File::try_clone`]
  (`docs/behavior/fs.md`), not two independent `Dir::open` calls on the
  same path: only `try_clone` shares the file's position the way a
  real `dup2` pair does.
  `Spawner::spawn` fails `Unsupported` for a `Stdio::File` whose `File`
  wasn't produced by that same backend (a foreign backend's `File`
  impl) rather than guessing how to extract a raw handle from it —
  pinned by a dedicated test per backend
  (`linux_stdio_file_refuses_a_foreign_backend_file` / the Windows
  copy), not the shared assertion set.
- `Command` is not `Clone` (a `Stdio::File` slot owns an open OS handle
  with no honest "clone for a log/re-spawn" meaning) and `Stdio` is not
  `Copy`/`Clone`/`PartialEq`/`Eq` for the same reason — consumers that
  need to distinguish `Stdio` variants use `matches!`, not `==`.
  `platform-mock::MockSpawner`'s spawn log (`spawned`) reflects this:
  it stores `SpawnRecord`/`StdioKind` snapshots (which wiring kind was
  requested, not the `Command`/`Stdio` values themselves), not clones
  of the original `Command`.

- `Spawner::adopt(pid)` (rustils#47): places an already-running process
  this `Spawner` did not itself spawn (e.g. one created by a
  third-party library — `portable-pty::Child::process_id()` is the
  forcing case) into a fresh kill-on-close group, returning a
  [`GroupHandle`] — narrower than `Child`, `kill_tree`/`kill_single`
  only, since an adopted pid was never spawned through this crate and
  `wait`/stdio have no meaning here. Windows: `OpenProcess` + a fresh
  kill-on-close Job Object (`AssignProcessToJobObject`) — the same
  mechanism `GroupSpec::NewGroup` uses at spawn time, applied after the
  fact. Unix: always `Unsupported` — POSIX `setpgid(pid, pgid)` can
  only retarget a process that is both the caller's own child *and*
  has not yet exec'd, which is never true by the time a caller has a
  pid to adopt — divergence **010**. `platform-mock` succeeds
  unconditionally (no real OS process to fail against, same "no OS
  limitation to model" stance `kill_single` already takes for every
  `Signal`), logging each call to `MockSpawner::adopted` for
  assertions, mirroring `spawned`.

- `Command::detach()` (`docs/decision-request-detach-liveness.md`): the
  child survives this process exiting, including crashing, and its
  terminal closing. Unix: `POSIX_SPAWN_SETSID` — the child becomes a new
  session **and** process-group leader (`pid == sid == pgid`) before its
  first instruction runs, giving `kill_tree` a sound target with no
  `NewGroup` needed. Windows: `CREATE_NEW_PROCESS_GROUP |
  DETACHED_PROCESS` — no console, not a member of this process's Ctrl-C
  group. Composes **only** with `GroupSpec::Inherit`; every other
  `GroupSpec` is refused at spawn time, on every backend, for two
  unrelated real reasons: `JoinGroup` (`InvalidInput`, every backend) —
  a fresh session cannot also join an existing, different pgid,
  self-contradictory rather than an OS limitation; `NewGroup`
  (`Unsupported`, every backend) — Linux: `setsid` always makes the
  child a session leader, and `setpgid(2)` forbids changing a session
  leader's process group ID even as a self-targeting `setpgid(0, 0)`
  no-op, so the underlying `posix_spawn` would fail `EPERM` outright
  (confirmed against a real kernel); Windows: a kill-on-close Job Object
  is torn down — killing every member — the instant every handle to it
  closes, which happens unconditionally when this process terminates
  for any reason, silently defeating the guarantee `detach` promises.
  Both refusals land on the same observable outcome, so this is
  documented here rather than in `docs/divergences.md` — that registry
  is for cases where backends genuinely differ.

- `Spawner::is_alive(pid)`: liveness for any pid, not tied to a `Child`
  this backend spawned. Unix: `kill(pid, 0)` — `Ok(true)` on success or
  `EPERM` (exists, not signalable by this process), `Ok(false)` on
  `ESRCH`. Windows: `OpenProcess` + `GetExitCodeProcess` —
  `Ok(true)` if `STILL_ACTIVE` or the open failed with anything other
  than "no such process", `Ok(false)` otherwise. A zombie (Linux,
  unreaped) reports `true` here — `kill(pid, 0)` cannot distinguish
  "running" from "exited but unreaped"; see `is_zombie` for that
  distinction. `platform-mock` has no real OS process table for an
  arbitrary pid: it reports whatever the test declared via the
  externally-mutable `MockSpawner::live_pids` registry (`false` for any
  pid not explicitly inserted).

- `Spawner::is_zombie(pid)`: Linux — `/proc/<pid>/stat`'s state field is
  `Z`; a missing `/proc` entry means the pid doesn't exist at all
  (`Ok(false)`, not a zombie, not an error). Windows: always
  `Unsupported` — divergence **015**, no zombie concept exists (a
  process handle stays valid and queryable indefinitely after exit).
  `platform-mock`: same registry shape as `is_alive`
  (`MockSpawner::zombie_pids`).

## Async path (rusty_foundation_akb §9)

`Child::wait` is the potentially-blocking operation here (`try_wait` is
explicitly non-blocking, see Specified above). The R3 reactor promised in
`process::wait_any`'s doc comment (RFC v2 §5.6) has landed for the two
backends where it matters:

- **Linux** (`LinuxSpawner::wait_any`): pidfd + `poll` — no worker thread
  is occupied while waiting; falls back to the portable tick-loop only
  when a child isn't a `LinuxChild` (foreign `Box<dyn Child>`) or the
  kernel predates `pidfd_open`.
- **Windows** (`WindowsSpawner::wait_any`): `WaitForMultipleObjects`, with
  its 64-handle limit absorbed internally (batched waits), same fallback
  rule.
- **`platform-mock`**: no native multiplexer to reach — uses the portable
  tick-loop unconditionally, which is fine for scripted single-process
  tests.

**Cancellation safety:** `wait_any` takes ownership of nothing — the
`&mut [Box<dyn Child>]` slice is borrowed, not consumed, so a caller that
drops the future/stops polling before `wait_any` returns leaves every
child exactly as it was (no partial reap, no lost wakeup) and can safely
call `wait_any` again later.

**Completion ownership:** the winning child's exit status is reaped and
stashed into that `Child` (retrieved via its own `try_wait`/`wait`) before
`wait_any` returns — the caller, not the reactor, owns what happens next
for every child, including the ones that didn't win this round.

**Executor assumptions:** none. `wait_any`/`Spawner::wait_any` are plain
blocking-with-timeout functions, callable from any thread; `platform`
itself has no async runtime dependency (`#![forbid(unsafe_code)]`, and
per `tun.rs`/`term.rs`'s explicit layering rule, Layer 2 must not depend
on a Layer 3 async runtime). The async *path* this satisfies is: an
external multiplexed wait, not a `Future`-returning API. A consumer that
wants `wait_any` driven from an async executor (e.g. `rusty_tokio`) calls
it from a blocking-task/reactor-thread the same way it would any other
OS-multiplexed wait — this mirrors how `rusty_tokio`'s own reactor already
consumes `platform-linux`/`platform-bsd`'s raw-fd escape hatches (see
`docs/behavior/net.md`'s Async path section) rather than `platform`
depending on `rusty_tokio`.

**Backpressure:** not applicable — `wait_any` returns at most one ready
index per call; a caller polling a large child set controls its own
concurrency by how many children it hands in.

## Deliberately unspecified (until the R2 hoist supplies them)

- PTY — a distinct Process×Terminal surface (D13), gated on an
  emulator/mux consumer.
- `ExitStatus::Signaled`/`Stopped`'s payload is still the raw OS signal
  number, not the portable `Signal` — `kill_tree`/`kill_single` grew a
  portable *sender*-side identity (rustils#46) without also converging
  the *received*-signal payload; that stays a future question, not
  something this slice needed to answer.
- Job-control terminal handoff (`tcsetpgrp` give/reclaim) is a
  `platform::term` extension trait (`JobControlTerminal`, Unix-only),
  not part of process at all — see `docs/behavior/term.md`.
