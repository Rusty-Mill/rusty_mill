# Error handling — taxonomy & the tool-result contract

> **Authoritative source** for the error model: the per-crate `thiserror` enums and their `#[from]` composition, the `thiserror`↔`anyhow` boundary, the no-panic rule and its lint backing, and the `ToolOutcome` tool-result contract (one type, one formatter, one parser). Other documents link here rather than restating the model. Wire encodings of any type below (serde, snake_case) are owned by [`../architecture/data-model.md`](../architecture/data-model.md); the lint *configuration* (clippy levels, MSRV) is owned by [`coding-standards.md`](./coding-standards.md). Decisions: [ADR-0021](../adr/0021-fixed-failuretype-taxonomy.md), [ADR-0022](../adr/0022-structured-tooloutcome-tool-result-contract.md), [ADR-0023](../adr/0023-error-model-thiserror-per-crate-anyhow-in-app-no-panic.md), [ADR-0025](../adr/0025-serde-wire-convention-snake-case.md).

Concrete variant sets and field names below are **v1 intent** — the design to build against and revisit after the Phase 1 spike, not a frozen ABI.

Related: [`../ARCHITECTURE.md`](../ARCHITECTURE.md) §10 (failure modes) · [`../architecture/data-model.md`](../architecture/data-model.md) §7 (serde) · [`testing-strategy.md`](./testing-strategy.md) (round-trip + property tests that guard these contracts).

---

## 1. The rule, in one line

**One `thiserror` enum per library crate; cross-crate composition via `#[from]`; `anyhow` only in `app`; nothing panics on a recoverable condition.** ([ADR-0023](../adr/0023-error-model-thiserror-per-crate-anyhow-in-app-no-panic.md))

Every library crate (`config`, `observe`, `constrain`, `feed`, `kernel`, `mcp`, `compose`) returns a concrete, matchable error enum on its public API. The application crate (`app`) — and only `app` — may collapse those into `anyhow::Error` for ergonomic top-level handling and context-chaining. This makes ADR-0001's compile-time-correctness promise real: callers `match` on variants instead of parsing prose.

---

## 2. Per-crate error enums

