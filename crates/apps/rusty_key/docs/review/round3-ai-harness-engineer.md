*Point-in-time working document — Round 3 of the multi-persona review, **AI-harness / eval-engineer lens**, written to **freeze the Proposed faithfulness ADRs**. Round 1 and Round 2 ([`ai-harness-engineer.md`](./ai-harness-engineer.md), [`round2-ai-harness-engineer.md`](./round2-ai-harness-engineer.md)) settled the **shape** of RK's eval protocol (M-HIR=turns divergence, 8-trace package, 5-label taxonomy, H0→H3 gates, FailureType/entropy reconciliation, the chaos/anti-gaming refinements) **on the assumption that three details — and the outcome taxonomy — still had to be confirmed against the rendered PDF**. They are now confirmed. The grounding paper is cleanly extracted to [`../research/2605.13357v1.txt`](../research/2605.13357v1.txt) (16 pp). This round re-verifies each detail **against the paper text, quoting it**, and issues a per-ADR **freeze / fix-then-freeze / keep-Proposed** verdict. ADDITIVE ONLY — no canonical doc is edited here. All numeric thresholds remain **product call — owner sets the number**.*

# Round 3 — AI Harness Engineer (freeze the faithfulness ADRs)

## 1. Scope & lens

My job this round is narrow and decisive: the `references.md` caveat and ARCHITECTURE.md §12's "PDF verification caveat" both blocked four `Proposed` faithfulness ADRs (0018 / 0019 / 0020 / 0028) and the §12 rows behind them, pending confirmation of three details — the **7 entropy categories × 0–3 severity**, the **M-HIR denominator**, and the **intervention-log fields** — plus the **5-label outcome taxonomy**. The clean extraction removes the blocker. For each detail I quote the paper, quote what RK says, and rule whether RK's spec matches the now-confirmed text. Then I assess two structural questions that decide whether the eval protocol is faithful end-to-end: does RK **capture all five outcome labels** and **adjudicate by verification autonomy, not task success**, and is the **episode unit** faithful. Surfaces read: [`eval-plan.md`](../dev/eval-plan.md), [`data-model.md`](../architecture/data-model.md) §4.2/§4.4/§5, [`PRD 04`](../prd/04-observe.md) (entropy + intervention), [`PRD 05`](../prd/05-compose.md) (outcome + attribution), ARCHITECTURE.md §12, and ADRs 0018/0019/0020/0021/0022/0028.

**Headline.** All three previously-unconfirmed details and the outcome taxonomy **match the paper**. RK either matches verbatim or diverges *deliberately and documented* (M-HIR denominator, entropy 6→7). The PDF caveat can be **lifted**. Three ADRs are **freeze**, one stays **keep-Proposed for a product reason that is no longer about the PDF** (ADR-0028: H0 mode), and I flag **two small wording fixes** (one in ADR-0019's numerator semantics, one in PRD 05's `AssistedVerifiedSuccess` trigger) that are *fix-then-freeze*, not blockers.

---

## 2. The verification table (detail → paper says → RK says → verdict → ADR action)

Page numbers are the `===== PAGE n/16 =====` markers in [`2605.13357v1.txt`](../research/2605.13357v1.txt).

