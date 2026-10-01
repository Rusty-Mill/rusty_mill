# Architecture

## Overview
A user submits one validated goal. The dispatcher decomposes it into task cards, routes each card by role to an agent adapter, and records everything on a per-goal blackboard. Agents exchange **references** (entry ids, paths, commits), never pasted content, so no information is paid for twice. See [ADR-0001](./docs/adr/0001-shared-substrate-over-agent-messaging.md).

Non-goals: agent-to-agent chat; an LLM deciding routing; using any vendor credential outside that vendor's own CLI.

## Boundaries
Ports-and-adapters. `orch-core` holds all invariants and does no I/O; everything that touches a process, clock, network, or disk is an adapter.

| Port | Adapter(s) | Notes |
| ---- | ---------- | ----- |
| Agent runner | `claude -p`, `codex exec`, `gemini -p`, Ollama HTTP | Planned. One adapter per CLI; prompt in, entry refs out. Subscriptions only, no API keys. |
| Board store | remind-me MCP (`board:<project>`) or SQLite | Planned. Persists `Board`/`Plan`; the domain assigns ids. |
| Goal intake | CLI / JSON → `GoalDraft` | Planned. Parsing and serde live here, not in the core. |
| Clock / budget meter | dispatcher | Planned. Wall-clock and call counting are I/O. |

## Structure
Cargo workspace, modular monolith. `orch-core` is one crate with three aggregates that share `Text`, `Ref`, and the id types:

- `goal` — `GoalDraft` → `Goal` via `TryFrom`; rejects drafts missing DONE WHEN, out-of-scope, or budget, reporting every problem at once.
- `task` — `TaskSpec`, `Task`, and `Plan`, the only mutation point for task state. Dependencies must pre-exist (acyclic by construction); review targets are implicit prerequisites; no agent reviews its own output; completion must return ≥1 board entry.
- `board` — append-only `Board`. Changes are same-kind supersessions (linear, no forks); entry refs, answers, and artifacts are validated on append; `live()` and `open_questions()` are what agents read.

## Data flow
1. Goal intake parses input into `GoalDraft`; `Goal::try_from` validates it.
2. Dispatcher adds cards to a `Plan` and loops on `Plan::ready()`.
3. For each ready card: route role → agent, `Plan::start`, run the adapter with the card and its refs.
4. Agent writes entries to the `Board`; dispatcher calls `Plan::complete` with their ids, or `block` on a `Question`.
5. Review cards run on a different agent than the author; `Checkpoint` stop rules surface `open_questions()` to the user.

## Key decisions
See [docs/adr/](./docs/adr/).