One enum per crate, named `<Crate>Error`. The DAG ([ARCHITECTURE.md §5](../ARCHITECTURE.md#5-crate-dependency-dag)) bounds which errors a crate may wrap: a crate composes only the errors of crates *below* it.

| Crate | Error type | Representative variants (v1 intent) | Wraps (via `#[from]`) |
|---|---|---|---|
| `config` | `ConfigError` | `MissingVar`, `ParseInt{key}`, `BadEnum{key,value}`, `Io` | — (leaf) |
| `observe` | `ObserveError` | `Io`, `Serde`, `TornRecord{line}` | `ConfigError` |
| `constrain` | `PolicyError` | see §3 | `ConfigError`, `ObserveError` |
| `feed` | `ToolError` | `NotFound{tool}`, `BadArgs{tool,detail}`, `Exec{tool}`, `Timeout{tool}`, `Storage`, `Policy(PolicyError)` | `ConfigError`, `ObserveError`, `PolicyError` |
| `kernel` | `KernelError` | `Provider{retryable}`, `Timeout`, `MaxStepsExhausted`, `Dispatch(ToolError)`, `Policy(PolicyError)` | `ConfigError`, `ObserveError`, `PolicyError` |
| `mcp` | `McpError` | `Connect`, `Transport`, `Protocol{code}`, `CallFailed{tool}`, `Policy(PolicyError)` | `ConfigError`, `PolicyError`, `ToolError` |
| `compose` | `ComposeError` | `Io`, `Serde`, `JudgeUnavailable`, `Check{name}` | `ConfigError`, `ObserveError`, `ToolError` |

`kernel` does **not** wrap `feed`/`compose` types directly (it never imports them); it receives an abstract `&dyn ToolDispatch` whose dispatch surface returns `ToolError`, which `KernelError` wraps via `#[from]`.

### `#[from]` composition

Composition is mechanical and downhill only:

```rust
// feed/src/error.rs
#[derive(thiserror::Error, Debug)]
pub enum ToolError {
    #[error("unknown tool {tool}")]
    NotFound { tool: String },
    #[error("tool {tool} timed out")]
    Timeout { tool: String },
    #[error(transparent)]
    Policy(#[from] constrain::PolicyError), // downhill: feed imports constrain
    #[error(transparent)]
    Storage(#[from] observe::ObserveError),
}
```

`type Result<T> = std::result::Result<T, <Crate>Error>` is defined per library crate; bare `Result<T>` in a library crate means *that crate's* alias, never `anyhow`. (This resolves the mixed `Result<()>` / `Result<T, SpecificError>` usage in the PRDs.)

### The `anyhow` boundary

`anyhow` is permitted **only** in `app` (and in genuinely best-effort post-turn glue inside `app`, e.g. the `tokio::join!` block where a failed consolidation must not fail the turn). Library crates never name `anyhow` on a public signature. At the boundary, `app` lifts a typed error with `.context(...)`:

```rust
// app — the only place anyhow appears
let report = verifier
    .verify_with_judge(&reply, &episode, &judge)
    .await
    .context("verification failed")?; // ComposeError -> anyhow::Error
```

---

## 3. `PolicyError`: struct → enum

The prior spec defined `PolicyError { reason: String }` — a free-text struct. A prose `reason` forces the compose layer to *parse English* to tell a path-traversal block from a mode gate, and forces `security.jsonl` to record an unstructured `checker`. We convert it to an enum so attribution ([PRD 05 `(category, layer)` matrix](../prd/05-compose.md)) and the `SecurityEvent` record ([data-model §4.3](../architecture/data-model.md#43-securityjsonl--blocked-calls-prd-02)) derive **structurally**.

```rust
// constrain/src/error.rs
#[derive(thiserror::Error, Debug)]
pub enum PolicyError {
    #[error("path {0} escapes workspace root")]
    OutsideWorkspace(PathBuf),
    #[error("mode {mode} forbids tool {tool}")]
    ModeForbidden { mode: &'static str, tool: String },
    #[error("blocked by {checker}: {pattern}")]
    SecurityCheck { checker: &'static str, pattern: String },
    #[error("web access disabled; set RUSTYKEYS_ALLOW_WEB=1")]
    WebDisabled,
    #[error("blocked by approval gate")]
    ApprovalDenied,
}
```

- The `security.jsonl` `checker` field ([data-model §4.3](../architecture/data-model.md#43-securityjsonl--blocked-calls-prd-02)) is the **variant name** (`SecurityCheck`, `OutsideWorkspace`, …), not free prose ([ADR-0023](../adr/0023-error-model-thiserror-per-crate-anyhow-in-app-no-panic.md)).
- Each variant maps to a fixed attribution category, so the compose layer never string-matches a `reason`.
- A new block reason is an exhaustively-checked enum addition, not a new ad-hoc string.

---

## 4. The no-panic rule, enforced

ADR-0007 states the harness *returns* errors and never panics on a recoverable condition; [ADR-0023](../adr/0023-error-model-thiserror-per-crate-anyhow-in-app-no-panic.md) makes it enforceable instead of aspirational. The workspace denies the panic-shaped lints in **library crates**, allow-listed in tests:

```rust
// workspace lint policy (mirrored in coding-standards.md)
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
)]
```

- `unwrap()` / `expect()` / `panic!()` / `a[i]` indexing in library code becomes a **CI failure** (`clippy -D warnings`, see [coding-standards §CI](./coding-standards.md)).
- Tests, benches, and example fixtures allow these via `#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing))]` (or per-module `#[allow]`).
- Recoverable conditions return an `Err`; truly-impossible states use an explicit, commented `unreachable!`/`expect` only where the invariant is locally provable — and even then prefer returning `<Crate>Error::Internal`.
- A panic that *does* escape a turn (a bug) is caught at the `Session::send` boundary, recorded as an aborted episode ([ARCHITECTURE.md §10](../ARCHITECTURE.md#10-failure-modes--resilience)), and surfaced as a typed `KernelError`, so one poisoned turn never tears down a long-lived gateway/MCP session.

The exact clippy level configuration (and the `[lints]` table location) is pinned in [coding-standards.md](./coding-standards.md).

---

## 5. The `ToolOutcome` contract — status carried, not guessed

**Problem.** Today `ToolStatus` is *reverse-engineered* from the result string by magic-prefix match: `BLOCKED …` → `Blocked`, `ERROR …`/`TIMEOUT …` → `Error`, else → `Ok` ([PRD 04](../prd/04-observe.md), [PRD 05 `NoToolErrors`](../prd/05-compose.md)). This is fragile in both directions: a tool whose *legitimate* output begins with `ERROR` is mis-flagged as failed, and a real failure whose text lacks the magic prefix is counted `Ok`. Verification correctness (`NoToolErrors`), entropy, and attribution all sit downstream of this guess.

**Decision ([ADR-0022](../adr/0022-structured-tooloutcome-tool-result-contract.md)).** One type carries the status **structurally**, with exactly **one formatter** (to the model-facing string) and **one parser** (status). Nothing else stringifies or sniffs a tool result.

```rust
// observe — the single tool-result contract
pub enum ToolOutcome {
    Ok(String),
    Error(String),
    Blocked(String),   // policy/security block (the PolicyError text)
    Timeout(String),
}

impl ToolOutcome {
    /// The ONLY producer of the model-facing string ("BLOCKED by policy: …",
    /// "ERROR: …", "TIMEOUT: …", or the raw Ok payload).
    pub fn to_model_string(&self) -> String { /* … */ }

    /// Total, structural — no prefix guessing.
    pub fn status(&self) -> ToolStatus { /* Ok|Error|Blocked → snake_case on wire */ }
}
```

**Wiring.**
- `ToolFn::call` returns `String` at the dispatch boundary (errors are *values*, not `Err`s, at the model-facing seam) — but that string is produced **only** by `ToolOutcome::to_model_string()`. The registry maps a `Result<String, ToolError>` from a fallible tool into the matching `ToolOutcome` (e.g. `PolicyError::*` → `Blocked`, `ToolError::Timeout` → `Timeout`) before calling `to_model_string()`.
- The `Tracer` stores the `ToolOutcome` (or its `status()`), so `ToolEvent.status` ([data-model §5 `tool_trace`](../architecture/data-model.md#5-episode-package--episodesturn_idjson-h3-prd-05)) is authoritative, never re-parsed from `result`. This retires the `ToolStatus`-is-inferred note in [PRD 04](../prd/04-observe.md) and the deferred `ToolResultClassifier` seam.
- `ToolStatus` serializes `ok` / `error` / `blocked` (snake_case, [ADR-0025](../adr/0025-serde-wire-convention-snake-case.md), [data-model §7](../architecture/data-model.md#7-serde-wire-conventions-adr-0025)).

**Round-trip invariant.** For any payload, `ToolOutcome::to_model_string()` parsed back yields the same `status()`. This is a property test ([testing-strategy §property tier](./testing-strategy.md)) — the direct guard that the contract cannot silently regress.

---

## 6. Error → model-facing surface, and the `FailureType` link

The model only ever sees the `to_model_string()` rendering of a `ToolOutcome`; it never sees a Rust error type. The mapping is one-directional and centralized:

| Internal | `ToolOutcome` | Model-facing string |
|---|---|---|
| `PolicyError::*` | `Blocked` | `BLOCKED by policy: {variant render}` |
| `ToolError::Timeout` | `Timeout` | `TIMEOUT: {tool}` |
| `ToolError::{Exec,BadArgs,NotFound}` | `Error` | `ERROR: {detail}` |
| `McpError::CallFailed` | `Error` | `ERROR: MCP call failed` |
| success | `Ok` | raw tool result |

Attribution then maps a *failed* `ToolOutcome`/check into the fixed 8-member `FailureType` enum (`f_context`, `f_tool`, `f_feedback`, `f_verify`, `f_recovery`, `f_entropy`, `f_model`, `f_unknown`) per [ADR-0021](../adr/0021-fixed-failuretype-taxonomy.md) and [data-model §5](../architecture/data-model.md#5-episode-package--episodesturn_idjson-h3-prd-05). Because both `PolicyError` (§3) and `ToolStatus` (§5) are now structured, this classification is a `match`, not a heuristic.

The `CriteriaJudge`'s call/parse failure is a typed `ComposeError::JudgeUnavailable`, recorded as `judge_unavailable` and **barred from `AutonomousVerifiedSuccess`** — it is never a passing `CheckResult` ([ARCHITECTURE.md §10](../ARCHITECTURE.md#10-failure-modes--resilience), [PRD 05](../prd/05-compose.md)).