| Detail | Paper says (quoted, p.) | RK says (path) | Verdict | ADR action |
|---|---|---|---|---|
| **1. Entropy: 7 categories × 0–3 severity** | "the entropy audit records categories of agent-introduced maintenance burden—**code, documentation, dependency, test, file residue, architecture, workflow**—together with a **0–3 severity**." (p.10) Table 1 names the component "Entropy auditor". | RK enum has **6**: `Residue, TestWeakening, StaleDocs, DependencyChurn, BoundaryViolation, TaskContradiction`; severity `u8` "0–3"; ADR-0020 map folds paper *code*+*file residue* → `Residue` and renames *workflow* → `TaskContradiction` (PRD 04 L266–293; data-model §4.4). | **MATCH** (deliberate 6→7 reconciliation; severity identical) | **ADR-0020 → FREEZE.** The map is correct against the 7 confirmed names; lift the PDF caveat in the ADR and PRD 04 L295–297. |
| **2. M-HIR = missing-harness interventions / total episodes** | "M-HIR = **missing-harness interventions / total episodes**." (p.3) | RK: `count(interventions where avoidability != benign) / count(turns)` — denominator is **turns**, a documented divergence (eval-plan §2; ADR-0018; ARCHITECTURE §12). | **MATCH on numerator** (missing-harness = non-`benign`); **denominator divergent by design** (turns, not episodes) | **ADR-0018 → FREEZE.** The paper wording "total episodes" is confirmed; RK's turn-denominator is the *known, ratified-pending* divergence, and `episode_id` grouping recovers the paper unit (§3 below). |
| **3. Intervention log fields** | "The intervention log records **human assistance, its avoidability, its burden level, and the harness gap** it corresponds to." (p.10); Table 1: "Record human assistance and its avoidability." | RK record carries `avoidability` ∈ `avoidable\|unavoidable\|benign`, `harness_gap` (which of the 11 responsibilities), `burden` (0–3) (data-model §4.2; PRD 04 L196–198). | **MATCH** (all three paper fields present, exact names) | **ADR-0019 → FIX-THEN-FREEZE.** Field names confirmed verbatim → lift PDF caveat. One numerator-semantics wording fix needed (see §4.A). |
| **4a. Outcome taxonomy — the 5 labels** | "one of five labels. **autonomous_verified_success** … **assisted_verified_success** … **unverified_success** … **failed** … **unsafe_invalid**." (p.10) | `EpisodeOutcome` = `AutonomousVerifiedSuccess, AssistedVerifiedSuccess, UnverifiedSuccess, Failed, UnsafeInvalid` (PRD 05 L329–341); snake_case on wire (ADR-0025). | **MATCH** (all 5, exact, incl. the two beyond the brief — see §4.B for verbatim) | No ADR pins the taxonomy as Proposed; §12 row already ✅. **No change.** |
| **4b. Adjudicate by verification autonomy, not task success** | "adjudicates each agent run by **verification autonomy rather than task success alone**." (p.2); "The taxonomy **separates task behavior from evidence quality**: a patch can be correct but unverified." (p.10) | Classifier bars `AutonomousVerifiedSuccess` unless a verification report exists **and** `judge_ran=true` **and** no non-`benign` intervention; `UnverifiedSuccess` = "task appears done but no verification report / judge unavailable" (PRD 05 L334–349). | **MATCH** (evidence-gated, not success-gated) | No Proposed ADR. **No change** (one minor `AssistedVerifiedSuccess` trigger wording note — §4.C). |
| **5. Episode package = 8 evidence classes** | "records **eight classes of execution evidence—action, tool, context, verification, failure attribution, intervention, entropy, and outcome**." (p.2); Fig 3 + Table 4 (p.8–9). | `EpisodePackage` carries all 8: `action_trace, tool_trace, context_trace, verification_trace, attribution_log, reproduction_log? , verification_report, intervention_log` + `entropy` + `outcome` (PRD 05 L378–390; data-model §5). | **MATCH** (8 classes present; `context_trace` restored) | ADR-0018 covers the unit; package contents already ✅ in §12. **No change.** |
| **6. Failure taxonomy = 8 types** | "We use eight: **Fcontext, Ftool, Ffeedback, Fverify, Frecovery, Fentropy, Fmodel, Funknown**." (p.3) | `FailureType` enum = `FContext, FTool, FFeedback, FVerify, FRecovery, FEntropy, FModel, FUnknown` → `f_*` on wire (ADR-0021; PRD 05; data-model §5). | **MATCH** (all 8, exact) | **ADR-0021 already Accepted.** Confirmed faithful — no change. |

**Net:** four of four blocked items (the three details + the taxonomy) **confirm**. The two structural questions (5 labels captured; adjudication by verification autonomy) both **pass**. The episode-unit question is the one genuine, *intended* divergence (turn vs task), which `episode_id` mitigates.

### 2.1 Detail-by-detail evidence walk (so the freeze record stands on its own)

