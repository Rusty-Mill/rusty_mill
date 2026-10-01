# ADR-0003: Dispatcher crate reuses nothing from other `apps/` families

- **Status:** Proposed (draft, awaiting sign-off)
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
- `orch-dispatch` (at `crates/apps/rusty_orch/crates/orch-dispatch`) depends only on `orch-core` in P1. It defines its own `Agent` port trait, a `Role → orch_core::Agent` routing table, and a call counter checked against `Budget::max_calls`.
- `orch-core` stays zero-dependency and I/O-free; nothing from this ADR touches it.
- Cross-family reuse of `sessionmgr-git`'s worktree logic or `rp-providers` requires first moving the reused crate to `libs/` (root ADR-0003 already anticipates this for `rp-core`/`rp-providers`). That is a separate decision taken when a real adapter needs it, not now.
- Process spawning for CLI adapters will go through `contract::ProcessRunner`, which is an allowed `platform` edge and already has a native implementation and fakes.

## Consequences
- P1 scope is unchanged: no monorepo crate shortens the dispatcher work.
- The dispatcher is synchronous and dependency-free, matching `orch-core`'s test story.
- Two future ADRs are implied: adapter transport via `contract` (allowed today) and a `libs/` move for any `apps` crate rusty_orch wants to share.
