# Architecture Decision Records

An Architecture Decision Record (ADR) captures a single significant decision —
its context, the decision itself stated imperatively, and the consequences and
trade-offs that follow. Each ADR is one immutable file; decisions are never
edited away but superseded by a later ADR that links back. `Status` is one of
`Proposed` (awaiting ratification) or `Accepted`. This directory is the
single source of truth for decisions; other docs link here rather than
restating rationale. ADRs **0001-0015** were extracted verbatim-in-substance
from the "Architecture Decision Log" of `../prd/00-overview.md`, which now links
here instead of inlining them; ADRs **0016-0028** record the new decisions and
deliberate divergences locked by `../review/00-consolidated-plan.md`; ADRs
**0029-0034** record the Round 2 decisions locked by
`../review/round2-consolidated.md`; ADRs **0035-0036** record the Round 3
faithfulness-audit decisions locked by `../review/round3-consolidated.md`, which
also flips ADRs **0018/0019/0020** to Accepted and broadens **0028**.

| # | Title | Status | Tags |
|---|-------|--------|------|
| [0001](0001-rust-as-implementation-language.md) | Rust as implementation language | Accepted | language, runtime, correctness |
| [0002](0002-aisdk-as-llm-provider-abstraction.md) | aisdk as LLM provider abstraction | Accepted | llm, provider, integration |
| [0003](0003-tokio-as-async-runtime.md) | tokio as async runtime | Accepted | async, runtime, concurrency |
| [0004](0004-session-first-not-repl-first.md) | Session-first, not REPL-first | Accepted | architecture, session, transport |
| [0005](0005-harness-decomposed-into-four-verbs.md) | Harness decomposed into constrain / feed / observe / compose | Accepted | architecture, decomposition, modularity |
| [0006](0006-tool-proc-macro-for-tool-registration.md) | `#[tool]` proc macro for tool registration | Accepted | tools, macro, ergonomics |
| [0007](0007-policy-vets-tool-calls-before-dispatch.md) | Policy vets tool calls before dispatch; errors returned, not panicked | Accepted | constrain, policy, safety |
| [0008](0008-memory-is-observe-orient-half-of-ooda.md) | Memory is the Observe + Orient half of the OODA loop | Accepted | memory, ooda, feed |
| [0009](0009-tiered-consolidation-idle-sleep-explicit.md) | Tiered consolidation — idle / sleep / explicit | Accepted | memory, consolidation, async |
| [0010](0010-pluggable-storage-via-rust-traits.md) | Pluggable storage via Rust traits | Accepted | storage, traits, memory |
| [0011](0011-skills-exempt-from-pruning.md) | Skills exempt from pruning | Accepted | memory, skills, retention |
| [0012](0012-post-turn-compose-runs-concurrently.md) | Post-turn compose runs concurrently | Accepted | async, compose, concurrency |
| [0013](0013-verification-carries-its-limits.md) | Verification carries its limits | Accepted | compose, verification, honesty |
| [0014](0014-intervention-logger-and-mhir-in-observe-layer.md) | Intervention Logger + M-HIR in observe layer | Accepted | observe, metrics, mhir |
| [0015](0015-evidence-journal-append-only-jsonl.md) | Evidence journal is append-only JSONL | Accepted | compose, storage, audit |
| [0016](0016-before-tool-becomes-async-fn.md) | `before_tool` becomes `async fn` | Accepted | constrain, async, policy |
| [0017](0017-subagent-spawning-via-sessionfactory-trait.md) | Subagent spawning via a `SessionFactory` trait | Accepted | architecture, dag, subagents |
| [0018](0018-episode-equals-turn-with-episode-id-grouping.md) | Episode = turn, with `episode_id` grouping | Accepted | faithfulness, eval, observe |
| [0019](0019-intervention-model-maps-to-avoidability-harness-gap-burden.md) | Intervention model maps UI actions to avoidability / harness_gap / burden | Accepted | faithfulness, mhir, observe |
| [0020](0020-entropy-categories-six-reconciled-to-seven.md) | Entropy categories — RK's 6 reconciled to the paper's 7 | Accepted | faithfulness, entropy, observe |
| [0021](0021-fixed-failuretype-taxonomy.md) | Fixed `FailureType` taxonomy | Accepted | compose, attribution, faithfulness |
| [0022](0022-structured-tooloutcome-tool-result-contract.md) | Structured `ToolOutcome` tool-result contract | Accepted | tools, error-model, observe |
| [0023](0023-error-model-thiserror-per-crate-anyhow-in-app-no-panic.md) | Error model — `thiserror` per library crate, `anyhow` in `app`, no-panic rule | Accepted | error-model, standards, lints |
| [0024](0024-trait-objects-at-plugin-seams-async-trait-msrv.md) | Trait objects at all plugin seams + async-trait mechanism + MSRV pin | Accepted | standards, traits, async |
| [0025](0025-serde-wire-convention-snake-case.md) | Serde wire convention — `rename_all = "snake_case"` for on-disk/wire enums | Accepted | serde, data-model, standards |
| [0026](0026-secret-redaction-by-default.md) | Secret redaction by default before logging, journaling, or emitting | Accepted | security, redaction, observe |
| [0027](0027-on-disk-schema-versioning.md) | On-disk schema versioning | Accepted | data-model, versioning, storage |
| [0028](0028-h0-selectable-harness-level-or-eval-only.md) | H0-H3 ladder as a controlled-visibility ablation (and H0 reachability) | Proposed | faithfulness, eval, maturity |
| [0029](0029-adopt-rmcp-mcp-sdk.md) | Adopt `rmcp` as the foundation of the `mcp` crate | Accepted | mcp, dependencies, integration, licensing |
| [0030](0030-capability-isolation-toolexecutor.md) | Capability isolation via a `ToolExecutor` seam + `RUSTYKEYS_ISOLATION` | Accepted | constrain, isolation, security, roadmap |
| [0031](0031-validation-gated-skill-promotion.md) | Validation-gated skill promotion | Accepted | feed, memory, skills, self-improvement |
| [0032](0032-divergent-converge-subagent-strategy.md) | Divergent→converge subagent strategy | Accepted | feed, subagents, planning, cost |
| [0033](0033-eval-integrity-anti-gaming.md) | Eval-integrity / anti-gaming guard | Accepted | eval, faithfulness, integrity, security |
| [0034](0034-kernelevent-unified-observability-stream.md) | `KernelEvent` as the unified observability stream | Accepted | observe, observability, otel, standards |
| [0035](0035-controlled-visibility-ablation-eval-substrate.md) | Controlled-visibility ablation as the eval substrate | Accepted | faithfulness, eval, maturity, integrity |
| [0036](0036-episode-package-assembly-projector.md) | Episode-package assembly projector at `compose`-time | Accepted | compose, observe, data-model, faithfulness |
| [0037](0037-guides-loaded-into-cached-system-prefix.md) | Layered guides loaded at session start into the cached system prefix | Proposed | feed, guides, prompt, context, roadmap |
| [0038](0038-stratified-declarative-extension-model.md) | Stratified declarative extension model — skills, hooks-as-policy, plugins | Proposed | feed, constrain, config, extensibility, integration, roadmap |
| [0039](0039-defer-loop-ownership-behind-contextcontroller.md) | Defer loop ownership; revisit behind a `ContextController` seam | Proposed | kernel, constrain, loop, context, roadmap |
