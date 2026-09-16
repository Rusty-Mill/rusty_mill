*Point-in-time working document — Round 3, software-architect lens, 2026-05-27. The blocker that produced Rounds 1–2 (the research PDF was only available as a degraded zlib/`FlateDecode` text recovery, stripped of inter-word spaces and ligatures) is **RESOLVED**: the paper is now cleanly extracted to [`docs/research/2605.13357v1.txt`](../research/2605.13357v1.txt) (16 pages), and that text is the authoritative source for this review. A 13-slide NotebookLM deck + mind-map framed the read but every claim below is audited against the paper text, not the slides. **Additive only** — this doc edits nothing; it records what the clean text confirms, what it now lets us un-caveat, and what it newly exposes. Not spec.*

# Software Architect Review (Round 3) — faithfulness of the architecture to the clean paper

## 1. Scope & lens

Does Rusty Keys' **overall architecture** faithfully and completely realise the paper's *definition + eleven component responsibilities + five design principles*? I map each responsibility to its home in the constrain/feed/observe/compose (+kernel/app) layering, and each principle to where it is honoured, then flag gaps, unintended drift, deliberate-and-sound divergences, and deliberate-but-questionable ones. I do **not** re-litigate the crate DAG, concurrency model, or error taxonomy (Rounds 1–2; landed). Primary inputs: [`ARCHITECTURE.md`](../ARCHITECTURE.md) (esp. §2, §3, §12), [`prd/00-overview.md`](../prd/00-overview.md), [`prd/01-kernel.md`](../prd/01-kernel.md), and the per-verb PRDs 02–05; voice modelled on [`round2-software-architect.md`](./round2-software-architect.md).

**Headline verdict: architecturally faithful and now near-complete.** The four-verb decomposition (ADR-0005) is a sound and load-bearing realisation of the paper's "harness mediates how an agent observes, acts, receives feedback, and establishes completion" (paper p.1, p.4–5). All eleven responsibilities have a clear home; all five principles are honoured somewhere concrete. The clean text **vindicates** the divergence ADRs (0018–0020) — their premises are now confirmed verbatim, not guessed — which means the single biggest blocker to "freezing" the faithfulness work is gone. The residual issues are (a) **ARCHITECTURE §12's PDF caveat is now obsolete and should be retired**, (b) one **questionable mapping** (task-interface → constrain), and (c) two **structural divergences** (episode=turn; entropy 6≠7) that are sound but still carry `Proposed` ADRs.

## 2. The definition — faithfully realised

Paper definition (p.4): *"a runtime substrate surrounding a foundation-model software agent that manages context, tools, project memory, task state, observability, failure attribution, verification, permissions, and maintenance state, so that latent model coding capability becomes auditable software-engineering behaviour."* The four implications (p.4–5) are each honoured:

- **External to the model** — the kernel (PRD 01) is deliberately thin and "knows nothing about memory, policy, verification, or the UI"; the harness is the *other* crates. Faithful (ADR-0005).
- **Task-runtime infrastructure governing observe/act/feedback/complete** — `Session::send()` owns the cycle observe → orient → kernel → compose (ARCHITECTURE §6); the four verbs map cleanly onto OODA. Faithful.
- **Evaluable (exposed/hidden/ablated/traced/compared)** — the H0–H3 ladder (ARCHITECTURE §3) is the ablation instrument; partially realised (see §6 drift below).
- **Produces evidence** — the episode package + 8 traces (PRD 05) are the evidentiary output. Faithful.

`C_system = F(C_model, C_harness, C_environment, T)` is reproduced verbatim (ARCHITECTURE §2; PRD 00) and matches paper p.2. The Ashby's-Law framing (ARCHITECTURE §2, PRD 00) is RK's own justification layered *on top of* the paper — sound, not drift, but it is RK-original and not a paper claim (the paper never invokes requisite variety); worth keeping labelled as such.

## 3. The eleven responsibilities → their home in the layering

Paper Table 1 (p.6) names eleven. Mapping each to RK's home and judging fit:

