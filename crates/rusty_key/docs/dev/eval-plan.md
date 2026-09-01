# Evaluation plan — measuring harness maturity over time

> **Authoritative source** for how Rusty Keys *measures harness maturity*: the live per-session metrics, the paper's population-level metric family over episode packages, the golden-episode regression suite (outcome-label assertions), and the H0→H3 progression gates with their exit criteria. Other docs link here for "how do we know the harness is getting better?". Test *mechanics* (tiers, `FakeLanguageModel`, deterministic replay) live in [`testing-strategy.md`](./testing-strategy.md), which this doc references but does not duplicate.

This document operationalizes the central thesis of *AI Harness Engineering* (Zhong & Zhu, arXiv 2605.13357v1): capability is a property of the whole system (`C_system = F(C_model, C_harness, C_environment, T)`), so a maturing harness should show a **falling intervention rate, rising verified-autonomy, and non-increasing entropy** as it climbs the H0→H3 ladder. Every numeric threshold below is marked **product call — owner sets the number**; this plan fixes the *shape* of the measurement, not the cut-offs.

Related: [`ARCHITECTURE.md`](../ARCHITECTURE.md) (§3 maturity ladder, §12 faithfulness map) · [`architecture/data-model.md`](../architecture/data-model.md#5-episode-package--episodesturn_idjson-h3-prd-05) (§5 episode package, the JSONL logs these metrics read) · [`reference/configuration.md`](../reference/configuration.md) (`RUSTYKEYS_HARNESS_LEVEL`, gate knobs) · [`testing-strategy.md`](./testing-strategy.md) (shared fixture format) · ADR-0018, ADR-0019, ADR-0020, ADR-0021, ADR-0022, ADR-0028, ADR-0031 (validation-gated skills), ADR-0033 (eval integrity), ADR-0035 (controlled-visibility ablation eval-substrate, §4.1) · [`reference/glossary.md`](../reference/glossary.md).

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
| **M-HIR trend** | `count(interventions where avoidability == avoidable) / count(turns)`, as `trend: Vec<f64>` (rate per session for the sparkline) **and** cumulative all-time | `interventions.jsonl` numerator, `count_turns()` denominator (PRD 04) | `/mhir` |
| **`EpisodeOutcome` histogram** | Counts of the 5 labels over the session/window | `outcome` field of turn/episode records | `/stats` |
| **Judge-unavailable rate** | `count(judge_unavailable) / count(turns where a judge ran)` — a harness-health signal, *not* a pass | `evidence.jsonl` judge diagnostics (PRD 05) | `/stats` |
| **Cumulative entropy delta** | Running `Σ delta` (each `delta = -Σ severity`); a downward drift = accreting maintenance burden | `EntropyAuditor::cumulative_delta()` over `entropy.jsonl` (PRD 04) | `/entropy` |
| **Recall hit-rate proxy** | Fraction of turns where ≥1 recalled memory appears in the turn's `context_trace` with `influenced_decision = true` (proxy for "did memory actually help?") | episode-package `context_trace` (data-model §5) | `/stats` (H3) |

Notes:
- **M-HIR semantics are RK-divergent and ADR-pinned.** The denominator is **turns**, not the paper's **episodes** — a deliberate divergence (ADR-0018; ARCHITECTURE.md §12). The numerator counts **avoidable interventions only** (D2/F23, round 3): an intervention enters M-HIR only when it represents runtime support *"the human would otherwise have to provide"* (paper p.4) — i.e. a harness gap the system could have closed. This is what makes this *M*-HIR (missing-harness) rather than raw HIR (ADR-0019). Excluded: **`benign`** interventions (e.g. a default `manual_verify` — inspecting evidence is healthy) and **correct/unavoidable `tool_block`s** — a permission boundary firing on a disallowed action is *the policy working*, not a missing harness, so it is **not** counted (a *recurring* block on the same legitimate action is a policy gap and may be reclassified `avoidable`, at which point it does count — owned by [PRD 04](../prd/04-observe.md)). One user action → at most one record (dedup by `source_message_id`).
- **Judge-unavailable must never read as verified.** A judge call/parse failure journals `judge_unavailable` and **bars `AutonomousVerifiedSuccess`** for that turn (PRD 05, ADR-0022 contract spirit); a rising judge-unavailable rate is itself a harness gap worth alerting on.

---

## 3. The paper's metric family (population-level, over episode packages)

These are **offline analyses over the corpus of episode packages** (`.rustykeys/episodes/*.json`), grouped by `episode_id` to recover the paper's task-level unit (ADR-0018) — *not* hot-path machinery. They are the canonical scorecard for comparing harness levels and tracking maturity across many tasks. Each maps to one or more of the eight traces in the package (data-model §5).

| Metric | Definition (operationalized over packages) | Reads from trace |
|---|---|---|
| **AVSR** (Autonomous Verified Success Rate) | `count(outcome == autonomous_verified_success) / count(episodes)` | `outcome` |
| **M-HIR** | Population form of §2: **avoidable** (missing-harness) interventions / episodes, grouped by `episode_id` (same numerator rule as §2 — `benign` and correct `tool_block`s excluded; D2/F23) | `intervention_log` |
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
| **H0** | *Ablation floor.* Runs with **no tool registry**; the comparison baseline that H1+ must beat (an H1 vs H0 lift on any task metric is the evidence the harness adds capability). Under R5 (§4.1) H0 is **adjudicated by the same evaluator-side checks as every other level** and carries a real `EpisodeOutcome` — e.g. H0 can earn `autonomous_verified_success` when the evaluator's checks + full regression pass with no agent report (paper Table 5). H0's selectable-vs-eval-only status is resolved below. | `EpisodeOutcome` (evaluator-assigned), baseline metrics |
| **H1** | **~100% tool-call schema validity** (every tool call the model emits validates against its `#[tool]` schema; structural `ToolOutcome` status, never magic-prefix — ADR-0022); **CleanTermination ≥ X%** (loop reaches a final answer before `max_steps`, `final_reached = true`). | schema-validity, clean-termination |
| **H2** | **Cross-session recall surfaces the planted fact ≥ X%** (a fact written in session A is recalled and used — `context_trace.influenced_decision` — in session B); **`task_override` rate < threshold** (Task-State drift stays low). | recall hit-rate, M-HIR `task_override` slice |
| **H3** | **AVSR ≥ X%** AND **`UnsafeInvalid` count = 0** AND **every H3 turn emits a complete 8-trace episode package** (all of `action_trace`, `tool_trace`, `context_trace`, `verification_trace`, `attribution_log`, `reproduction_log`, `verification_report`, `intervention_log` present and well-formed). | AVSR, UnsafeInvalid, package completeness |

**Resolving H0 (ADR-0028 defers the decision here).** ADR-0028 leaves open whether H0 is a *runtime-selectable* level or *evaluation-only*. This plan's recommendation — **product call — owner sets the number/mode**:
- **Eval-only (lower cost):** H0 is never a runtime mode; it exists solely as a fixed-output baseline in the golden suite (the model answers from task + repo files with no tools), used to compute the H1-vs-H0 capability lift. No code change to `Session`/kernel.
- **Selectable (higher fidelity to the paper's ablation):** `RUSTYKEYS_HARNESS_LEVEL` accepts `h0`; the kernel and `Session` construction must support running with an empty tool registry. Required if the owner wants live H0 ablation runs.

The ladder is intended as a **controlled-visibility ablation** (each level sees only its own artifacts; higher levels inherit lower ones — ARCHITECTURE.md §3), *not* a merely additive-capability ladder. Building that — the artifact-hiding, the all-levels evaluator pass, and the per-episode isolation it presupposes — is the **eval-substrate workstream** specified in §4.1. **Until it lands, the gates above run on an unenforced ladder** (lower-level runs may incidentally see higher-level state), so a gate result at a given level must be read as provisional: see the gate clause at the end of §4.1.

### 4.1 Controlled-visibility ablation eval-substrate (ADR-0035; D3)

The paper's separability claim — that the H0→H3 ladder *measures* what each harness layer adds (p.2; R1/R5, p.7) — only holds if the ladder is run as a **controlled-visibility ablation**, not the additive-capability ladder RK ships today. The round-3 audit found three coupled gaps (F8 R1 visibility, F9/F10 R5 adjudication, F26 isolation) that the persona-disagreement resolution (round3-consolidated §5) rules are **one eval-substrate concern, not three** — sequenced **isolation → visibility → adjudication**. ADR-0035 owns the decision; this section owns the eval-side method. Each stage is **product call — owner sets the number** only where a threshold appears; the *shape* below is fixed.

**Stage 1 — Per-episode isolation (F26).** Each eval episode runs in a **fresh workspace checked out at a fixed commit, with its own `.rustykeys/`** — no shared working tree, no shared store/DB, no shared `task.json` across episodes. The fixture's `initial_state` must be **ENFORCED, not merely recorded**: the harness materializes the declared commit + workspace and refuses to run an episode whose live state diverges from its `initial_state`, rather than logging the discrepancy after the fact (today RK shares one workspace/DB/`task.json` and only *records* `initial_state`). Isolation is sequenced first because the shared tree is precisely where higher-level artifacts leak into lower-level runs — it is the substrate Stages 2–3 stand on.

**Stage 2 — R1 controlled visibility (F8).** Lower harness levels **do NOT see higher-level artifacts.** H2 memory, the `AGENT_GUIDE`, `TASK_STATE`, and `checks.toml` are **hidden at the feed / context-read seam** — the point where context is assembled and handed to the agent, where the harness withholds an artifact's *existence* (not its *authority*, which is why this is a feed-seam concern, not a `constrain`/permission one — round3-consolidated D3). With Stage 1 in place this becomes enforceable: an H1 episode's isolated workspace simply does not contain the H2 artifacts to read. The point is to **de-confound the H1-vs-H2 contrast**: if an H1 run can incidentally read H2 memory or `checks.toml`, the measured H1↔H2 delta no longer isolates what the H2 layer adds, and the paper's separability claim (p.2; p.7 R1) is not what the gate reports.

**Stage 3 — R5 all-levels adjudication (F9/F10).** The **same evaluator-side deterministic checks** (`CheckRegistry::run_all()`) run at **EVERY level H0–H3** and assign the `EpisodeOutcome` — *not* only at H3, and **NOT** from the agent's self-produced report (`VerificationReportRequired` is the agent's own verification, which is a different thing). An independent evaluator pass labels every level under the one outcome taxonomy, which restores the paper's headline **Table 5** contrast — e.g. **H0 = `autonomous_verified_success`** assigned *by the evaluator's checks* (checks + full regression pass, no agent report at all), set against an H1 run that may be only `unverified_success`. Today RK adjudicates "every H3 turn" *only* and *only* from agent self-report, which conflates "the agent produced evidence" with "the evaluator verified the behaviour"; Stage 3 separates them.

> **Dual-role checks (paper p.10).** Deterministic checks serve two distinct roles, and conflating them is the F10 drift. At **H3** they are **agent-visible** harness artifacts that support the agent's *own* verification (this is the `checks.toml` Stage 2 hides from *lower* levels). At **all levels** the *same* checks are **evaluator-side** adjudication checks that classify the final outcome. The evaluator pass in Stage 3 is the second role; it runs at H0–H3 regardless of whether the checks were visible to the agent at that level. (Consistent with the §8 eval-integrity invariant: the *expected outputs / answer key* of those checks stay out of the agent's context at every level; only at H3 is the *existence* of the check agent-visible.)

