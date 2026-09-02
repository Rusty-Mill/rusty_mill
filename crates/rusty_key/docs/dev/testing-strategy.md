# Testing strategy

> **Authoritative source** for how Rusty Keys tests its **deterministic logic**: the four test tiers (unit / integration / property / snapshot), the `FakeLanguageModel` scripted-turn fixture that makes all LLM-dependent code deterministically testable, and golden-episode deterministic replay over the episode-package JSON. Other documents link here. This doc is distinct from [`eval-plan.md`](./eval-plan.md): **this measures deterministic logic; eval-plan measures model/maturity.** They share only the episode-package fixture *format* ([data-model §5](../architecture/data-model.md#5-episode-package--episodesturn_idjson-h3-prd-05)) — reference, do not duplicate.

Tool/crate choices and tier names are **v1 intent** — the approach to build against, revisit after the Phase 1 spike. The engineering substrate (this doc + [error-handling.md](./error-handling.md) + [coding-standards.md](./coding-standards.md)) lands **with Phase 1**, not after: every phase's "done" includes its tier of tests.

Related: [`error-handling.md`](./error-handling.md) (the `ToolOutcome` / `PolicyError` contracts these tests guard) · [`../architecture/data-model.md`](../architecture/data-model.md) §5/§7/§10 (episode package, serde, torn-line) · [`eval-plan.md`](./eval-plan.md) (maturity gates over the same fixture).

---

## 1. The four tiers

| Tier | Crate(s) | What | Determinism |
|---|---|---|---|
| **Unit** | all libraries | Pure logic, no async, no I/O | Total |
| **Integration** | `app` + below | A real `Session` driven by `FakeLanguageModel` (§2) | Total (scripted) |
| **Property** (`proptest`) | `feed`, `compose`, `observe` | Invariants over arbitrary inputs | Total |
| **Snapshot** (`insta`) | `compose`, `app` | Stable human/model-facing rendered surfaces | Total |

The hard constraint: **no test hits a live provider.** The harness's thesis is "verification you can trust" — a test suite that is itself nondeterministic (flaky, token-costing, network-bound) cannot establish that. The `FakeLanguageModel` (§2) is what makes the kernel loop, `CriteriaJudge`, consolidation, and compaction testable at all.

### 1a. Unit — pure logic

No async, no I/O. Concrete examples:

- **Policy boundary** — `WorkspacePolicy` canonicalization: `../` escape, symlink-out, absolute-path-in-workspace all resolve correctly to the right `PolicyError` variant ([error-handling §3](./error-handling.md#3-policyerror-struct--enum)).
- **Security checks** — each `SecurityCheck` (command-injection, etc.) flags its pattern and only its pattern.
- **Recall math** — relevance + recency + importance blend, decay, cross-domain (FTS5 vs cosine) batch normalization ([data-model §3](../architecture/data-model.md#3-long-term-store--storedb-sqlite--storeduckdb-duckdb), PRD 03).
- **M-HIR arithmetic** — numerator (non-`benign` interventions) over denominator (`count_turns()`), burden weighting, edge cases (zero turns) ([data-model §4.2](../architecture/data-model.md#42-interventionsjsonl--human-interventions-drives-m-hir-prd-04-adr-0019), PRD 04).
- **`ToolOutcome` mapping** — each internal error → the right `ToolOutcome` variant and `status()` ([error-handling §5](./error-handling.md#5-the-tooloutcome-contract--status-carried-not-guessed)).

### 1b. Integration — a real `Session` over `FakeLanguageModel`

The keystone. A real `Session` (full turn cycle, [ARCHITECTURE.md §6](../ARCHITECTURE.md#6-runtime-view--the-turn-cycle)) wired over the fake model (§2). Lets you assert end-to-end, deterministically:

- A policy block surfaces to the model as a `BLOCKED …` string and the verifier marks the turn `UNVERIFIED` ([ARCHITECTURE.md §10](../ARCHITECTURE.md#10-failure-modes--resilience)).
- A tool `Error`/`Timeout` `ToolOutcome` fails `NoToolErrors`.
- `tokio::join!` post-turn ordering: judge + consolidation + entropy all complete before their signals are observed (ADR-0012).
- Compaction triggers at the configured token thresholds; `judge_unavailable` bars `AutonomousVerifiedSuccess`.

### 1c. Property (`proptest`) — invariants

- **Policy** — no arbitrary input path escapes the workspace root (the security invariant, stated as a property).
- **JSONL round-trip** — for every persisted record type, `serde` encode → decode is identity, and a torn trailing line is skipped not fatal ([data-model §7](../architecture/data-model.md#7-serde-wire-conventions-adr-0025), [§10](../architecture/data-model.md#10-append-only-durability)).
- **`ToolOutcome` round-trip** — `to_model_string()` → parse yields the same `status()` for arbitrary payloads. This directly guards the [ADR-0022](../adr/0022-structured-tooloutcome-tool-result-contract.md) contract against regression ([error-handling §5](./error-handling.md#5-the-tooloutcome-contract--status-carried-not-guessed)).

### 1d. Snapshot (`insta`) — rendered surfaces

These strings are the human/model-facing prompt surface; snapshot them so a refactor cannot silently change what the model or user sees:

- `VerificationReport::render()` ([PRD 05](../prd/05-compose.md)) — the `/verify` output, including the always-present `limits` line.
- The recall block, the startup banner, `/mhir` output (PRD 06).
- A canonical episode-package JSON ([data-model §5](../architecture/data-model.md#5-episode-package--episodesturn_idjson-h3-prd-05)) — pins the serde shape and the 8 traces.

---

## 2. `FakeLanguageModel` — the keystone fixture

A `LanguageModel` implementation (the trait the kernel calls) that returns **scripted turns** instead of calling a provider. This is the single seam that makes every LLM-touching path — kernel loop, `CriteriaJudge`, consolidation, compaction summaries — deterministic.

```rust
// kernel test-support: a scripted, deterministic LanguageModel
pub struct ScriptedTurn {
    pub tool_calls: Vec<(String, serde_json::Value)>, // emit these, in order
    pub final_text: Option<String>,                   // then this final reply
}

pub struct FakeLanguageModel {
    turns: std::sync::Mutex<std::collections::VecDeque<ScriptedTurn>>,
}
// impl LanguageModel for FakeLanguageModel: pop the next ScriptedTurn per step,
// emit its tool calls (dispatched/vetted for real), then its final_text.
```

- **Location.** Defined in `kernel` behind a `test-support` feature (a small exported test crate / `pub` under that feature), so **every crate above** can depend on it to drive a deterministic episode — not buried in one crate's `#[cfg(test)]`.
- **What is real vs faked.** Only the *model's choices* are scripted. Tool dispatch, policy vetting, the `Tracer`, verification, consolidation, and storage all run for real against the scripted turns — so the integration tests in §1b exercise the genuine harness, with the only nondeterministic input (the model) pinned.
- **Per-role reuse.** The same fixture backs the judge model, the consolidate model, and the compact model (the per-role knobs in [configuration.md](../reference/configuration.md#model-selection-per-role-)), so `CriteriaJudge` parse-failure / `judge_unavailable` behavior is testable by scripting a malformed judge reply.

This is the *engineering* substrate beneath [eval-plan.md](./eval-plan.md): the fake model makes the code unit-testable; the eval plan separately scores a *real* model.

---

## 3. Golden-episode deterministic replay

A fixtures directory of frozen episode-package JSON ([data-model §5](../architecture/data-model.md#5-episode-package--episodesturn_idjson-h3-prd-05)). The replay harness feeds the recorded `tool_trace` / interventions / entropy through the **verifier and outcome classifier** and asserts the same `EpisodeOutcome` and `VerificationReport`. **Record real episodes once, replay forever** — that is how "verification you can trust" becomes itself tested.

- Tests the **deterministic compose/verify logic** (does the same evidence produce the same verdict), **not** model quality.
- Guards the [ADR-0021](../adr/0021-fixed-failuretype-taxonomy.md) `FailureType` mapping, the `(category, layer)` attribution matrix ([PRD 05](../prd/05-compose.md)), and `JSONL` round-trip ([data-model §10](../architecture/data-model.md#10-append-only-durability)) against drift.
- **Shared-format, separate-purpose.** [eval-plan.md](./eval-plan.md) replays the *same* JSON fixtures to measure maturity/regression against a live or graded model. The two docs reference the format; neither restates the schema (the schema lives in [data-model §5](../architecture/data-model.md#5-episode-package--episodesturn_idjson-h3-prd-05)).

---

## 4. Tier-by-example summary

| Concern | Tier | Asserts |
|---|---|---|
| Policy boundary (`../`, symlink) | unit | correct `PolicyError` variant |
| Security checks | unit | pattern flagged, structurally |
| Recall math (blend/decay/normalize) | unit | scores match expected |
| M-HIR arithmetic | unit | numerator/denominator/burden correct |
| `ToolOutcome` round-trip | property | `to_model_string()`→`status()` stable ([ADR-0022](../adr/0022-structured-tooloutcome-tool-result-contract.md)) |
| JSONL round-trip + torn line | property | encode/decode identity; skip-on-tear ([data-model §10](../architecture/data-model.md#10-append-only-durability)) |
| Path-escape invariant | property | no input escapes workspace |
| Block → `UNVERIFIED`; `join!` order | integration | full `Session` over `FakeLanguageModel` |
| `judge_unavailable` bars success | integration | scripted malformed judge reply |
| `VerificationReport::render()` | snapshot | rendered `/verify` surface incl. `limits` |
| Episode-package JSON / `/mhir` / banner | snapshot | serde + prompt surface stable |
| Verdict reproducibility | golden replay | same evidence → same `EpisodeOutcome` |

CI runs all four tiers via `cargo test --workspace`; the matrix and gates are in [coding-standards §CI/CD](./coding-standards.md).
