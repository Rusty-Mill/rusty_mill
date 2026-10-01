# ADR-0003: Dispatcher crate reuses nothing from other `apps/` families

- **Status:** Accepted
- **Date:** 2026-10-01

## Context
The in-memory dispatcher (`orch-dispatch`) needs a step loop, call metering, role→agent routing, an agent port, and eventually process spawning, worktree isolation, and persistence. The monorepo already has crates in each area, so each was checked for reuse. The workspace enforces one invariant (root ADR-0003, `.github/scripts/check_workspace_layers.py`): a crate whose layer is `apps` may not depend on an `apps` crate in another family ("cross-family apps dependency"). `rusty_orch` is an `apps` family.

| Need | Candidate | Layer | Verdict | Evidence |
| --- | --- | --- | --- | --- |
| Loop, step control | `rk-kernel` (`rusty_key/crates/kernel`) | apps | ignore | Wraps `aisdk::LanguageModelRequest` with tokio + async-trait; forbidden edge and async/network. |
| Budgets, metering, routing | `rp-router` (`rusty_provider/crates/router`) | apps | ignore | HTTP gateway: `Router`, `ClientBudgetSetting`, Postgres/SQLite persistence, reqwest; forbidden edge, wrong abstraction (spend in dollars, not calls). |
| Agent adapters, Ollama | `rp-providers`, `rusty_llama` | apps / libs | ignore (P1) | `rp-providers` is a forbidden edge and HTTP-only; `rusty_llama` is an in-process inference engine, not a CLI adapter. Revisit when real adapters land. |
| Worktree isolation | `sessionmgr-git` (`rusty_yirp`) | apps | ignore (P1) | `SystemGit` implements `sessionmgr_core::ports::GitPort::worktree_add`; exactly right, but a forbidden edge today. |
| Subprocess running | `proc-runner` | platform | ignore; depend on `contract` later | `proc-runner` is a binary. The reusable piece is `contract::ProcessRunner` + `compat::NativeProcessRunner` (platform layer, allowed). Out of scope until a real CLI adapter exists. |
| Plan/Board persistence | `rusty_sqlite` | libs | depend later | Allowed edge; persistence is out of P1 scope. |

## Decision
- `orch-dispatch` (at `crates/apps/rusty_orch/crates/orch-dispatch`) depends only on `orch-core` in P1. It defines the `AgentRunner` port, a validated routing table, and a call meter.
- Routing is a function over a config struct, not a map keyed on `Role` (`Review` carries a `TaskId`). Work roles (`Research`, `Design`, `Implement`, `Triage`) map to one agent each. `Review` takes an ordered reviewer preference list and picks the first agent that is not the target's author. The config is validated at construction: the list must contain at least two distinct agents, otherwise some author could never be reviewed.
- Both ceilings are enforced before every call: `Budget::max_calls` for the goal and `TaskSpec::max_calls` for the card. Exceeding either returns `DispatchError::CeilingReached` naming the ceiling and the card; nothing is spent on refusal.
- Call accounting lives in a caller-owned `Ledger`, passed to `run` beside the plan and board. The dispatcher holds only routing and the runner, so it can be dropped and rebuilt mid-goal without forgetting what was spent. Persistence of the ledger belongs with persistence of the plan and board.
- Each step is check → start → count → run → record. An agent failure leaves the card `Running` and counted, so the next `run` retries it under the same ceilings. When a `Running` card is refused by its own task ceiling, its retries are exhausted: the dispatcher calls `Plan::fail` with the refusal as the reason and returns `CeilingReached`. Dependents of a failed card are never ready, and a run with nothing ready, nothing blocked, and the plan unfinished returns `Stuck`.
- Board writes are atomic per call: outputs are staged on a copy of the board, which replaces the original only if every append validates. A rejected output leaves the board untouched, the card `Running`, and the call counted.
- Ready cards run sequentially in `TaskId` order. This keeps every run reproducible from the plan alone; parallel fan-out over independent ready cards is the trigger for an async dispatcher.
- `orch-core` stays zero-dependency and I/O-free; nothing from this ADR touches it.
- Cross-family reuse of `sessionmgr-git`'s worktree logic or `rp-providers` requires first moving the reused crate to `libs/` (root ADR-0003 already anticipates this for `rp-core`/`rp-providers`). That is a separate decision taken when a real adapter needs it, not now.
- Process spawning for CLI adapters will go through `contract::ProcessRunner`, which is an allowed `platform` edge and already has a native implementation and fakes.

## Consequences
- P1 scope is unchanged: no monorepo crate shortens the dispatcher work.
- The dispatcher is synchronous and dependency-free, matching `orch-core`'s test story.
- Two future ADRs are implied: adapter transport via `contract` (allowed today) and a `libs/` move for any `apps` crate rusty_orch wants to share.
