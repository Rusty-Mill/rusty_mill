# Windows AI Coding-Agent Session Manager — Scope

## Origin & clean-room boundary

Idea prompted by inspecting `Xirp-0.14.0-x64-external.dmg` (Spotify's closed-beta,
macOS-only Electron app for managing Claude Code / Codex / Gemini CLI sessions).
That inspection stopped at container/framework level (confirmed: Electron +
`app.asar`, electron-builder packaging) — **no Xirp source, assets, strings, or
UI was extracted, read, or used as reference.** This project borrows only the
high-level *concept* (isolate each agent session in its own git worktree; let a
manager UI drive multiple CLI agents in parallel; optionally feed in org context
via MCP) — concepts with substantial prior art outside Xirp — and implements it
independently. No Spotify code, branding, or copy in this repo, ever.

## Problem

Running multiple Claude Code / Codex / Gemini CLI sessions against the same
repo in parallel today means manually juggling worktrees, terminals, and branch
state. There's no Windows-native tool that isolates sessions in worktrees; see
Competitive landscape below for why this is a narrower (but still real) gap
than "no Windows tool exists at all."

## Competitive landscape (verified 2026-08-16, not from a video)

Directly fetched and confirmed, not secondhand from a review:

| Tool | Worktree isolation | Windows | License/cost |
|---|---|---|---|
| **Xirp** (Spotify) | Yes, per-session | No (macOS only) | Closed, Spotify-account-gated |
| **Conductor** (Melty Labs, YC S24) | Yes, per-session | No (macOS Apple Silicon only; Intel "in development") | Free, closed source |
| **Solo** (Aaron Francis, soloterm.com) | **No** — explicitly *not* a worktree orchestrator; coordinates parallel agents in one shared workspace via lease-based locks, shared key-value state, scratchpads/todos, 40+ MCP tools | **Yes** (macOS + Windows today; Linux "coming soon"). Built with Tauri. | Proprietary: free tier (4 projects/20 processes), Pro $99/yr |