| # | Paper responsibility (Table 1, p.6) | RK home | Fit |
|---|---|---|---|
| 1 | **Task interface** (present objective, requirements, constraints, success criteria) | ARCHITECTURE §12 says **constrain**; realised in **feed** (`TaskState`/`set_task`, `TaskStore::render` into `extra_context`, PRD 03) + **compose** (`CriteriaJudge` over `success_criteria`, PRD 05) | ⚠️ **questionable** — see §7.A |
| 2 | **Context manager** (select/expose task-relevant content) | **feed** — `orient()`, `recall()`, context assembly, the context trace (PRD 03; trace in PRD 05 `ContextEntry`) | ✅ |
| 3 | **Tool registry** (declare available tools + allowed commands) | **feed** — `ToolRegistry`, `ToolFn`, `#[tool]` suite (PRD 03); allowed-commands gate is **constrain** | ✅ |
| 4 | **Project memory** (architecture/testing/known-failure knowledge) | **feed** — `Stream`/`Store`, consolidation, recall (PRD 03); H2 artifacts (`AGENT_GUIDE`, `ARCHITECTURE`, `TESTING`, `KNOWN_FAILURES`) per visibility matrix | ✅ |
| 5 | **Task state** (hypothesis, inspected files, open questions, next steps) | **feed** — `TaskState { goal, success_criteria, scope, status }` (PRD 03) | ⚠️ **narrowed** — see §7.D |
| 6 | **Observability layer** (logs, traces, outputs, runtime errors) | **observe** — `Tracer`, `Episode`, `ToolEvent` (PRD 04) | ✅ |
| 7 | **Failure attribution** (separate observation/expected/diagnosis) | **compose** — `Attribution`, `attribute_failure`, frozen `(category,layer)→FailureType` matrix (PRD 05) | ✅ |
| 8 | **Verification protocol** (map requirements to deterministic evidence) | **compose** — `Verifier`, `Check`, `CriteriaJudge`, deterministic `CheckRegistry` (PRD 05) | ✅ |
| 9 | **Permission boundary** (restrict risky actions; approval gates) | **constrain** — `Policy`, `WorkspacePolicy`, `PermissionMode`, `BashGuard`, `ApprovalGate` (PRD 02) | ✅ |
| 10 | **Entropy auditor** (detect agent-introduced maintenance burden) | **observe** — `EntropyAuditor`, `EntropyCategory`, severity 0–3 (PRD 04) | ⚠️ **6≠7** — see §7.B |
| 11 | **Intervention logger** (record human assistance + avoidability) | **observe** — `InterventionLogger`, M-HIR (PRD 04) | ⚠️ **denominator** — see §7.C |

**No responsibility is homeless.** Eight are clean; the four `⚠️` rows are diagnosed in §7. ARCHITECTURE §12's one-line gloss — *"constrain ≈ permissions+task-interface; feed ≈ context+tools+memory+task-state; observe ≈ observability+intervention+entropy; compose ≈ attribution+verification"* — is accurate for ten of eleven (the task-interface assignment to constrain is the exception).

## 4. The five design principles → where honoured

Paper p.5 enumerates P1–P5. Each maps to a concrete RK mechanism:

- **P1 Explicit runtime resources** (resources named, not implicit) — honoured pervasively: tools (`ToolRegistry`), context/memory (`orient`, `Store`), verification evidence (`VerificationReport`), human attention (`InterventionLogger`), permission boundary (`PermissionMode`/`WorkspacePolicy`), maintenance state (`EntropyAuditor`). This is exactly the four-verb thesis. ✅
- **P2 Traceable mediation** (record how context is selected, tools invoked, verification attempted, failure recovered, intervention incurred) — honoured by the 8-trace episode package (PRD 05) + the `on_event`/`Tracer` hook (PRD 01/04). The `context_trace` (which records *whether memory influenced a decision*, PRD 05 `ContextEntry.influenced_decision`) is the precise realisation of the paper's "memory is auditable only when its use is traced" (p.12). ✅
- **P3 Requirement-level verification** (completion bound to evidence, not assertion) — honoured by the H3 `CheckRegistry` + `VerificationReportRequired` check + `H3_LIMITS` (PRD 05); deterministic checks map requirements → observable output (paper p.10, Methods p.14). ✅ This is RK's strongest faithfulness point.
- **P4 Attribution before recovery** (classified diagnosis before the next edit) — honoured by the `ReproduceBeforeEdit` check (`edit_file` without a prior `attribute_failure` fails, PRD 05) and the reproduce→attribute→fix→verify→report loop **with the verify→re-attribute back-edge** (PRD 05). The back-edge directly realises paper Figure 4's "re-attribute if verification reveals diagnosis was wrong" (p.12). ✅
- **P5 Maintenance & entropy awareness** (record burden rather than treat it as outside the loop) — honoured by `EntropyAuditor` running in the post-turn `tokio::join!` (PRD 04) and feeding the `UnsafeInvalid` outcome trigger (PRD 05). ✅

