*Point-in-time working document — the locked output of Wave 1 (consolidation) of the five-persona review. It dedupes the five findings files into one ordered work plan and locks the deliverable set. Superseded once the canonical docs land; cite those, not this file.*

# Consolidated refinement plan — Rusty Keys

## Verdict

The specification is **strong, coherent, and unusually complete for a pre-implementation repo** (~2,400 lines, 9 PRDs, 15 ADRs, a 15-phase roadmap). The five reviews converge on one conclusion: this effort is **~80% consolidation/extraction/pinning and ~20% net-new**. The real work is (1) giving cross-cutting concerns a home (architecture, data model, error model, testing, eval), (2) pinning algorithms that are *named but not specified* (recall scoring, system prompt, entropy heuristics, M-HIR), (3) reconciling **spec-internal contradictions**, and (4) closing **faithfulness drift** against the research paper. The dominant risk is **doc drift and over-speccing**, not missing content — so single-source-of-truth ownership and aggressive cross-linking are mandatory.

## Single-source-of-truth (SSOT) ownership — the anti-drift rule

Every fact lives in exactly one canonical doc; everything else links to it. New/refined docs must add an "Authoritative source" line and link rather than restate.

| Concern | Authoritative home |
|---|---|
| On-disk state: `.rustykeys/` tree, SQLite DDL, all JSONL/TOML/JSON schemas, serde encodings, versioning | `docs/architecture/data-model.md` |
| System structure: component map, crate DAG, concurrency, topologies, feature matrix, NFRs, faithfulness map | `docs/ARCHITECTURE.md` |
| Decisions | `docs/adr/` (one file per ADR) |
| Env vars | `docs/reference/configuration.md` |
| Error model, tool-result contract | `docs/dev/error-handling.md` |
| Test approach | `docs/dev/testing-strategy.md` |
| Maturity measurement | `docs/dev/eval-plan.md` |
| Standards (MSRV, lints, async-trait, features, CI, visibility) | `docs/dev/coding-standards.md` |
| Threats / trust boundaries | `docs/architecture/threat-model.md` |
| Per-component depth | the PRDs (`docs/prd/*`) — link to the SSOTs above |
| Phasing / roadmap | `BACKLOG.md` |

## Cross-cutting themes and the decisions that resolve them

### A. Spec-internal contradictions to reconcile (highest priority — these are bugs)
1. **Crate count 7 vs 8.** PRD 00 omits `mcp`; PRD 06/07 define it. → **8 crates + `frontend/`.** Fix PRD 00 component map; update README diagram. *(systems G2)*
2. **`before_tool` sync vs async.** PRD 02 trait + ADR-007 say sync; the ApprovalGate section says it must become `async`. → **Decision: `async fn before_tool`** (ApprovalGate is a concrete need; remote ACL is a stated seam). Record as ADR; propagate to `ToolRegistry::dispatch` and kernel. *(systems G11, software)*
3. **`kernel.run()` signature drift.** PRD 01 `(history, registry, extra_context, tracer)` vs PRD 06 `(history, registry, policy, context, tracer)`. → pick the PRD-06 form (kernel needs policy to pass to dispatch); fix PRD 01. *(systems N1b)*
4. **`TaskState.scope` missing.** Entropy `BoundaryViolation` reads it; struct lacks it. → **add `scope: Vec<String>`** to `TaskState`. *(systems N1a, harness, ai)*
5. **serde encodings inconsistent.** PascalCase / snake_case / lowercase mixed. → **`#[serde(rename_all="snake_case")]` for all wire enums** (`ToolStatus`, `EpisodeOutcome`, `InterventionKind`, `EntropyCategory`, `CompactionTier`, `FailureType`). Owned by data-model. *(systems N4, software, harness)*
6. **`rk://` event catalog disagreement.** BACKLOG=6, PRD 06=8, PRD 08 uses a 9th (`rk://turn_start`). → **one canonical event table** (in PRD 06, cited by 08 and data-model). *(integration, me)*
7. **`feed`→`app` layering cycle.** The `agent` subagent tool lives in `feed` but must construct a `Session` (in `app`, which imports `feed`). → introduce a **`SessionFactory`/spawn trait** in a low crate (or host `agent` in `app`); record as ADR. *(my finding; extends systems N3)*
8. **Two "task" concepts.** `TaskState`/`task.json` (working memory) vs `task_create` background-op registry (in-session). → disambiguation note in data-model. *(systems N2)*
9. Minor: `checks.toml` precedence (project vs `.rustykeys/`), session identity for `/resume`/`session_id`, `EpisodePackage` shown only as JSON. *(systems N5/N6/N1c)*

