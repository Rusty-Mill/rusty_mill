# Architecture

## Overview
A user submits one validated goal. The dispatcher decomposes it into task cards, routes each card by role to an agent adapter, and records everything on a per-goal blackboard. Agents exchange **references** (entry ids, paths, commits), never pasted content, so no information is paid for twice. See [ADR-0001](./docs/adr/0001-shared-substrate-over-agent-messaging.md).

Non-goals: agent-to-agent chat; an LLM deciding routing; using any vendor credential outside that vendor's own CLI.

## Boundaries
Ports-and-adapters. `orch-core` holds all invariants and does no I/O; everything that touches a process, clock, network, or disk is an adapter.

| Port | Adapter(s) | Notes |
| ---- | ---------- | ----- |
| Agent runner | `orch-dispatch::AgentRunner`; `orch-ollama::OllamaAgent` (`ollama run --format json`), `orch-codex::CodexAgent` (`codex exec --sandbox read-only`, [ADR-0006](./docs/adr/0006-codex-adapter.md)) and `FakeAgent` today; `claude -p`, `gemini -p` planned. Shared core in `orch-cli`. | One adapter per CLI; card in, `Output` entries out. The dispatcher stamps task and author and appends, so adapters never write the board directly. Subscriptions only, no API keys. |
| Board store | remind-me MCP (`board:<project>`) or SQLite | Planned. Persists `Board`/`Plan`; the domain assigns ids. |
| Goal intake | `rusty_orch run <goal.json>`: JSON → `GoalDraft` + task list + routing ([ADR-0009](./docs/adr/0009-command-line.md)) | Parsing lives in the binary with `rusty_json`, not in the core. Tasks are human-authored; `--interactive` feeds answers back as Human `Answer` entries; the wall clock is enforced between dispatcher runs. |
| Call meter | `orch-dispatch::Ledger` | Caller-owned; counts usable calls against `Budget::max_calls` and each card's `TaskSpec::max_calls`, checked before every call. An adapter-classified unavailable prerequisite (such as a missing CLI login) is not charged, so the same card can resume after it is restored. Wall-clock is still planned (needs a clock adapter). |
| Process | `orch-cli::CommandRunner` | Fixed argv, stdin bytes, deadline, env scrub. `StdCommand` is real; `orch_cli::fake::FakeCommand` for tests. Own seam because `contract::ProcessRunner` lacks stdin and timeout ([ADR-0004](./docs/adr/0004-ollama-output-protocol.md)). |

## Structure
Modular monolith inside the `rusty_mill` workspace. `orch-dispatch` is the application layer over `orch-core` and depends on nothing else ([ADR-0003](./docs/adr/0003-dispatcher-reuse-boundary.md)). It runs ready cards one at a time in `TaskId` order; parallel fan-out is the trigger for an async dispatcher later. `orch-core` is one crate with three aggregates that share `Text`, `Ref`, and the id types:

- `goal` — `GoalDraft` → `Goal` via `TryFrom`; rejects drafts missing DONE WHEN, out-of-scope, or budget, reporting every problem at once.
- `task` — `TaskSpec`, `Task`, and `Plan`, the only mutation point for task state. Dependencies must pre-exist (acyclic by construction); review targets are implicit prerequisites; no agent reviews its own output; completion must return ≥1 board entry.
- `board` — append-only `Board`. Changes are same-kind supersessions (linear, no forks); entry refs, answers, and artifacts are validated on append; `live()` and `open_questions()` are what agents read. Agents never settle decisions alone: an agent-authored `Decision`, new or superseding, must reference a live approving `Review`, else `DecisionNeedsApproval` ([ADR-0005](./docs/adr/0005-agents-never-settle-decisions-alone.md)).

## Data flow
1. Goal intake parses input into `GoalDraft`; `Goal::try_from` validates it.
2. Dispatcher adds cards to a `Plan` and loops on `Plan::ready()`.
3. For each ready card, lowest id first: route role → agent (reviews take the first configured reviewer that is not the target's author), verify the runner supports that agent/role pair, check both ceilings in the `Ledger`, `Plan::start`, count the call, run the adapter with the card and the board. An unsupported pair fails the card before start/count/invocation (ADR-0007).
4. Dispatcher appends the agent's entries to the `Board` atomically (all or none) and calls `Plan::complete` with their ids, or `block` on a `Question`.
5. The loop returns `Blocked` with the waiting cards; once a human appends an `Answer`, the next `run` resumes them. Hitting a ceiling, an agent failure, or zero outputs stops the loop with a typed error; the runner classifies each failure ([ADR-0008](./docs/adr/0008-permanent-agent-errors.md)): a transient one is metered and retried on the next run until the card's own ceiling fails it; an unavailable backend (a missing CLI login) is not metered and leaves the card `Running`, so the same card resumes once a human repairs the environment; a permanent one, like an unsupported agent/role pair ([ADR-0007](./docs/adr/0007-explicit-runner-capabilities.md)), fails the card at once.

## Key decisions
See [docs/adr/](./docs/adr/).