**All five principles have a clear, named home.** None is aspirational-only; each is wired into a check, trace, or policy.

## 5. The evaluation machinery — eight traces, five outcomes, eight Ftypes

Beyond the eleven responsibilities, the paper's other two contributions (p.2 (iii)–(iv)) are the trace-based protocol and its taxonomies. The architecture must give each an evidentiary home, and it does.

**Eight trace types (paper Table 4, p.9) → `EpisodePackage` (PRD 05, data-model §5).** RK's `EpisodePackage` carries all eight as named fields — `action_trace, tool_trace, context_trace, verification_trace, attribution_log, reproduction_log, verification_report, intervention_log` — plus `entropy` and `outcome`. This is a *structural* match to Figure 3 (p.8). The one historically-missing trace, `context_trace`, is now present (PRD 05 `ContextEntry { artifact, contribution, influenced_decision }`), which is exactly the paper's "which project-memory artifact was consulted, what it contributed, and whether it influenced a decision" (Methods, p.14; Implications, p.12). The trace schemas are line-structured per the paper's JSONL requirement (p.9, Methods p.14) — RK's append-only `.jsonl` logs honour this. ✅ Faithful and complete.

- One subtle point: the paper says the **failure-attribution log is required only at H3** (Methods, p.14). RK matches this — `attribute_failure` and `ReproduceBeforeEdit`/`VerificationReportRequired` checks are H3-gated (PRD 05) — so the trace's *level-conditionality* is preserved, not just its presence.
- The paper's `tool_trace` fields (command, exit code, duration, timeout status, failure type, recovery, p.9) map onto RK's `ToolEvent` + `ToolOutcome.status` (PRD 04, ADR-0022). RK reads status *structurally* rather than sniffing prefixes — an implementation virtue that makes the trace machine-aggregatable, beyond what the paper specifies. ✅

**Five-label outcome taxonomy (paper p.10) → `EpisodeOutcome` (PRD 05).** `AutonomousVerifiedSuccess / AssistedVerifiedSuccess / UnverifiedSuccess / Failed / UnsafeInvalid` is a verbatim match. The architecture honours the paper's defining property — *"separates task behaviour from evidence quality: a patch can be correct but unverified"* (p.10) — through two load-bearing gates: (i) `judge_ran = false` bars `AutonomousVerifiedSuccess` (a `judge_unavailable` turn degrades to `UnverifiedSuccess`, never silently passes — PRD 05, ARCHITECTURE §10); (ii) any entropy `severity ≥ 2` on `TestWeakening`/`BoundaryViolation` forces `UnsafeInvalid` (PRD 04/05), realising the paper's `unsafe_invalid` definition exactly. The clean text confirms the Methods adjudication rules (p.15) and RK's classifier rules match them rule-for-rule, *including* the full-regression-timeout exemption (Methods "Full regression handling", p.15 ↔ PRD 05 timeout exemption). ✅ This is the most precisely-faithful corner of the whole system.

**Eight failure types (paper p.4) → `FailureType` (PRD 05, ADR-0021).** `FContext, FTool, FFeedback, FVerify, FRecovery, FEntropy, FModel, FUnknown` is a verbatim match to the paper's taxonomy. The architecture binds them through a *frozen `(category, layer) → FailureType` matrix* (PRD 05) so attribution is aggregatable rather than free-text — a discipline the paper implies ("the taxonomy enables a question pass/fail cannot answer," p.4) but does not itself enforce. The matrix reserving `f_context`/`f_feedback` for H3-`attribute_failure`-raised attributions (no deterministic check emits them) is a sound architectural reading of the paper's "agent records observed/expected/inferred failure type" (p.11–12). ✅