### B. Data model (the central new doc, upstream of everything)
`.rustykeys/` tree + DB filenames (harness DBs are unnamed today; the only `.db` is an *external* mcp-server's); concrete **SQLite DDL** for the short-term stream and long-term store (+FTS5, edge table); all JSONL/TOML/JSON schemas; **`schema_version` on every record + `PRAGMA user_version` per DB**; **torn-line/atomic-append policy** (+ make `count_turns()` tolerant); serde conventions. Consolidates from PRDs 02–07. *(systems G4–G7, N4)*

### C. Standalone architecture doc
Component map, **formal DAG with node/edge list + Mermaid diagram** (including the unstated `compose→observe` and `compose→feed` edges; prove acyclicity); concurrency model summary; **topology section + feature-flag matrix**; **failure modes** (mid-turn LLM failure with already-executed side effects; retry policy; episode-abort); **SQLite contention** (WAL + `busy_timeout` + single-writer for multi-session gateway/MCP sharing one DB and `task.json`); **NFR/quality-attributes**; **faithfulness map** to the paper. *(systems G1/G3/G8/G9/G10, harness P0)*

### D. Error model + engineering substrate (software lens — lands *with* Phase 1)
Unified **`thiserror`-per-library-crate** taxonomy, `#[from]` composition, **anyhow only in `app`**; convert `PolicyError` struct→enum (structured attribution); the **no-panic rule (ADR-007) backed by `unwrap_used`/`panic` lints**; a single **`ToolOutcome` type + one formatter/parser** replacing the fragile magic-prefix `ToolStatus` inference (the `ToolResultClassifier` seam, pulled forward); **define the `ToolFn` trait** and show the **aisdk `#[tool]`→`Box<dyn ToolFn>` registration adapter** (the load-bearing aisdk↔harness seam); trait-object-vs-generics convention; async-trait mechanism + MSRV; feature-flag table; CI pipeline; public-API visibility policy. *(software G1–G9, N1–N5; integration error-taxonomy)*

### E. Testing + eval (distinct docs, shared fixture format)
**testing-strategy.md** (engineering substrate): 4 tiers + the keystone **`FakeLanguageModel`** scripted-turn fixture + **golden-episode deterministic replay** of the compose/verify logic. **eval-plan.md** (maturity measurement): live M-HIR + 5-label histograms + judge-unavailable rate, golden-episode regression, **H0→H3 progression gates with measurable exit criteria**, and the paper's full metric family (AVSR, verification autonomy, context-trace meaningfulness, tool-recovery rate, attribution completeness, entropy delta). They share the episode-package JSON fixture (PRD 05) — reference, don't duplicate. *(software G4; ai G9; harness)*

### F. Model-facing intelligence (ai lens — pin the algorithms; mark "v1 intent, revisit after a spike")
System-prompt construction (producer + layered template + how it composes with `extra_context`; resolve the Task-State-in-two-places contradiction); **recall scoring formula** (weights, decay, cross-domain normalization, neighbor rule, output-block format, token cap); embedding strategy; **consolidation JSON contract** (Memory emit schema, importance rubric, create-vs-merge); **CriteriaJudge robustness** (parse-failure must NOT silently pass-as-verified → journal `judge_unavailable`, bar `AutonomousVerifiedSuccess`; optional self-consistency); per-role model knobs; context ordering/de-dup + token-budget line items; subagent system-prompt inheritance; **close the self-improvement loop** (feed `Attribution` into consolidation; boost failure-born skills at recall). *(ai 1–8, N1–N5; harness loop)*

### G. Faithfulness to the paper → deliberate divergences become ADRs
M-HIR denominator **turns vs episodes**; **episode = turn vs whole task** (+`episode_id` grouping); intervention log missing **avoidability/harness_gap/burden** (so it's raw-HIR not *M*-HIR) and the 7 kinds are an RK invention; **entropy 6 vs 7 categories** (+paper→RK map); **failure attribution free-strings vs the paper's fixed 8-type `FailureType` enum**; **`context_trace` missing** from the episode package (paper has 8 traces); **H0 unreachable** + visibility monotonicity not enforced; **verify→re-attribute back-edge** missing; entropy heuristics + 0–3 severity thresholds. *(harness 2/4, N1–N7)*

### H. Integration boundaries (pin the cross-cutting bits NOW; defer surface detail to Phases 12/14/15)
**NOW:** aisdk integration policy (version pin, per-call timeout, retry/backoff+jitter, `429`/`Retry-After`, retryable-vs-terminal `KernelError`; decide it lives in a shared aisdk-client wrapper since kernel/judge/consolidate/embed/summarise all call aisdk); **secret-redaction rule as a required default** (deny-list arg keys + value scrub before anything hits evidence/security logs, `/evidence`, `rk://tool_event`); **web-tool egress/SSRF guard** (block loopback/RFC1918/link-local/`169.254.169.254`, redirect+size caps, `RUSTYKEYS_WEB_ALLOWLIST/_DENYLIST`); boundary **error→surface mapping** (CLI text / HTTP status+body / Tauri invoke rejection); `config_set` hot-reload-vs-restart note; integration-test seams per boundary. **DEFER:** SSE `/stream` framing, multi-session TTL/eviction + auth binding, MCP SSE reconnect/heartbeat, turn cancellation/backpressure, `/health` liveness-vs-readiness — but sketch the protocol shapes + auth-header convention now. *(integration §2/§4/§5)*

## Locked deliverable set

### New documents (priority, owner)
| Doc | Purpose | P | Owner |
|---|---|---|---|
| `docs/architecture/data-model.md` | On-disk SSOT (tree, DDL, schemas, versioning, serde) | P0 | **author (me)** |
| `docs/ARCHITECTURE.md` | System architecture + faithfulness map | P0 | **author (me)** |
| `docs/reference/configuration.md` | Env-var SSOT (extract from PRD 06; add new vars) | P0 | author (me) |
| `docs/adr/0001..NNNN` | Extract 15 inline ADRs + add new ones | P0 | sub-agent (mechanical) |
| `docs/dev/error-handling.md` | Error taxonomy + `ToolOutcome` contract | P0 | sub-agent (software sketch) |
| `docs/dev/testing-strategy.md` | 4 tiers + FakeLanguageModel + golden replay | P0 | sub-agent (software sketch) |
| `docs/dev/eval-plan.md` | Maturity metrics + H0→H3 gates | P0 | sub-agent (ai+harness sketch) |
| `docs/dev/coding-standards.md` | MSRV, lints, async-trait, features, CI, visibility | P1 | sub-agent (software sketch) |
| `docs/architecture/threat-model.md` | Trust boundaries (LLM semi-trusted), redaction, egress, auth | P1 | sub-agent |
| `docs/reference/glossary.md` | Concept reference (H-levels, M-HIR, episode, the verbs, labels) | P2 | sub-agent |
| `docs/README.md` | Docs index | P1 | author (me, at stitch) |

ADRs to ADD (deliberate divergences + new decisions): async `before_tool`; `SessionFactory` (agent-tool cycle); episode=turn (with `episode_id` grouping); intervention model maps UI actions→avoidability/harness-gap; entropy 6→7 reconciliation; fixed `FailureType` taxonomy; `ToolOutcome`/error-taxonomy; trait-object+async-trait convention; secret-redaction-by-default; H0 selectable-or-eval-only.

### Refinements (edit in place — git preserves originals)
| File | Headline changes | P |
|---|---|---|
| `BACKLOG.md` | Refined roadmap: per-phase DoD/AC/sizing/`depends-on`/test-gate/risk/demo; phase DAG (Mermaid); risk register; engineering-substrate lands with Phase 1; sequence async-`before_tool` (breaking) before MCP/gateway; add H0-instantiation + controlled-visibility + redaction/egress + integration-test workstreams | P0 |
| `docs/prd/00-overview.md` | Slim: extract 15 ADRs → `docs/adr/`; move component map/DAG/maturity → ARCHITECTURE.md; **fix crate count to 8**; become product brief + index; H0 selectable-or-eval-only note | P0 |
| `docs/prd/03-feed.md` | System-prompt construction; recall formula; consolidation contract; **add `TaskState.scope`**; define `ToolFn` + aisdk adapter; close the loop; subagent prompt | P0 |
| `docs/prd/04-observe.md` | Entropy heuristics+thresholds + paper→RK 6↔7 map; M-HIR formula + `avoidability/harness_gap/burden` + edge cases; `ToolOutcome` status (kill magic-prefix); redaction note | P0 |
| `docs/prd/05-compose.md` | Versioned `EpisodePackage` + `context_trace` (8 traces); **fixed `FailureType` enum**; CriteriaJudge no-silent-pass; verify→re-attribute back-edge; freeze `(category,layer)` matrix; serde | P0 |
| `docs/prd/06-app.md` | Canonical `rk://` event table (+`turn_start`); boundary error taxonomy + per-surface map; config additions; session lifecycle note; `SessionFactory`; link config/data-model SSOTs | P0 |
| `docs/prd/01-kernel.md` | System-prompt producer; **fix `kernel.run` signature**; aisdk integration policy (or link a shared client) | P1 |
| `docs/prd/02-constrain.md` | **`async before_tool`**; `PolicyError` struct→enum; redaction default; web egress hook | P1 |
| `docs/prd/07-mcp.md` | SSE auth-header convention; reconnect/heartbeat (promote from seam); cross-links | P2 |
| `docs/prd/08-frontend.md` | Cite the canonical `rk://` table (incl. `turn_start`); cross-links | P2 |
| `README.md` | Link ARCHITECTURE.md + docs index; fix the 4-box diagram to show kernel/config/mcp | P2 |

## Execution waves
- **Wave 1 (done):** this consolidated plan.
- **Wave 2 — Foundations (author myself, mostly serial):** `data-model.md` → `configuration.md` → `ARCHITECTURE.md` (+ launch ADR extraction in parallel, mechanical). These set the conventions everything else cites. Commit + push.
- **Wave 3 — Cross-cutting + PRDs (parallel sub-agents, each cites the Wave-2 SSOTs):** `error-handling.md`, `testing-strategy.md`, `eval-plan.md`, `coding-standards.md`, `threat-model.md`, `glossary.md`; PRD refinements 00–08 + README. Commit + push.
- **Wave 4 — Roadmap + stitch (author myself):** refined `BACKLOG.md`; `docs/README.md` index; cross-reference + terminology consistency sweep. Commit + push.

## PDF verification caveat (carry into ARCHITECTURE.md faithfulness map)
The research PDF is **not renderable in this environment** (no poppler/pdftotext/pypdf). The harness reviewer recovered ~87KB via raw zlib `FlateDecode`, but inter-word spaces and ligatures were stripped. Before **freezing** the P0 faithfulness edits, a human (or a poppler-equipped run) must confirm against the rendered PDF: (a) the exact 7 entropy categories and that severity is 0–3; (b) the M-HIR denominator wording ("total episodes"); (c) the intervention-log fields (avoidability / burden / harness-gap). These three drive P0 edits in PRD 04/05.

## Open product decisions (flag to owner; sketch shape, owner sets numbers)
Recall weights/decay τ; H0–H3 progression-gate thresholds; entropy severity cut-offs; acceptable judge-nondeterminism budget; whether H0 is a runtime mode or eval-only.
