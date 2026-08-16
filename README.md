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

**Phase 0 complete.** `rusty_tokio` is confirmed as the async runtime — see
[ADR-0001](docs/decisions/0001-async-runtime-rusty-tokio.md), including its
stated verification limits. Phase 1 (walking skeleton + two spikes) is next.

Phase order is deliberate and gated: the highest-uncertainty work (agent-CLI
"needs input" detection, runtime availability) is proven earliest, and no phase
whose plan section depends on an earlier spike begins before that spike has an
actual answer.

## Building

```
cargo build --workspace
cargo test --workspace
```

Gated agent-CLI adapter tests require the real CLIs installed and are opted into
per CLI (e.g. `SESSIONMGR_TEST_CLAUDE_CODE=1 cargo test`); they skip cleanly
rather than failing when a CLI is absent.