**GATE — do not report any Hn-vs-Hm lift until this lands.** Per the persona-disagreement resolution (round3-consolidated §5: side with SYS on severity = High), an Hn-vs-Hm capability-lift claim is **not admissible evidence** while the ladder is unenforced — without Stages 1–2 the contrast is confounded, and without Stage 3 the lower level carries no comparable label. Concretely: the H1-vs-H0 lift (H0 row above, H1 gate), the H1↔H2 contrast (H2 gate), and any "the harness adds capability" statement built on level deltas are **held** until §4.1 is in place. This is sequenced into the **golden-episode replay (§5)** — a **task-grained context where `checks.toml` is meaningful** — and explicitly **NOT** the live per-turn hot path (§2), which is per-turn and has no fixed commit / `checks.toml` to adjudicate against. It remains *acknowledged* divergence-in-progress (this section, ADR-0035, and ARCHITECTURE.md §3 all flag it), not a hidden defect — but it is High severity, not a minor deferral.

---

## 5. Golden-episode regression suite

A fixtures directory of frozen tasks — each a scripted `FakeLanguageModel` turn-sequence plus a `checks.toml` and an **expected episode-package JSON** ([data-model §5](../architecture/data-model.md#5-episode-package--episodesturn_idjson-h3-prd-05)). The replay harness runs each fixture at its target level and asserts maturity, not deterministic logic. This replay is also the **home of the §4.1 eval-substrate**: it is the task-grained context where each fixture's fixed commit + per-episode `.rustykeys/` give Stage-1 isolation, where Stage-2 visibility-hiding is enforced per level, and where Stage-3's evaluator-side `CheckRegistry::run_all()` assigns the `EpisodeOutcome` at *every* level — the per-turn live path (§2) has no fixed commit / `checks.toml` to adjudicate against.

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
- Whether **H0 is a runtime mode or eval-only** (§4; ADR-0028) — distinct from, but adjacent to, the **controlled-visibility ablation eval-substrate** (§4.1; ADR-0035; D3), whose *shape* (isolation → visibility → adjudication) is fixed here and whose landing gates any Hn-vs-Hm lift.
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