**Resource-management lens (paper Table 2, p.7).** The paper offers ten managed resources (context budget, tool budget, verification evidence, project memory, task state, human attention, permission boundary, failure signal, entropy budget, test-time compute) "strictly as a resource-management lens" (p.5), explicitly *not* an OS. RK's four verbs cover all ten, but **test-time compute** is the thinnest: it surfaces only as `max_steps`, per-call timeouts, and the post-turn budget (PRD 01, ARCHITECTURE §11), with no first-class "compute spent on verification vs exploration" accounting (paper Table 2: "Runaway commands; expensive loops"). Not a responsibility gap (it is not one of the eleven), but if RK wants the *resource* framing complete, test-time-compute accounting is the one resource without a named home. Minor; backlog-worthy, not a faithfulness defect.

## 6. Drift the clean text confirms (ladder fidelity)

The one place the architecture **under-delivers** against the paper is the H0–H3 ladder's *controlled-visibility* property. The paper's R1 (p.7) is explicit: *"each level exposes only the artifacts assigned to that level; lower levels do not see higher-level artifacts,"* and the Table 3 visibility matrix (p.7) is "the operational definition of the ladder." ARCHITECTURE §3 candidly admits RK "today the level gates tools/checks but does not yet hide H2 memory from H1," and H0 is unreachable at runtime (only `h1/h2/h3` accepted). This is **acknowledged, not hidden** drift — tracked by ADR-0028 — but it means RK currently implements the ladder as *additive capability*, not as the paper's *monotonic-visibility ablation*. For the architecture to be a faithful *ablation instrument* (a core paper contribution, p.2 (ii)), monotonic hiding must eventually be enforced, not just the H3 check-set toggled by `RUSTYKEYS_HARNESS_LEVEL`. Sound to defer; important not to lose.

## 7. Divergences — sound vs. questionable