**Conclusion: the actual gap is the intersection** — nothing on the market
combines git-worktree isolation with Windows support. Xirp and Conductor have
the isolation model but not Windows; Solo has Windows (and is a genuinely
strong, actively-developed native app — worth using directly instead of
building this, if worktree isolation specifically isn't a hard requirement)
but deliberately doesn't do worktree isolation.

This sharpens (and shrinks) the differentiator from "a Windows session
manager" to specifically "a Windows session manager with per-session git
worktree isolation." If that specific combination isn't actually needed —
i.e. Solo's shared-workspace + locks model is good enough — the honest move
is to stop here and use Solo rather than build a competitor to it. Worth
answering before writing more of this project (see Open questions).

## Goal

A Windows-native (Rust) session manager that:
- Spawns and supervises multiple AI coding-agent CLI sessions concurrently.
- Isolates each session in its own git worktree + branch, created/torn down
  automatically.
- Presents session state (running/idle/needs-input/errored, diff summary,
  branch name) in one place.
- Lets the user attach to a session's terminal, send input, and merge/discard
  its worktree when done.

## Non-goals (v1)

- Not a general remote-control/mobile-companion app.
- Not an org-context/knowledge-graph platform (Portal/Backstage equivalent) —
  MCP server config is user-supplied, not built.
- Not cross-platform polish for macOS/Linux — Windows-first; if the core is
  kept in a platform-agnostic crate, other OSes are a later port, not a v1
  requirement.
- Not a fork, clone, or "compatible" reimplementation of Xirp's UI/UX, file
  formats, or branding.

## Prior art to check before writing anything new

Per this repo owner's own sovereignty-loop practice: check the following
before hand-rolling —
- `rustils` — already has Windows Job Object sandboxing (suspended-spawn →
  `CreateJobObjectW` → `AssignProcessToJobObject` → resume) built for
  `rusty_prime_agent`'s Python worker. Same primitive applies to sandboxing
  each agent CLI child process here.
- `rusty_tokio` — async runtime already used for process spawning
  (`process::Command` built on raw `std::process`, not `rustils::Command`,
  because the reactor needs raw handles). Reuse rather than a second process
  layer.
- `rusty_prime_agent` — closest existing sibling; confirm how much of its
  session/process supervision is reusable vs. specific to its sandboxed Python
  worker use case.

**Open question:** how much of the above is directly reusable vs. needs a new
crate. Resolve this in a design pass before implementation starts.

## Architecture (ports-and-adapters, modular monolith)

```
core/              # domain logic, no I/O
  session.rs       # Session state machine: Created -> Running -> (NeedsInput | Errored) -> Merged | Discarded
  worktree.rs       # worktree lifecycle rules (naming, cleanup policy)
adapters/
  git/             # shell out to `git worktree add/remove`, or gitoxide if it covers worktrees adequately
  process/         # agent CLI spawn/supervise, built on rusty_tokio + rustils Job Object sandboxing
  agent_cli/       # per-agent adapter trait: launch args, stdin/stdout framing, "needs input" detection
    claude_code.rs
    codex.rs
    gemini_cli.rs
  mcp/             # optional: read user-supplied MCP server config, expose to agent adapters
ui/                # thin layer over core — TUI first (ratatui), GUI deferred
```

Domain (`core/`) has no knowledge of Windows, git, or any specific agent CLI —
only adapters do. This is what makes "port to another OS later" cheap without
being built for it prematurely.

## Tech stack (proposed)

| Concern | Choice | Why |
|---|---|---|
| Language | Rust | User preference; Windows syscalls via `windows-rs` where needed |
| Async | `rusty_tokio` | Already exists, already handles Windows process I/O |
| Process sandboxing | `rustils` Job Object helper | Already built and proven for this exact primitive |
| Git operations | Shell out to system `git worktree` | Worktree support in Rust git libs (gitoxide) is less mature than CLI; shelling out is simple, correct, and matches what Xirp itself does |
| UI (v1) | `ratatui` (TUI) | Minimal deps, no Electron/Chromium tax (the whole point is to *not* ship a 191 MB Electron bundle), fast to build |
| UI (stretch) | Tauri | Only if a GUI is later justified — still far lighter than Electron, and genuinely cross-platform if that becomes a goal |
| Config | TOML via `serde` | Explicit config, no magic globals, matches house style |

## MVP slice (v1)

1. `sessionmgr new <agent> [--branch <name>]` — create worktree, launch agent
   CLI attached to current terminal.
2. `sessionmgr list` — show all active sessions, state, branch, worktree path.
3. `sessionmgr attach <id>` — reattach to a running session's I/O.
4. `sessionmgr close <id> [--merge|--discard]` — tear down worktree, optionally
   merge branch back.
5. Job-Object-based process sandboxing on every spawned agent child (reuse
   `rustils`).

TUI dashboard (multi-session view) is v1.1, not blocking the MVP — a CLI that
does the above solo is already useful and de-risks the core session/worktree
logic before investing in UI.

## Explicitly deferred

- MCP/org-context integration (config plumbing only, no server of our own).
- Remote control / mobile companion.
- Auto-update mechanism.
- macOS/Linux builds.

## Open questions for sign-off

0. **Build vs. use Solo.** Solo already ships on Windows today, is actively
   developed, and covers most of the "manage multiple agent CLIs from one
   place" problem via shared-workspace coordination (locks/scratchpads/
   todos/MCP) instead of worktrees. Before writing any code: is per-session
   git-worktree isolation specifically required, or would Solo's model
   (paying $99/yr if past the free tier) actually solve the real problem?
   If worktree isolation isn't a hard requirement, this project shouldn't
   exist — adopt Solo instead. This is the highest-leverage question in this
   doc; everything below assumes the answer is "yes, worktree isolation is
   required," which hasn't actually been confirmed yet.
1. New crate (`rusty_session_mgr`?) vs. a module inside an existing repo —
   depends on the prior-art check above.
2. Which agent CLIs to support at MVP: just Claude Code, or Claude Code +
   Codex + Gemini CLI from day one? (Each adds an `agent_cli/` adapter and a
   "needs input" detection heuristic — non-trivial per CLI.)
3. TUI vs. headless-CLI-only for v1 — dashboard adds real scope; confirm it's
   wanted before MVP rather than assumed.
