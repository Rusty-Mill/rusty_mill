# Windows hardening pass — issue #2, the Defender smoke test, and the longPathAware manifest

Run on a real Windows 11 desktop (not a CI runner), 2026-08-16. Closes out the
three items phase-1-report.md and phase-2-report.md explicitly left owed:
"manual verification still owed on Windows," the Defender smoke test, and
wiring the `longPathAware` manifest into the build.

## Status

| Item | Outcome |
|---|---|
| Issue #2 (daemon hang on `SessionList`) | **Root-caused and fixed** — plus a second, worse instance of the same bug found and fixed in the worktree path |
| Windows Defender smoke test (PLAN.md risk 7) | **Run** — 24 concurrent worktree sessions, real-time protection on, no failures, no hangs |
| `longPathAware` manifest wiring (PLAN.md risk 6) | **Done** — embedded, verified present in the built binary via `FindResource`/`RT_MANIFEST` |
| Deep-repo-path test | **Run — and it surfaced a real, unfixed-by-us limitation.** See below; this is not a clean "resolved." |

## Issue #2 — the daemon hang, root-caused

Issue #2's own diagnosis named the suspect exactly: `Supervisor::session_list`
runs synchronous filesystem and `/proc` work directly on the async runtime.
Confirmed by reading the call path rather than guessing:

- `catalog::list_sessions` does synchronous `std::fs::read_dir` plus one
  `std::fs::read_to_string` per session.
- `catalog::reconcile` → `recovery_for` → `is_same_process` → `is_alive` /
  `start_fingerprint`, which on Linux reads `/proc/<pid>/stat` directly and
  on macOS/BSD **shells out to `ps` and waits for it to exit** — real
  subprocess spawn-and-wait, not a syscall.
- None of this went through `spawn_blocking`. `#[rusty_tokio::main]` builds
  the default multi-threaded flavor (a fixed worker-thread pool, not
  `current_thread`), and issue #2's own notes that CI ran on a **2-core**
  runner are the tell: with only 2 worker threads, a handful of concurrent
  `list`/`new`/`close` calls each doing blocking I/O inline is enough to
  starve the pool, which is indistinguishable from the daemon hanging to
  anything else trying to connect.

**Fixed**: `Supervisor::session_list` now runs its filesystem/pid-probe work
inside `rusty_tokio::spawn_blocking`.

**A second, more serious instance of the same bug, found while investigating
this one**: `Supervisor::prepare_workspace` (`git worktree add`) and
`Supervisor::dispose_workspace` (`git worktree remove` / `branch delete` /
`merge --ff-only`) also ran their `SystemGit` calls — synchronous subprocess
spawns doing real disk I/O — directly on the async runtime, inside
`session_new`/`session_close`. This is the *worse* case: a `/proc` read is
microseconds; `git worktree add` checking out a working copy with an
antivirus scanner in the path (PLAN.md risk 7) is not, and it is exactly the
concurrent-worktree scenario the Defender smoke test below exercises. Both
are now wrapped in `spawn_blocking` too.

All four spots share one shape: the fix is `spawn_blocking`, not a retry or a
longer timeout, per the issue's own explicit instruction not to paper over
this class of bug.

**Verification**: `cargo test --workspace` green (same 105+ tests as
phase-2-report.md, run on this machine), `cargo clippy --workspace
--all-targets -- -D warnings` clean, `cargo fmt --all --check` clean. This
was not reproduced under contention on this machine (a fast, high-core-count
desktop is the wrong environment to reproduce a 2-core-runner starvation
bug) — the fix is confirmed correct by code inspection and is architecturally
the same class as the already-fixed bind-before-recover race, not confirmed
by forcing a repro. The next CI occurrence (if any) is the real test.

## Windows Defender smoke test (PLAN.md risk 7)

**Defender state confirmed, not assumed**: `Get-MpComputerStatus` showed
`RealTimeProtectionEnabled: True`, `AntivirusEnabled: True`, default —
this machine has no exclusions configured for the test paths (exclusion
list itself requires admin to read; real-time protection being on is the
part that matters for this test).

**Workload**: 24 concurrent `sessionmgr new --kind worktree` sessions
against one scratch repo, each running a `--no-pty` PowerShell payload that
writes 150 × 4 KB random-content files and commits them — real,
Defender-scanned file churn, not synthetic sleeps.

**Result**: all 24 sessions `Finished` (0 `Errored`), 0 `git`
worktree/branch errors, 0 close failures. Session creation (24 concurrent
`new` calls, each spawning a worker + `git worktree add`) completed in 27.5s
wall time. `close --discard` on all 24 left `.sessionmgr-worktrees` empty and
`git branch --list "sessionmgr/*"` empty — full cleanup, no leftovers.
`Get-MpThreatDetection` showed no detections from the run (the one entry in
the log predates this session by months and is an unrelated Downloads-folder
installer).

No hang, no Defender-induced failure, no leftover state. This closes PLAN.md
risk 7 as tested, not as assumed — with the caveat that this is one run on
one machine's Defender configuration, the same caveat ADR-0002 already
states for the ConPTY-survival CI run.

## `longPathAware` manifest — wired and verified