**A. Task-interface → constrain (QUESTIONABLE mapping).** ARCHITECTURE §12 assigns the task-interface responsibility to **constrain**. The paper (Table 1, p.6; Figure 1, p.4) describes the task interface as *"present objective, requirements, constraints, success criteria"* and places it at the *top* of the harness column next to context manager and tool registry — it is an **input/feed-shaped** responsibility, not a permission-shaped one. In RK, the actual realisation lives in **feed** (`TaskState`, `set_task`, render into `extra_context`) and **compose** (`CriteriaJudge` over `success_criteria`) — *not* in constrain at all. So the §12 gloss is **internally inconsistent with where the code actually puts it**. Recommendation (additive, for a future edit): re-attribute task-interface to *feed (+compose)* in §12; constrain owns *permission boundary* (#9) and that is its clean single mapping. This is the one genuine mis-statement in the faithfulness map. (Note: the paper itself is loose here — the abstract/intro (p.1–2) call this responsibility "task specification" while Table 1 calls it "task interface"; RK should pick Table 1's name since that is the normative enumeration.)

**B. Entropy 6 vs 7 (SOUND divergence — now fully verifiable).** The clean text settles what was a caveat. Paper p.10 lists the entropy categories verbatim: *"code, documentation, dependency, test, file residue, architecture, workflow"* — **seven**, each with a **0–3 severity** ("together with a 0–3 severity"). RK has six (`Residue, TestWeakening, StaleDocs, DependencyChurn, BoundaryViolation, TaskContradiction`) and merges paper-*code* + paper-*file-residue* into `Residue`, renames *workflow*→`TaskContradiction` (PRD 04 map; ADR-0020). The merge is defensible (dead code and debris are one detector in practice) and the map is lossless-for-comparison. **The architecture is sound here; the ADR's "pending human PDF confirmation" rationale is now satisfied by the clean text** and ADR-0020 can move Proposed→Accepted.

**C. M-HIR denominator = turns, not episodes (SOUND divergence — confirmed).** Paper p.4 is unambiguous: `M-HIR = missing-harness interventions / total episodes`. RK uses `denom = count(turns)` (PRD 04), recovering task-level M-HIR by aggregating over `episode_id`. The clean text confirms the paper's wording exactly, so this is a *known, correct-by-aggregation* divergence (ADR-0018), not a misreading. Also confirmed: the paper's intervention record fields — *"human assistance, its avoidability, its burden level, and the harness gap"* (p.10) — match RK's `{avoidability, harness_gap, burden}` (PRD 04, ADR-0019) **exactly**, so ADR-0019's premise is verified and it too can move Proposed→Accepted.

**D. Task state narrowed (SOUND but worth a note).** Paper Table 1 (p.6) scopes task state as *"hypothesis, inspected files, open questions, next steps."* RK's `TaskState` is `{goal, success_criteria, scope, status}` — it captures the *goal/criteria* facet richly but does **not** persist hypothesis / inspected-files / open-questions / next-steps as first-class fields; those live implicitly in the short-term `Stream` and the action trace. This is a reasonable architectural choice (the stream + episode package already hold inspected-files and next-steps as traces), but it is a *narrower* TaskState than the paper's, and unlike B/C it is **not** captured by any ADR. Minor completeness gap — flag for the harness-engineer/AI-engineer lenses; no architectural change needed if the stream/trace genuinely covers it, but it should be stated.

**E. Episode = turn (SOUND, structural).** Already covered by C/ADR-0018; the architectural consequence is that RK's evidence unit is the `send()` turn and the paper's is the task. The `episode_id` grouping (PRD 05 `EpisodePackage`) is the right bridge. Sound.

## 8. Is ARCHITECTURE §12 now accurate against the real text?

**Mostly yes, with two required updates and one obsolete block.**

1. **Retire the PDF caveat (obsolete).** §12's closing "PDF verification caveat" says a human must still confirm (a) the 7 entropy categories, (b) the M-HIR "total episodes" wording, (c) the intervention-log fields against a rendered PDF. **All three are now directly confirmed in [`2605.13357v1.txt`](../research/2605.13357v1.txt)** (p.10, p.4, p.10 respectively). The caveat block is stale and should be removed/replaced with a citation to the clean text; keeping it understates RK's actual fidelity and blocks the Proposed→Accepted moves on ADR-0018/0019/0020.
2. **Fix the task-interface row** (§7.A): re-attribute to feed(+compose), not constrain.
3. **Everything else in §12 audits clean** against the text: the `C_system` row (verbatim, p.2), the 5-label taxonomy (p.10), the 8-trace package incl. the now-added `context_trace` (p.8, Table 4), the reproduce→…→report loop *with* back-edge (Figure 4, p.12), and the deterministic-check dual role / limits-always-carried (p.10, Methods p.14–15) are all faithful.

## 9. Summary of recommended (additive) follow-ups

- **Retire ARCHITECTURE §12's PDF caveat**; cite `docs/research/2605.13357v1.txt` instead. Unblocks the next item.
- **Move ADR-0018, ADR-0019, ADR-0020 Proposed→Accepted** — their premises (M-HIR denominator wording, intervention fields, 7 entropy categories + 0–3 severity) are now confirmed verbatim against the clean text; the only thing keeping them Proposed was the PDF blocker that is gone. (ADR-0028 stays Proposed — it is a genuine open *product* decision, not a text-confirmation one.)
- **Re-attribute the task-interface responsibility** in §12 from constrain → feed(+compose); use paper Table 1's name "task interface."
- **Note the TaskState narrowing** (§7.D) somewhere — either a new short ADR or a line in PRD 03 — since hypothesis/inspected-files/open-questions/next-steps are paper-named TaskState facets RK covers only via the stream/trace.
- **Keep monotonic-visibility enforcement on the roadmap** (§6; ADR-0028) so the ladder is a true ablation instrument, not just additive capability.

## 10. Cross-persona handoffs

- **Harness engineer:** owns the *content* of B/C/D — confirm the entropy 6→7 map and the TaskState-narrowing coverage (does the action trace truly capture inspected-files/next-steps?) are adequate, and ratify the M-HIR aggregation story.
- **Systems architect:** the §12 edits and the ADR status flips are doc-state changes in their lane; the task-interface re-attribution touches no crate boundary (feed already owns the code).
- **AI engineer:** TaskState facet coverage (§7.D) intersects memory/stream semantics — theirs to confirm that hypothesis/open-questions live somewhere recallable.
- **Product/roadmap:** ADR-0028 (H0 selectability + monotonic visibility) remains the one open *product* call, not a faithfulness one.