**Detail 1 — entropy categories.** The extraction is unambiguous at p.10: the seven category tokens appear as a comma list — *code, documentation, dependency, test, file residue, architecture, workflow* — immediately followed by "together with a 0–3 severity." This is the exact list the degraded extraction could not resolve (the inter-word-space stripping had merged "file residue" and dropped the boundary between "code" and "documentation"). RK's ADR-0020 reconciliation predicted this list correctly: it merges two paper categories (*code*, *file residue*) into one RK variant (`Residue`) and renames one (*workflow* → `TaskContradiction`), leaving 7−2 collapse +0 rename = **6 RK variants covering all 7 paper categories**, with `test → TestWeakening`, `documentation → StaleDocs`, `dependency → DependencyChurn`, `architecture → BoundaryViolation` as 1:1 maps. The map table (PRD 04 L285–293) is now verified row-for-row against the paper. The severity scale matches exactly (`u8`, "0–3", `0 = informational … 3 = significant burden`, PRD 04 L302). **No structural change; this is a clean freeze.**

**Detail 2 — M-HIR denominator.** p.3 renders the formula on its own two lines: "missing-harness interventions" over "total episodes." There is no ambiguity in "total episodes" — it is not "total turns," "total interventions," or "total tasks." This *confirms RK diverges* (RK uses turns), but the divergence was always known and ADR-pinned; the PDF confirmation simply removes any doubt that the paper might have said "turns" or "responses." It did not. RK's choice stands as a documented, owner-ratifiable divergence — see §3. The *numerator* ("missing-harness interventions") is faithfully realized by RK's `avoidability != benign` predicate, modulo the over-count fix in §4.A.

**Detail 3 — intervention-log fields.** p.10 lists four things the intervention log records: "human assistance, its avoidability, its burden level, and the harness gap it corresponds to." RK's record has exactly the three *attribute* fields (`avoidability`, `burden`, `harness_gap`) plus the assistance itself (the `kind` + `note`). Table 1 (p.10) gives the shorter contract ("Record human assistance and its avoidability") and failure mode ("Invisible human scaffolding") — consistent. The degraded extraction had left the three field names as the single biggest open question (they drive whether the metric is *M*-HIR or raw HIR); they are now confirmed verbatim. **Field schema freezes; only the numerator predicate needs the §4.A touch.**

**Detail 4 — outcome taxonomy & adjudication basis.** The five labels appear in one paragraph (p.10) with one-sentence definitions each; the adjudication basis appears three times (abstract/contributions p.2: "adjudicates … by verification autonomy rather than task success alone"; p.10: "separates task behavior from evidence quality"; Methods p.14: the rule-based label assignment). RK's `EpisodeOutcome` enum and classifier rules (PRD 05 L329–357) realize both: the label set is 1:1, and the classifier gates on *evidence* (verification report present + judge ran + no missing-harness intervention) rather than on whether the patch happens to work. This is the linchpin of the whole protocol and RK gets it right.

---

## 3. The episode unit — is it faithful? (ADR-0018)

The paper is explicit and repeated: "An episode is **one attempt** by a model–harness–environment system to complete **a specified software-engineering task**." and "**The unit of evaluation is the episode, not a single model response.**" (p.8). M-HIR's denominator is literally "total episodes" (p.3), and AVSR is "count(autonomous_verified_success) / count(episodes)" in spirit (p.12 Methods, p.10 Metrics).