`crates/sessionmgr-daemon/build.rs` (new) embeds
`sessionmgr.exe.manifest` via the `embed-manifest` crate, gated on
`CARGO_CFG_WINDOWS` (the *target* cfg, not `cfg!(windows)`, so the existing
`cargo check --target x86_64-pc-windows-msvc` cross-check from Linux CI
still runs the same code path). Chosen over `winres`/`embed_resource`
specifically because it does not shell out to `rc.exe` — it emits
`/MANIFEST` linker arguments directly, so the build does not depend on the
Windows SDK being discoverable on `PATH`, on this machine or on
`windows-latest`.

**Verified embedded**, not just "the build didn't fail": extracted the
`RT_MANIFEST` resource from the built `sessionmgr.exe` via
`LoadLibraryEx`/`FindResource`/`LoadResource` and confirmed the XML content
— `longPathAware=true`, `activeCodePage=UTF-8`, the `supportedOS` entry —
byte-for-byte the checked-in manifest.

## Deep-repo-path test — a real, unresolved-by-us limitation found

This is the one item that did **not** come back clean, and it would be
dishonest to file it as resolved.

**What the manifest actually covers.** `longPathAware` is a per-image Win32
manifest property. It affects `sessionmgr.exe`'s *own* filesystem calls. It
has **zero effect on `git.exe`**, which this tool invokes as a child
process — a manifest does not propagate to children, structurally, on
Windows. This was not previously stated explicitly anywhere in the plan and
is worth recording: the manifest was necessary but was never going to be
sufficient for `git worktree add`/`remove` on a deep path.

**First finding, and fixed**: with the OS-level `LongPathsEnabled` registry
policy already on (confirmed via `Get-ItemProperty`) and the manifest
embedded, `git init` still failed inside a ~250-character path with
`Filename too long` creating `.git/hooks/`. Git for Windows has its own
opt-in, `core.longpaths`, independent of both the manifest and the registry
policy. `sessionmgr-git`'s `git()` helper now passes `-c core.longpaths=true`
on every invocation, `#[cfg(windows)]`-gated, rather than depending on the
user's global git config. Measured fix: `git init`'s hook-directory creation,
which failed identically without it, now succeeds at the same depth.

**Second finding, and not fixed — because there is no configuration fix.**
`git worktree add` on a sufficiently deep path fails with a different error,
`fatal: '$GIT_DIR' too big`, and **`core.longpaths=true` does not prevent
it** — verified directly, both as a `-c` flag and as a persisted
`.git/config` entry, both failing identically. This is a documented, widely
reported Git for Windows limitation in its MSYS2 compatibility layer (a
fixed-size internal buffer computing the worktree's `$GIT_DIR`), triggered
specifically by `git worktree add`/the linked-worktree machinery, distinct
from the ordinary long-path support `core.longpaths` and the registry policy
cover. It is not something an app manifest, a git config flag, or anything
else this project controls can work around; a `\\?\`-prefixed extended-length
path was tried and made git fail differently (`not a git repository`) rather
than fixing it — Git for Windows does not handle that prefix for repository
discovery.

**Measured threshold**, on this machine's git (`2.54.0.windows.1`), by
binary search over repository path length:

| Full worktree path length | `git worktree add` |
|---|---|
| 156 chars | OK |
| 186 chars | OK |
| 216 chars | **fails** (`$GIT_DIR too big`) |
| 236+ chars | **fails** (same) |

The real ceiling sits somewhere in the ~190–210 character range for the
**full worktree path** (repo root + `.sessionmgr-worktrees/<12-char-id>` +
whatever the repo itself is nested under) — well short of the 260-character
`MAX_PATH` the manifest and registry policy target, and not moved by
anything short of a Git for Windows fix upstream.

**What this means for PLAN.md risk 6**: "mitigated but not eliminated" was
the right call, but the actual ceiling is lower than the plan assumed, and
`longPathAware` is not the thing that mitigates it — `core.longpaths` is,
and only partially. A repository already nested beyond roughly 150–170
characters will hit this regardless of anything sessionmgr does, and the
failure surfaces as git's own cryptic error, not a clear message from this
tool. Producing a clearer error (checking the computed worktree path length
before calling `git worktree add` and failing with an explanation instead of
propagating git's message) is real, buildable scope — deliberately not done
here, since the actual threshold is git-version-dependent and internal to
Git for Windows, and a hard-coded guess baked into this tool would be a
second thing to keep in sync with git's own behavior rather than a real fix.
Worth a follow-up issue, not a silent gap.

## Files changed

- `crates/sessionmgr-daemon/src/supervisor.rs` — `session_list`,
  `prepare_workspace`, `dispose_workspace` moved to `spawn_blocking`.
- `crates/sessionmgr-daemon/build.rs` (new) — embeds the manifest.
- `crates/sessionmgr-daemon/Cargo.toml` — `embed-manifest` build-dependency.
- `crates/sessionmgr-daemon/sessionmgr.exe.manifest` — comment updated to
  reflect that it is now wired in.
- `crates/sessionmgr-git/src/lib.rs` — `-c core.longpaths=true` on every git
  invocation, Windows-only.
