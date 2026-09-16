# sessionmgr

A Windows-native Rust session manager for AI coding-agent CLIs — Claude Code,
Codex, and Gemini CLI — where each session is optionally isolated in its own
git worktree, presented through a TUI grid dashboard, and **survives the
manager application closing**.

## Clean-room boundary

This is a clean-room build. It was prompted by observing Spotify's Xirp (a
closed-beta, macOS-only Electron app solving an overlapping problem), but **no
Xirp code, assets, strings, or UI were ever extracted, read, or used as
reference** at any point. The feature target in
[`docs/plan/CAPABILITIES.md`](docs/plan/CAPABILITIES.md) was assembled entirely
from five public, black-box sources (review videos, a vendor announcement,
third-party analysis), with each capability tagged by which source confirmed it
and how reliable that source is. No Spotify code, branding, or copy appears in
this repository.

## Why this exists

Nothing on the market combines **git-worktree isolation** with **Windows
support**. Xirp and Conductor have per-session worktree isolation but are macOS
only; Solo runs on Windows but deliberately does not do worktree isolation. See
[`docs/plan/SCOPE.md`](docs/plan/SCOPE.md) for the verified competitive
landscape and the resulting differentiator.

## Governing documents

| Document | Role |
|---|---|
| [`docs/plan/PLAN.md`](docs/plan/PLAN.md) | **The authority.** Architecture, process-supervision design, worktree lifecycle, adapter strategy, testing strategy, phased milestones, risk list. |
| [`docs/plan/CAPABILITIES.md`](docs/plan/CAPABILITIES.md) | The feature target every phase is scoped against, source-tagged by reliability. |
| [`docs/plan/SCOPE.md`](docs/plan/SCOPE.md) | Original scope and non-goals. **Superseded by PLAN.md wherever the two conflict** — notably its claims about Job-Object sandboxing, which PLAN.md corrects. |
| [`docs/decisions/`](docs/decisions/) | Architecture decision records, one per resolved gate. |

## Architecture in one paragraph

Ports-and-adapters: `sessionmgr-core` is pure domain logic with zero I/O, and
every OS interaction lives behind a port implemented by an adapter crate. One
binary (`sessionmgr.exe`) plays three roles — a long-running **daemon**
supervisor that outlives the UI, a detached **worker** process per session that
owns the agent CLI child, and **client** roles (`new`/`list`/`attach`/`close`/
`tui`). Job Objects are deliberately **not** used: kill-on-close is structurally
incompatible with sessions surviving the manager closing, a conflict
`rusty_prime_agent` independently discovered and documented. Detachment is
instead `CREATE_NEW_PROCESS_GROUP | DETACHED_PROCESS` on Windows and `setsid()`
on Unix, with liveness tracked by PID plus a start-time fingerprint so a reused
PID is never mistaken for a live worker.

## Status

**Phase 8 complete** — a real graphical desktop front end, on top of a
worktree-isolated session manager with fork, mid-session agent switching, and
dependent sessions all working against real agent CLIs.

- **Phase 0**: `rusty_tokio` confirmed as the async runtime —
  [ADR-0001](docs/decisions/0001-async-runtime-rusty-tokio.md), including its
  stated verification limits.
- **Phase 1**: daemon / detached worker / client roles, `PlainTerminal`
  sessions, and 79 passing tests. `supervisor_restart_recovery.rs` proves the
  central promise against real processes: a session survives its daemon being
  `SIGKILL`ed and is **adopted** — not respawned — by the replacement. Both
  spikes ran; see the [Phase 1 report](docs/phase-1-report.md).
  - Claude Code's hooks **do** fire when it runs headless under a detached
    worker, so Phase 3's hook-based design stands.
  - A real PTY turns out to be **mandatory**, not preferable, for interactive
    agent sessions — [ADR-0002](docs/decisions/0002-pty-required-for-agent-sessions.md),
    now implemented, with ConPTY confirmed to survive an unclean daemon kill.
- **Phase 2**: git worktree isolation, all three session kinds, and
  merge/discard teardown.
- **Verified on real Windows**: the full suite passes **105/105** on
  `x86_64-pc-windows-msvc`, not just cross-compiled. That pass found two
  genuine product bugs the Linux suite had not — an unbounded socket wait
  behind a bind-before-recover race, and `terminate()` wrongly failing on an
  already-exited pid. Both fixed; see the
  [Phase 2 report](docs/phase-2-report.md). The Defender smoke test and the
  `longPathAware` manifest wiring flagged as outstanding at the time are both
  since done — see [Phase 2 Windows verification](docs/phase-2-windows-verification.md).
- **Phase 3/3b/4/4b**: the `sessionmgr-tui` grid dashboard (panes, git diff
  view, command palette), Codex and Gemini CLI adapters, and session
  hooks/outbound webhook extensibility.
- **Phase 5**: dependent sessions (`new --parent`), sharing a running
  session's worktree.
- **Phase 6/7**: fork (clone a session's own agent-CLI conversation into a
  new one) and switch-agent (hand a live conversation off to a different CLI
  mid-session), both working across Claude Code, Codex, and Gemini CLI.
- **Phase 8**: [`sessionmgr-desktop`](crates/sessionmgr-desktop) — a
  Tauri-based graphical client with full interactive parity with the TUI
  (real xterm.js panes, command palette, git diff view, fork/switch-agent),
  since redesigned to a light theme with a project-grouped session sidebar.
  See [Desktop UI](#desktop-ui) below to build and run it.

Phase order was deliberate and gated: the highest-uncertainty work (agent-CLI
"needs input" detection, runtime availability) was proven earliest, and no
phase whose plan section depended on an earlier spike began before that
spike had an actual answer.

## Building

```
cargo build --workspace
cargo test --workspace
```

Gated agent-CLI adapter tests need the real CLIs installed to exercise
anything — each one auto-skips (prints a message, doesn't fail) when its CLI
isn't on `PATH`, so `cargo test --workspace` is always safe to run regardless
of which CLIs are present.

On Linux, building the workspace also builds `sessionmgr-desktop`
(`crates/sessionmgr-desktop`), which links against GTK/WebKitGTK. Install the
dev headers first — see `.github/workflows/ci.yml`'s "Install Tauri Linux
build dependencies" step for the exact package list. Windows needs nothing
extra (WebView2 ships with Windows 10/11).

## Desktop UI

`sessionmgr-desktop` is a second binary — a Tauri-based graphical client, an
alternative to the terminal `sessionmgr tui` with the same session grid,
command palette, and git diff view. It talks to the same daemon over the
same protocol as every other client role, so nothing else needs to be
running differently to use it.

```
cargo run -p sessionmgr-desktop
```

It starts a daemon automatically if none is running, same as `sessionmgr
tui` does. See [Phase 8 report](docs/phase-8-report.md) for the
architecture and design rationale.