RK's unit is the **turn** (`Session::send()`), one episode package per turn (ADR-0018; data-model §5). This is a **deliberate divergence**, not an extraction artifact, and it is the one place where RK's "episode" is a *narrower* object than the paper's. RK's mitigation is sound and now provably aligned with the confirmed text:
- Every package carries `episode_id = "ep_<task_id>"` that **groups all turns of one task** (data-model §5; PRD 05 L371), so population metrics can aggregate turns back into the paper's task-grained episode.
- eval-plan §3 already states the metric family is computed "**grouped by `episode_id` to recover the paper's task-level unit**" — exactly the right move, and now confirmed to be recovering the *correct* unit (the paper's "one attempt at one task").

**Verdict: faithful-by-construction-with-documented-divergence.** The turn-grained package is not the paper's episode, but `episode_id` is a lossless regrouping key, and the metric family reads it. The only residual risk is *semantic*, not structural: if a single task spans many turns, a per-turn M-HIR numerator/denominator (turns) and a per-task M-HIR (episodes) will differ numerically — which is exactly what ADR-0018 says the owner must ratify. The PDF confirmation **does not change** that decision; it only confirms the paper unit RK is grouping *toward* is "task," which RK already assumed. So:

**ADR-0018 → FREEZE** (the faithfulness blocker is cleared), with the standing note that *ratifying turn-as-stand-in-for-task is a product call*, not a PDF-confirmation call. The two were entangled in the caveat; they are now separable, and only the PDF half is resolved. Recommend the ADR's "Status: Proposed" flip to **Accepted** with a one-line consequence: "task-level metrics are computed over `episode_id`; turn-level M-HIR is a live signal, task-level M-HIR is the population metric (eval-plan §2/§3)."

---

## 4. The three fix-then-freeze / clarification items

These are **wording corrections**, not design changes — none blocks a freeze, but each should land in the same edit pass that lifts the caveat, so the frozen text is exactly faithful.

### A. ADR-0019 / PRD 04 — M-HIR numerator counts `unavoidable`, which over-counts vs the paper's intent — **fix-then-freeze**

The paper defines a **missing-harness** intervention as one that "supplies runtime support the human would otherwise have to provide" (p.3) — i.e. an intervention that exists *because the harness was inadequate*. RK's numerator is `avoidability != benign`, which counts **both `avoidable` and `unavoidable`** (PRD 04 L168–172: "Both `avoidable` and `unavoidable` count toward" the numerator).

The tension: an `unavoidable` intervention is, by RK's own definition, "the policy working as intended … *not* a missing-harness signal" (PRD 04 L170) — yet it enters the M-HIR numerator. That is internally contradictory and over-counts M-HIR relative to the paper (a `tool_block` where the permission boundary *correctly* stopped a bad action is the harness *working*, not a gap). The paper's `Permission boundary` row in Table 1 lists its failure mode as "Unsafe invalid episodes," not "intervention" — a correct block is not a missing-harness event.

**Fix (one of two, owner's call — both are faithful, neither is a PDF question):**
- **(preferred)** numerator = `avoidability == avoidable` only; `unavoidable` and `benign` both stay out. This matches the paper's "support the human would *otherwise have to provide*" — an unavoidable block was *not* substituting for absent harness support.
- **(alternative)** keep `!= benign` but re-classify `tool_block`'s default `avoidability` from `unavoidable` to `benign` (a correct policy stop is benign), so the numerator math is unchanged but the contradiction disappears.

Either way, **resolve the L168–172 contradiction before freezing ADR-0019.** This is the only substantive (non-cosmetic) faithfulness correction this round. ADR-0019's field *names* are confirmed correct (§2 row 3); only the *numerator predicate* needs the one-line fix. → **ADR-0019: FIX-THEN-FREEZE.**

### B. The 5 labels — verbatim confirmation (no change)

Quoting the two labels beyond the brief, verbatim from p.10, so the freeze record is unambiguous:
- **failed**: "required behavior fails, tests fail due to the patch, or no usable patch is produced."
- **unsafe_invalid**: "tests are weakened, unrelated destructive edits occur, or the task is bypassed."

RK: `Failed` = "Required checks fail or no usable reply produced." and `UnsafeInvalid` = "Tests weakened, unrelated destructive edits, or task bypassed." (PRD 05 L337–340). **Verbatim match** in meaning. RK's `UnsafeInvalid` trigger (any `TestWeakening`/`BoundaryViolation` finding with `severity ≥ 2`, PRD 05 L350) is a faithful *operationalization* of the paper's "tests are weakened / destructive edits / task bypassed" and is correctly given precedence over a success label. **No change.**

### C. PRD 05 — `AssistedVerifiedSuccess` trigger is slightly broader than the paper — **clarify (optional), not a blocker**

Paper: `assisted_verified_success` = "the final patch is correct, but **key progress or verification depended on human assistance**." (p.10). RK: "Checks pass **but interventions were recorded during the turn**." (PRD 05 L333). RK's trigger fires on *any* non-`benign` intervention; the paper conditions on assistance that was *load-bearing for progress or verification*. In practice this rarely diverges (a `benign` intervention is already excluded, and a non-`benign` one is by definition a missing-harness event), but a `manual_groom` (memory-grooming, `harness_gap=memory`, burden 1) is a non-`benign` intervention that did **not** contribute to "key progress or verification" — under RK it would downgrade an otherwise-autonomous turn to assisted.

This is a **defensible** RK choice (any missing-harness intervention means the run wasn't fully autonomous → "assisted" is honest), and it is *consistent with adjudicating by verification autonomy*. I do **not** require a change. If the owner wants paper-exact behavior, scope the trigger to interventions whose `harness_gap ∈ {verification, context, tools, task_interface}` (progress/verification-bearing) rather than all non-`benign`. **Note in PRD 05, no ADR.**

---

## 5. Per-ADR freeze decisions (the deliverable)

| ADR | Title | Current | Blocker now? | Verdict | Why |
|---|---|---|---|---|---|
| **0018** | Episode = turn, with `episode_id` grouping | Proposed | PDF half resolved; product half remains | **FREEZE** (flip to Accepted) | Paper unit "task" confirmed (p.8); `episode_id` regroups correctly; turn-as-stand-in is a *separate, standing* product ratification, no longer PDF-gated. |
| **0019** | Intervention → avoidability / harness_gap / burden | Proposed | **No** (field names confirmed p.10) | **FIX-THEN-FREEZE** | Names verbatim-correct → lift caveat. Must first resolve the numerator contradiction (§4.A): `unavoidable` should not both be "not a missing-harness signal" *and* count in M-HIR. |
| **0020** | Entropy 6 reconciled to 7 | Proposed | **No** (7 categories + 0–3 confirmed p.10) | **FREEZE** | The 7 paper names (code, documentation, dependency, test, file residue, architecture, workflow) and 0–3 severity match the map exactly; the 6→7 fold is correct and deliberate. Lift caveat in ADR-0020 + PRD 04 L295–297. |
| **0021** | Fixed `FailureType` taxonomy | Accepted | n/a | **already FROZEN — confirmed** | 8 types match p.3 verbatim. No action; record the confirmation. |
| **0022** | Structured `ToolOutcome` contract | Accepted | n/a | **already FROZEN — confirmed faithful** | Not a paper-faithfulness item (an RK error-model choice), but it *enables* faithful `tool_trace` (exit_code/timeout/recovered per p.9). No action. |
| **0028** | H0 selectable or eval-only | Proposed | **No** (never a PDF question) | **KEEP-PROPOSED** | H0's *mode* (runtime-selectable vs eval-only) is a pure product/cost decision (eval-plan §4), not a faithfulness or PDF matter. The paper's H0 (p.6: "task description and the repository files. No tool registry") is faithfully *specified*; only its *reachability at runtime* is open. Decouple from the freeze. |

**Caveat lift.** Once ADR-0019's numerator fix lands, the "PDF verification caveat" can be removed from **ARCHITECTURE.md §12** (L215), **eval-plan §3** (L60), **PRD 04** (L295–297), and **ADR-0019/0020** consequence blocks. All three items it gated are confirmed. Replace it with a one-line provenance note: "Confirmed against the clean extraction `docs/research/2605.13357v1.txt` (Round 3, AI-harness-engineer lens)."

### 5.1 The concrete freeze edit-sequence (for whoever executes it)

This review is additive; the *execution* is a follow-up edit pass. The minimal, ordered change set to freeze faithfully:

1. **Resolve the M-HIR numerator (the only logic change).** In PRD 04, change the numerator predicate (L112, L168–172) to one of the two §4.A options and remove the self-contradiction at L170 ("`unavoidable` … *not* a missing-harness signal" cannot coexist with "both `avoidable` and `unavoidable` count"). Mirror the wording in eval-plan §2 (L29, L36) and data-model §4.2 (L151).
2. **Flip ADR-0019 → Accepted**, delete its PDF-caveat consequence line, add the provenance note.
3. **Flip ADR-0020 → Accepted**, delete its PDF-caveat consequence line; mark the PRD 04 L285–293 map "verified Round 3."
4. **Flip ADR-0018 → Accepted** with the split-out consequence: PDF question resolved; turn-vs-task is a standing product ratification (eval-plan §3 already aggregates over `episode_id`).
5. **Leave ADR-0028 Proposed** untouched (H0 mode is a separate product call).
6. **Remove the PDF caveat block** from ARCHITECTURE §12 (L215) and eval-plan §3 (L60); optionally add the two confirmed-faithful rows (full-regression-timeout exemption; H3 back-edge) to §12 so the table is complete.
7. **Record ADR-0021/0022 confirmed** (no status change; they were already Accepted).

Items 2–7 are doc-only. Item 1 is the single behavioral correction and is small (one predicate + remove a contradictory sentence). Nothing here touches the wire schema, so no `schema_version`/`v` bump (data-model §9).

---

## 6. Three faithfulness cross-checks the freeze should not miss

These are not in the three-item brief but are *adjacent* to it and would be embarrassing to freeze wrong, since they live in the same traces and the same workflow:

1. **`verification_trace.method` controlled vocabulary vs the paper's verification types.** Paper (p.9): "bug reproduction; deterministic behavioral check; registered test; targeted test; full regression; lint; patch review; manual evaluator check" — **8 types**. RK (data-model §5 L201; PRD 05): `bug_reproduction, deterministic_check, registered_test, targeted_test, full_regression, lint, patch_review, manual` — **8, exact match**. ✅ Confirmed faithful; freeze as-is. (Note `manual` ↔ paper's "manual evaluator check" — same thing.)
2. **Full-regression timeout handling.** Paper (p.14 Methods): "a full-regression timeout does not by itself prevent autonomous_verified_success when deterministic requirement coverage is complete and the limitation is reported." RK (PRD 05 L354–357): the full-regression-timeout exemption is *verbatim faithful* — a timed-out `full_regression` is "covered within limits," not a failed check. ✅ This is a subtle, easy-to-get-wrong point that RK gets **exactly right**; worth recording in §12 as faithful (it currently isn't a distinct row).

Both reinforce that the *evidence-quality* (verification-autonomy) adjudication is faithful, not just the label set.

3. **The H3 workflow and its back-edge.** Paper Fig 4 (p.12) binds H3 to a five-step discipline — "reproduce → attribute → fix → verify → report" — with an explicit back-edge: "re-attribute if verification reveals diagnosis was wrong." This matters for the freeze because a verification protocol that *cannot* loop back from a failed verify to a fresh attribution would produce a premature, unfaithful report. RK realizes both: PRD 05 (L272–280) shows the `verify → re-attribute` back-edge ("the failed verification is itself an observation, producing a fresh attribution"), and ARCHITECTURE §12 L212 records that the *missing* back-edge was the one workflow gap, now fixed in PRD 05. The deterministic-check **dual role** (p.10: "at H3 they are agent-visible harness artifacts … at all levels they are evaluator-side adjudication checks") is faithfully captured by RK's two-location `checks.toml` precedence (data-model §8) + the classifier consuming check results at every level. ✅ The five-step H3 discipline and its back-edge are faithful. The episode package's `reproduction_log` (trace 6) is correctly H3-only/`Option` (data-model §5; paper p.14: "failure-attribution log is required only at H3").

---

## 7. Metric-family faithfulness (the protocol's other half)

Freezing the *traces* and *labels* is necessary but not sufficient: the paper also names a **metric family** (p.10/p.13) that the episode packages feed, and a frozen protocol must compute those metrics from the now-confirmed fields. I checked each against eval-plan §3:

| Paper metric (p.10/p.13) | eval-plan §3 definition | Reads (confirmed trace/field) | Faithful? |
|---|---|---|---|
| **AVSR** (autonomous verified success rate) | `count(outcome == autonomous_verified_success) / count(episodes)` | `outcome` (label confirmed §2 row 4a) | ✅ — depends on the now-confirmed `autonomous_verified_success` label and `episode_id` grouping |
| **M-HIR** | missing-harness interventions / episodes (grouped by `episode_id`) | `intervention_log.avoidability` (confirmed §2 row 3) | ⚠️ faithful *after* §4.A numerator fix; denominator divergence (turns vs episodes) is §3's standing item |
| **Verification autonomy** | episodes reaching a verdict without a `manual_verify` intervention and with a complete `verification_report` | `verification_trace`, `verification_report`, `intervention_log` | ✅ — this *is* the "adjudicate by verification autonomy" basis (confirmed §2 row 4b) operationalized as a population rate |
| **Context-trace meaningfulness** | fraction of `context_trace` entries with `influenced_decision = true` | `context_trace` (the restored 8th trace) | ✅ — depends on `context_trace` being present (it now is, data-model §5) |
| **Tool recovery rate** | `count(recovered == true) / count(failed tool calls)` | `tool_trace.recovered` | ✅ — `recovered` field present (data-model §5; p.9 "whether the agent recovered") |
| **Failure attribution completeness** | fraction of `Failed`/`UnsafeInvalid` with non-`f_unknown` `FailureType` + evidence + next_action | `attribution_log` (8-type enum, ADR-0021) | ✅ — depends on the confirmed 8-type taxonomy |
| **Entropy delta** | distribution of per-episode `delta` (= `-Σ severity`) | `entropy.findings.severity` (0–3, confirmed §2 row 1) | ✅ — depends on the now-confirmed 0–3 severity and 7-category space |

**Finding:** the entire metric family is computable from the confirmed fields, and **every metric except M-HIR is faithful as-specified**. M-HIR is the *only* metric touched by both open items (the §4.A numerator over-count and the §3 turn-vs-episode denominator). That concentrates the remaining faithfulness risk into one metric — which is reassuring for a freeze: fix M-HIR's numerator, ratify its denominator, and the population scorecard is paper-faithful end-to-end. The paper itself flags this metric family as the bridge to "statistically estimable quantities with confidence intervals" (p.13 Quantitative metrics), so getting M-HIR's two semantics right is the highest-value pre-freeze action.

A second, easy-to-miss point: eval-plan §3 already states entropy-delta is "reported in the paper's 7-category space via the RK 6→7 reconciliation map (ADR-0020)." That sentence is now *verified correct* — the 7-category space it maps into is exactly the confirmed list. So the eval-plan's comparability claim (RK entropy-delta comparable to paper figures) holds. **No eval-plan change needed beyond lifting the §3 PDF caveat (L60).**

---

## 8. Anti-faithfulness watch (carry-forward, not a freeze item)

Round 2 added **eval-integrity** items (anti-gaming invariant, recall provenance) that are explicitly **beyond the paper** (round2 Rec 3; round2-consolidated open-decision #2). Freezing the *paper-faithfulness* ADRs does **not** freeze those — they remain a separate, owner-gated scope expansion to ARCHITECTURE §12. I reaffirm: keep the faithfulness map (rows confirmed here) and the eval-integrity rows **visually distinct** in §12 so "faithful to the paper" is never conflated with "RK's defense-in-depth beyond the paper." The freeze in §5 covers only the former.

---

## 9. Summary

1. **All three previously-unconfirmed details and the 5-label taxonomy MATCH the paper** (entropy: 7 categories × 0–3, p.10; M-HIR = missing-harness/total episodes, p.3; intervention = avoidability + burden + harness-gap, p.10; the five labels + "adjudicate by verification autonomy not task success," p.2/p.10) — the PDF verification caveat can be **lifted** across §12, eval-plan §3, PRD 04, ADR-0019/0020.
2. **FREEZE ADR-0018, ADR-0020** (and record ADR-0021 confirmed); **FIX-THEN-FREEZE ADR-0019** after one numerator-semantics correction; **KEEP-PROPOSED ADR-0028** because H0's runtime mode is a product/cost call, never a PDF question.
3. **The one substantive correction**: RK's M-HIR numerator counts `unavoidable` interventions, which contradicts RK's own "policy-working-as-intended is *not* a missing-harness signal" and over-counts vs the paper — fix to `avoidable`-only (or reclassify `tool_block` as `benign`) before freezing ADR-0019; the `AssistedVerifiedSuccess`-on-any-intervention trigger is broader than the paper but defensible (clarify, optional).
4. **Episode unit is faithful-by-construction**: turn-grained packages with `episode_id` losslessly regroup to the paper's task-grained episode (confirmed p.8), and the metric family already aggregates over it — ratifying turn-as-stand-in is a standing product call, now cleanly *separable* from the resolved PDF question.
