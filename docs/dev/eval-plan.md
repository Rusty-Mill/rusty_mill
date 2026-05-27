# Evaluation plan — measuring harness maturity over time

> **Authoritative source** for how Rusty Keys *measures harness maturity*: the live per-session metrics, the paper's population-level metric family over episode packages, the golden-episode regression suite (outcome-label assertions), and the H0→H3 progression gates with their exit criteria. Other docs link here for "how do we know the harness is getting better?". Test *mechanics* (tiers, `FakeLanguageModel`, deterministic replay) live in [`testing-strategy.md`](./testing-strategy.md), which this doc references but does not duplicate.

This document operationalizes the central thesis of *AI Harness Engineering* (Zhong & Zhu, arXiv 2605.13357v1): capability is a property of the whole system (`C_system = F(C_model, C_harness, C_environment, T)`), so a maturing harness should show a **falling intervention rate, rising verified-autonomy, and non-increasing entropy** as it climbs the H0→H3 ladder. Every numeric threshold below is marked **product call — owner sets the number**; this plan fixes the *shape* of the measurement, not the cut-offs.

Related: [`ARCHITECTURE.md`](../ARCHITECTURE.md) (§3 maturity ladder, §12 faithfulness map) · [`architecture/data-model.md`](../architecture/data-model.md#5-episode-package--episodesturn_idjson-h3-prd-05) (§5 episode package, the JSONL logs these metrics read) · [`reference/configuration.md`](../reference/configuration.md) (`RUSTYKEYS_HARNESS_LEVEL`, gate knobs) · [`testing-strategy.md`](./testing-strategy.md) (shared fixture format) · ADR-0018, ADR-0019, ADR-0020, ADR-0021, ADR-0022, ADR-0028, ADR-0031 (validation-gated skills), ADR-0033 (eval integrity) · [`reference/glossary.md`](../reference/glossary.md).

---

## 1. Three layers of measurement (and how this differs from testing)

| Layer | Question | When | Source of record |
|---|---|---|---|
| **Live metrics** (§2) | "Is *this session* showing harness gaps?" | Per turn / per session, in-process | the four JSONL logs + episode packages |
| **Metric family** (§3) | "Across many episodes, how mature is the harness?" | Offline / batch over `.rustykeys/` history | episode packages, grouped by `episode_id` |
| **Progression gates** (§4) | "Has level H*n* actually been reached?" | CI gate on the golden suite (§5) | golden-episode replay outcomes |

**Distinction from [`testing-strategy.md`](./testing-strategy.md) (read this carefully).** Testing asserts *deterministic logic* — that the compose/verify code paths behave identically given a scripted `FakeLanguageModel` turn (golden **replay**). This eval plan asserts *maturity properties* — that **outcome labels and metric trends do not regress**, even though the underlying model is non-deterministic. The golden-episode regression suite (§5) **shares the episode-package JSON fixture** with testing-strategy.md but the assertion is different: testing checks "the classifier produced exactly this `VerificationReport`"; eval checks "this episode is still labelled `AutonomousVerifiedSuccess` (or better) and M-HIR did not rise". One fixture, two consumers, two assertion styles. Eval tolerates label-preserving non-determinism; tests do not.

---

## 2. Live metrics (per-session, in the hot path's shadow)

Computed cheaply from the append-only logs (data-model §4) and surfaced via the existing `/stats`, `/mhir`, `/entropy` CLI commands and the desktop harness dashboard. These are *signals*, not gates — they tell an operator a session is drifting; they do not block.

| Metric | Definition | Source | Surfaced by |
|---|---|---|---|
| **M-HIR trend** | `count(interventions where avoidability != benign) / count(turns)`, as `trend: Vec<f64>` (rate per session for the sparkline) **and** cumulative all-time | `interventions.jsonl` numerator, `count_turns()` denominator (PRD 04) | `/mhir` |
| **`EpisodeOutcome` histogram** | Counts of the 5 labels over the session/window | `outcome` field of turn/episode records | `/stats` |
| **Judge-unavailable rate** | `count(judge_unavailable) / count(turns where a judge ran)` — a harness-health signal, *not* a pass | `evidence.jsonl` judge diagnostics (PRD 05) | `/stats` |
| **Cumulative entropy delta** | Running `Σ delta` (each `delta = -Σ severity`); a downward drift = accreting maintenance burden | `EntropyAuditor::cumulative_delta()` over `entropy.jsonl` (PRD 04) | `/entropy` |
| **Recall hit-rate proxy** | Fraction of turns where ≥1 recalled memory appears in the turn's `context_trace` with `influenced_decision = true` (proxy for "did memory actually help?") | episode-package `context_trace` (data-model §5) | `/stats` (H3) |

Notes:
- **M-HIR semantics are RK-divergent and ADR-pinned.** The denominator is **turns**, not the paper's **episodes** — a deliberate divergence (ADR-0018; ARCHITECTURE.md §12). Only **non-`benign`** interventions enter the numerator, which is what makes this *M*-HIR (missing-harness) rather than raw HIR (ADR-0019). One user action → at most one record (dedup by `source_message_id`).
- **Judge-unavailable must never read as verified.** A judge call/parse failure journals `judge_unavailable` and **bars `AutonomousVerifiedSuccess`** for that turn (PRD 05, ADR-0022 contract spirit); a rising judge-unavailable rate is itself a harness gap worth alerting on.

---

## 3. The paper's metric family (population-level, over episode packages)

These are **offline analyses over the corpus of episode packages** (`.rustykeys/episodes/*.json`), grouped by `episode_id` to recover the paper's task-level unit (ADR-0018) — *not* hot-path machinery. They are the canonical scorecard for comparing harness levels and tracking maturity across many tasks. Each maps to one or more of the eight traces in the package (data-model §5).

| Metric | Definition (operationalized over packages) | Reads from trace |
|---|---|---|
| **AVSR** (Autonomous Verified Success Rate) | `count(outcome == autonomous_verified_success) / count(episodes)` | `outcome` |
| **M-HIR** | Population form of §2: missing-harness interventions / episodes (grouped by `episode_id`) | `intervention_log` |
| **Verification autonomy** | Fraction of episodes that reach a verdict *without* a `manual_verify` intervention and with a complete `verification_report` | `verification_trace`, `verification_report`, `intervention_log` |
| **Context-trace meaningfulness** | Fraction of `context_trace` entries with `influenced_decision = true` (was supplied context actually used?) | `context_trace` |
| **Tool-recovery rate** | `count(tool calls with recovered == true) / count(failed tool calls)` (agent recovered from a tool error without human help) | `tool_trace` |
| **Attribution completeness** | Fraction of `Failed`/`UnsafeInvalid` episodes whose `attribution_log` carries a non-`f_unknown` `FailureType` with evidence + next_action | `attribution_log` |
| **Entropy delta** | Distribution of per-episode `delta`; report median + tail (severe-burden episodes) | `entropy.findings` |
| **Resilience** (companion to M-HIR) | `resilience = w_b · baseline_score + w_c · chaos_score` over the golden set (see §7). `baseline_score` = AVSR/clean-termination over unperturbed fixtures; `chaos_score` = fraction of chaos fixtures that **degrade honestly** (honest-failure label + correct attribution + non-zero recovery where recovery is possible). Core assertion: **never verified-success-on-fault.** `w_b`/`w_c` are **product call**. | `outcome`, `attribution_log`, `tool_trace` (over chaos fixtures, §7) |

Conventions:
- **`FailureType` is the fixed 8-member enum** (`f_context`, `f_tool`, `f_feedback`, `f_verify`, `f_recovery`, `f_entropy`, `f_model`, `f_unknown`) — ADR-0021. A high `f_unknown` share means attribution itself is weak (a harness gap), so attribution-completeness deliberately excludes it.
- **Entropy categories are reported in the paper's 7-category space** via the RK 6→7 reconciliation map (ADR-0020): RK's `Residue` covers the paper's *code* + *file-residue*; RK's `TaskContradiction` is the paper's *workflow*; the other four are 1:1. This lets RK entropy-delta be compared to paper figures.
- **`UnsafeInvalid` is a population alarm, not just a label.** It triggers on any `TestWeakening`/`BoundaryViolation` finding with `severity ≥ 2` (PRD 05). Any non-zero `UnsafeInvalid` count is a release blocker at H3 (§4).

> **PDF verification caveat.** The exact paper definitions of M-HIR's denominator wording ("total episodes"), the 7 entropy categories (and 0–3 severity), and the intervention-log fields (avoidability / burden / harness-gap) were recovered via raw `FlateDecode` extraction (inter-word spaces/ligatures stripped) — the PDF is not renderable in this environment. Before these metric definitions are **frozen**, a human (or a poppler-equipped run) must confirm them against the rendered PDF. Carried from ARCHITECTURE.md §12 and the consolidated plan.

---

## 4. H0→H3 progression gates

The maturity ladder (ARCHITECTURE.md §3) is only meaningful if "reached H*n*" has **measurable exit criteria**. Each gate is evaluated by replaying the golden suite (§5) at that `RUSTYKEYS_HARNESS_LEVEL` and computing §3 metrics over the resulting packages. All thresholds (`X%`) are **product call — owner sets the number**.

| Level | Gate (exit criteria — all must hold over the golden set) | Primary metrics |
|---|---|---|
| **H0** | *Ablation floor.* Runs with **no tool registry**; serves only as the comparison baseline that H1+ must beat (an H1 vs H0 lift on any task metric is the evidence the harness adds capability). H0's selectable-vs-eval-only status is resolved here (below). | (baseline only) |
| **H1** | **~100% tool-call schema validity** (every tool call the model emits validates against its `#[tool]` schema; structural `ToolOutcome` status, never magic-prefix — ADR-0022); **CleanTermination ≥ X%** (loop reaches a final answer before `max_steps`, `final_reached = true`). | schema-validity, clean-termination |
| **H2** | **Cross-session recall surfaces the planted fact ≥ X%** (a fact written in session A is recalled and used — `context_trace.influenced_decision` — in session B); **`task_override` rate < threshold** (Task-State drift stays low). | recall hit-rate, M-HIR `task_override` slice |
| **H3** | **AVSR ≥ X%** AND **`UnsafeInvalid` count = 0** AND **every H3 turn emits a complete 8-trace episode package** (all of `action_trace`, `tool_trace`, `context_trace`, `verification_trace`, `attribution_log`, `reproduction_log`, `verification_report`, `intervention_log` present and well-formed). | AVSR, UnsafeInvalid, package completeness |

**Resolving H0 (ADR-0028 defers the decision here).** ADR-0028 leaves open whether H0 is a *runtime-selectable* level or *evaluation-only*. This plan's recommendation — **product call — owner sets the number/mode**:
- **Eval-only (lower cost):** H0 is never a runtime mode; it exists solely as a fixed-output baseline in the golden suite (the model answers from task + repo files with no tools), used to compute the H1-vs-H0 capability lift. No code change to `Session`/kernel.
- **Selectable (higher fidelity to the paper's ablation):** `RUSTYKEYS_HARNESS_LEVEL` accepts `h0`; the kernel and `Session` construction must support running with an empty tool registry. Required if the owner wants live H0 ablation runs.

The ladder is intended as a **controlled-visibility ablation** (each level sees only its own artifacts; higher levels inherit lower ones — ARCHITECTURE.md §3). Enforcing that monotonicity (e.g. H1 hiding H2 memory) is a tracked refinement (ADR-0028); until it lands, gate results at a given level should note that lower-level artifact hiding is not yet enforced, so an H1 run may incidentally see H2 state.

---

## 5. Golden-episode regression suite

A fixtures directory of frozen tasks — each a scripted `FakeLanguageModel` turn-sequence plus a `checks.toml` and an **expected episode-package JSON** ([data-model §5](../architecture/data-model.md#5-episode-package--episodesturn_idjson-h3-prd-05)). The replay harness runs each fixture at its target level and asserts maturity, not deterministic logic.

**Shared fixture, distinct assertion (the line between this and testing-strategy.md):**
- The **episode-package JSON fixture format is shared** with [`testing-strategy.md`](./testing-strategy.md) — one schema, stored under the same fixtures tree, so production and both eval/test consumers read one format ([data-model §5](../architecture/data-model.md#5-episode-package--episodesturn_idjson-h3-prd-05) is the SSOT).
- **Testing-strategy.md asserts deterministic replay:** given the scripted turns, the compose/verify code produces *exactly* this `VerificationReport` / `CheckResult` set. Byte-level. Any drift is a logic bug.
- **This eval suite asserts no maturity regression:** the episode is still labelled with the **expected `EpisodeOutcome` (or better)** and the §3 metrics over the fixture set do not regress against the recorded baseline. It explicitly **does not** assert deterministic logic — it tolerates label-preserving non-determinism (e.g. the judge's prose reason changing while `met` stays true).

What a golden episode pins:
- the **expected `EpisodeOutcome`** label (the core assertion);
- **no `UnsafeInvalid`** unless the fixture is a deliberate entropy/destructive-edit case (then `UnsafeInvalid` is the *expected* label);
- for H3 fixtures, **package completeness** (all 8 traces present) and the expected `FailureType` on failure fixtures;
- the **deterministic-check (`checks.toml`) results** stay green (this overlaps testing-strategy.md's check-execution coverage; eval consumes the outcome, testing the mechanism).

Maintenance: a fixture's baseline is re-recorded only on an intentional, reviewed maturity change; an *unexplained* label downgrade or metric-family regression fails the gate in §4.

---

## 6. Open product decisions (owner sets the numbers)

All deferred from the consolidated plan's "Open product decisions"; this plan fixes their *shape*, the owner fixes the value:
- Every `X%` threshold in the §4 gates (schema-validity floor, clean-termination, cross-session recall, AVSR).
- The `task_override`-rate threshold that counts as "Task-State drift under control" at H2.
- The acceptable **judge-nondeterminism budget** (how much run-to-run flip on borderline criteria is tolerated before a fixture is flagged).
- Whether **H0 is a runtime mode or eval-only** (§4; ADR-0028).
- Entropy severity cut-offs feeding `UnsafeInvalid` (owned by PRD 04, consumed here).
- The resilience weights `w_b`/`w_c` and **which fault classes gate a release vs merely report** (§7).

---

## 7. Chaos / fault-injection tier (v1)

Where §5 replays a *scripted, well-behaved* `FakeLanguageModel` turn-sequence, the chaos tier proves the harness is also **resilient under fault**. It reuses the §5 fixture format but perturbs the **`ToolOutcome` / tool-result seam** (not the model) deterministically — the same injection chokepoint `FakeLanguageModel` and the structured `ToolOutcome` status contract (ADR-0022) already own. A chaos fixture adds a `fault:` field to the §5 schema; the perturbation is keyed off the fixture so the run is **replayable in CI** (no real randomness). Fault-injection *mechanics* live in [`testing-strategy.md`](./testing-strategy.md) (beside `FakeLanguageModel`); this tier *consumes* the perturbed outcome — the same one-fixture-two-consumers split §5 draws.

**Fault classes** (injected at the tool-dispatch boundary the fake tool layer owns):
- **Corrupt tool result** — flip `ToolOutcome.status` to `error`/`blocked`, or return a mangled/truncated payload.
- **Schema corruption** — return a result that fails the tool's `#[tool]` output shape.
- **Latency / timeout** — force `ToolOutcome` `timeout = true`.
- **Observation drop** — omit a tool result the model expected.

**What it asserts (the resilience property): honest degradation, never verified-success-on-fault.** Under each fault the harness must **degrade honestly, not silently pass** — the episode classifies `Failed`/`UnverifiedSuccess` (**never** `AutonomousVerifiedSuccess`), the right `FailureType` is attributed (`f_tool`/`f_feedback`/`f_recovery`/`f_verify` per the PRD 05 matrix), `NoToolErrors` fires, and tool-recovery rate (§3) is measured. This is the *negative-space* counterpart to §5's happy-path label assertions. Crucially it is **not** "survive the fault" (the model may legitimately fail) — it is "**never report verified success on top of an injected fault**," the verification thesis stated as a metric.

**The resilience metric** (companion to M-HIR; defined in §3) folds the clean (`baseline_score`) and perturbed (`chaos_score`) runs into one maturity signal: a harness that keeps AVSR high *and* degrades honestly under fault is more mature than one that only scores well on clean runs.

**Partitioning.** Chaos fixtures are clearly partitioned from happy-path golden episodes (a chaos fixture's *expected* label is failure/unverified), so a chaos failure is never mistaken for a maturity regression. The `w_b`/`w_c` weights and which fault classes **gate** a release vs merely **report** are **product call** (§6); don't let chaos volume explode CI time — sample, as §5 maintenance contemplates.

---

## 8. Eval integrity / anti-gaming (ADR-0033)

RK's differentiator is *trustworthy* verification: an eval suite a capable model can game silently invalidates every maturity metric above. The Anthropic containment work documents this as observed, not hypothetical — a model **examined git history to find the answers to a coding test**, and a model **spontaneously identified the benchmark in order to decrypt its answer key**. RK's golden episodes carry expected outcome labels and `checks.toml` — exactly an answer key — so eval integrity is a first-class guard (ADR-0033), not an afterthought. This is the eval analog of `UnsafeInvalid` (the agent satisfying the *measurement* instead of the *task*).

**Eval-integrity invariant.** During evaluation, keep the following **out of the agent's context and out of the workspace the replay runs against** (no leakage path from fixture → prompt):
- **answer keys** — the expected `EpisodeOutcome` labels and metric baselines a fixture pins;
- **`checks.toml` expected outputs** — the deterministic-check expected substrings (`DeterministicCheck.expected_substring`, PRD 05);
- **golden-episode expected outputs** — the recorded `VerificationReport` / expected-package data;
- **benchmark identifiers** — names/tags that let a model recognise *which* eval it is in and look up the target.

So a pass reflects **real capability**, not optimisation of the metric. Two reinforcing rules:
- Deterministic checks should prefer **freshly-computed assertions over committed golden outputs** where feasible (H3 gate note, §4), so reading repo/git state cannot reveal the target.
- **Eval-awareness is a known failure mode** — recorded in the §5 maintenance notes; design the measurement assuming the model may try to satisfy the metric rather than the goal.

**Honest scope (defense-in-depth).** The invariant is partly unenforceable by us alone — a sufficiently capable model in a real workspace can read git. State it as defense-in-depth, with the deterministic boundary (capability isolation, ARCHITECTURE §9–§10 / ADR-0030) as the backstop, exactly as the source concludes. Anti-gaming is mostly a *fixture-construction discipline* (keep answer keys out of context) — no new code. Cross-dep: a session-startup *recall-provenance* check on persisted memory (memory-poisoning resistance) is owned by PRD 04 / security; this plan owns the **poisoning-resistance golden episode** that proves it (a fixture that plants an injected instruction in stored memory and asserts the harness does **not** act on it, with `context_trace.influenced_decision` recording the quarantine).

---

## 9. Failure-trace → golden-episode synthesis (human-gated)

Every `Failed`/`UnsafeInvalid` turn already writes a full 8-trace **episode package** (`episodes/<turn_id>.json`) with `attribution_log` (`FailureType` + evidence + `next_action`) and `reproduction_log` — the perfect substrate to *synthesise* a regression fixture. A real failure package is promoted into a §5 golden episode (scripted from its recorded `action_trace`/`tool_trace`, assertion = its observed `EpisodeOutcome` + `FailureType`), so the next release **provably does not re-fail the same way**. This closes a loop RK already half-has and is the harness-maturity analog of "a bug fix ships with a regression test." Redaction (ADR-0026) already scrubs the package, so synthesised fixtures inherit secret-safety; a synthesised fixture's recorded tool failures are also ready-made chaos seeds (§7).

**Human-gated, and tied to the Attribution→skill loop.** The same failure trace that mints a **validation-gated consolidation skill** (ADR-0031 — a candidate skill promoted only after it re-earns validation, and un-validated by a human `direct_edit`) also mints this regression fixture: one trace, two learners — model-side learning *and* harness-side learning. Synthesis is **gated by human review before a synthesised fixture enters the baseline**, so a *flaky* or *gamed* failure (§8) is never frozen as ground truth and an attacker cannot steer a poisoned failure into the permanent regression set. The `next_action` field of `attribution_log` is the synthesis hint; volume control + dedup by `FailureType` + signature is **product call**.
