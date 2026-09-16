*Point-in-time review (AI harness engineer lens), 2026-05-27. Superseded once the canonical docs (ARCHITECTURE.md faithfulness map, refined PRDs, docs/dev/eval-plan.md) absorb these edits — cite the PRDs/paper, not this file.*

# AI Harness Engineer Review — Rusty Keys

## 1. Scope & lens
Fidelity to the harness thesis and the research paper (Zhong & Zhu, *AI Harness Engineering*, arXiv 2605.13357v1). I judge whether the spec's H0–H3 ladder, M-HIR, episode package, outcome taxonomy, failure attribution, and entropy auditor match the paper's definitions; whether entropy heuristics, the M-HIR formula, and the episode schema are implementable; and whether the self-improvement loop is actually wired. **I read the PDF** — poppler/pdftotext/pypdf are all absent, but raw zlib `FlateDecode` extraction of the content streams succeeded (~87KB of text; inter-word spaces stripped but fully readable). The faithfulness assessment in §6 is grounded in the paper's own wording, not paraphrase.

## 2. Validated gaps (from the brief; one-line justification)
1. **Entropy detection is prose, not heuristics.** `04:187-194` gives one "detection method" sentence per category — no severity-threshold mapping, no diff mechanics, no false-positive guards. Not implementable as written.
2. **M-HIR formula drifts from the paper and is internally ambiguous.** `04:88` = `interventions / total_turns`; the paper defines `M-HIR = missing-harness interventions / total EPISODES`. "Episode" (paper) = one full task attempt; "turn" (RK) = one `send()`. Denominator semantics, session boundaries, and double-counting are all unpinned.
3. **Episode package schema is unversioned and field-count-inconsistent with the paper's 8 traces.** `05:255-292` shows ~10 JSON fields by example only, no `schema_version`, and silently merges/renames the paper's eight canonical traces (notably **context_trace is absent**).
4. **Verification catalog + (category, layer) matrix is incomplete vs. the paper's discipline.** `05:109-116` covers 6 rows but omits the paper's **back-edge** (verify → re-attribute), the recovery/feedback failure types, and the full-regression-timeout exemption (`05:84` hints, classifier `05:237` ignores).
5. **Self-improvement loop is half-wired.** Capture (`03:305-307`) and recall (`03:270`) exist, but the `Attribution{category,layer}` (`05:118`) is only *journaled* — it never reaches consolidation as structured input, and recall scoring does not privilege failure-born skills. Failure → skill → recall → changed behavior has a broken middle link.

## 3. Already-covered / pruned (cite file:section)
- **Outcome taxonomy enum + UnsafeInvalid trigger** — covered, `05:237-253`. Labels and snake_case wire names match the paper. Keep; only refine the *adjudication rules* (§5).
- **Deterministic-check dual role** (agent-visible@H3 / evaluator-side@all-levels) — covered and faithful, `05:230`; matches paper "Deterministic behavioral checks serve two roles."
- **`limits` always carried** — covered, `05:72-86`, ADR-013. Faithful to paper P3/R5 ("report evidence and limitations").
- **Reproduce → attribute → fix → verify → report discipline** — named, `00:208`, H3 tools `03:178-201`, checks `05:46-55`. The *spine* is present; gaps are the back-edge and the trace completeness.
- **Entropy is non-blocking/informational** — covered, `04:211-213`; consistent with paper treating entropy as a recorded audit, not a gate. Keep.
- **Tracer captures tool events / tokens / final_reached** — covered, `04:24-52`; this is the paper's `action_trace`+`tool_trace` substrate. Don't re-spec.
- **CriteriaJudge prompt + per-criterion contract** — covered, `05:148-166`. (Robustness gap is the AI-engineer's `ai-engineer.md §2.7`, not re-litigated here.)
- **PRUNED:** I do *not* propose new live metrics machinery beyond M-HIR + entropy delta for v1 — the paper's other five metrics (AVSR, verification autonomy, context-trace meaningfulness, tool-recovery rate, attribution completeness) are population-level analyses that belong in `docs/dev/eval-plan.md` (cross-ref `ai-engineer.md §5 eval-plan`), not in the hot path.

## 4. New gaps (found while reading; not in the brief)
- **N1 — Entropy categories don't map cleanly to the paper's seven.** Paper §Trace schemas + §Methods: entropy categories are **code, documentation, dependency, test, file-residue, architecture, workflow** (7, with 0–3 severity). Spec has 6 (`04:172-179`): it merges paper's "code" (redundant/dead code) into `Residue`, and renames "workflow" → `TaskContradiction`. Defensible, but it's an undocumented divergence — needs an ADR and a paper→RK category map.
- **N2 — Intervention record is missing the paper's three diagnostic fields.** Paper §Trace schemas: the intervention log records human assistance + **avoidability** + **burden level** + **the harness gap it corresponds to**. `InterventionRecord` (`04:107-110`) is only `{ts, kind, note}`. Without `avoidability`/`harness_gap`, M-HIR cannot distinguish "missing-harness" interventions (the metric's whole point) from benign ones — the metric is *named* M-HIR but counts *all* interventions.
- **N3 — The "7 intervention kinds" are a Rusty Keys invention, not the paper's.** The paper does not enumerate intervention kinds; it classifies by avoidability + harness-gap. The 7 kinds (`04:95-103`) are a reasonable concrete realization but should be an explicit ADR ("RK maps UI-observable user actions onto missing-harness signals"), not presented as if paper-derived.
- **N4 — H0 is unreachable in the spec; the ladder floor is wrong.** Paper H0 = task + repo files, **no tool registry**. `RUSTYKEYS_HARNESS_LEVEL` (`06:298`) only accepts `h1/h2/h3`; `00:202` maps H0 to "Phase 1 baseline" but Phase 1 *ships* a tool registry (= H1). So H0 cannot actually be instantiated. For the ladder to be a "controlled-visibility ablation" (paper R1), H0 must be a real selectable level (no tools) — or the spec must state H0 is an evaluation-only baseline, not a runtime mode.
- **N5 — Visibility monotonicity (paper R1) is not enforced.** Paper Table 3: each level sees *only* its artifacts; H3 inherits H2/H1. The spec gates *checks/tools* on `HARNESS_LEVEL` but never states that H1 hides AGENT_GUIDE/ARCHITECTURE/TASK_STATE or that H2 hides the deterministic-check registry. Memory/Task-State (H2 artifacts) appear unconditionally available. Without enforced controlled visibility, the maturity-claim self-assessment (`00:197-204`) can't be validated.
- **N6 — Episode = turn conflation breaks the unit of evaluation.** The paper is emphatic: "The unit of evaluation is the episode, not a single model response." RK produces one episode package *per `send()` turn* (`05:197`, `06:69`). A multi-turn task therefore yields N partial episode packages with no task-spanning rollup (no episode_id grouping, no `initial_commit`, no `final_outcome` over the whole task). This is the single deepest faithfulness divergence and must be a deliberate, documented ADR.
- **N7 — `failure_type` / `layer` are free-form strings with no controlled vocabulary.** `attribute_failure(... failure_type: String, layer: String ...)` (`03:189`) and the `Attribution` struct (`05:118`) take arbitrary strings. The paper has a *fixed* 8-member failure taxonomy (F_context, F_tool, F_feedback, F_verify, F_recovery, F_entropy, F_model, F_unknown). Free strings make attribution un-aggregatable and the (category, layer) matrix non-canonical.

## 5. Recommended edits

| target file | change | priority | depends-on |
|---|---|---|---|
| `ARCHITECTURE.md` (faithfulness map, new) | Add **paper-concept → where-realized → deliberate-divergence** table covering all 11 responsibilities, 8 traces, 5 labels, M-HIR, 7 entropy categories. Each divergence (episode=turn, 6-vs-7 entropy, 7 invented intervention kinds) links to an ADR. | **P0** | systems-architect ARCHITECTURE.md (G1) |
| `04-observe.md §EntropyAuditor` | Replace prose "detection method" with **concrete heuristics + severity thresholds** (sketch below). Add paper→RK category map; reconcile 6↔7 (N1). | **P0** | N1 ADR |
| `04-observe.md §InterventionLogger` | **Pin M-HIR** (formula sketch below): denominator = turns (document as RK divergence from paper "episodes", N6), add `avoidability`+`harness_gap` to `InterventionRecord` (N2), specify session-boundary + double-count + reset rules. | **P0** | N6 ADR, eval-plan |
| `05-compose.md §Episode package` | Define a **versioned `EpisodePackage` struct** with all 8 paper traces incl. `context_trace`; add `schema_version`, `episode_id`, `task_id`, `initial_state` (sketch below). | **P0** | systems-architect data-model (G6 versioning), N6 |
| `05-compose.md §Failure attribution` | Adopt the paper's **fixed 8-member failure taxonomy** as the canonical `FailureType` enum; make `attribute_failure.failure_type`/`layer` enums not strings (N7); add the **back-edge** (verify→re-attribute) and the **full-regression-timeout exemption** to the outcome classifier. | **P0** | N7 |
| `00-overview.md §Maturity` + `06-app.md` Config | Make **H0 a selectable level** (no tools) or declare it eval-only; state **controlled-visibility** rules (which artifacts each level hides, paper Table 3) so the ladder is a real ablation (N4, N5). | **P1** | — |
| `03-feed.md §Consolidation` + `§Recall` | **Close the self-improvement loop**: feed `Attribution{failure_type, layer}` into the consolidation prompt as structured input; boost recall score for `type=skill` memories whose stored failure-context matches the current turn. State the end-to-end chain explicitly. | **P1** | ai-engineer recall-formula edit |
| `docs/dev/eval-plan.md` (new) | Define the paper's **metric family** (AVSR, M-HIR, verification autonomy, context-trace meaningfulness, tool-recovery rate, attribution completeness, entropy delta) as population-level analyses over episode packages; H0→H3 progression gates. | **P1** | episode schema, ai-engineer eval-plan |
| `04-observe.md` / `00` ADR | Add **ADR: "RK intervention kinds map UI actions onto paper avoidability/harness-gap"** (N3); add **ADR: "RK episode = turn, not task"** (N6). | **P1** | — |
| `05-compose.md` | Record `verification_trace` method vocabulary from the paper (bug-reproduction / deterministic check / registered test / targeted test / full regression / lint / patch review / manual) — currently free-form. | **P2** | — |

### v1 sketch — entropy heuristics + severity thresholds (mark: v1 intent, revisit after a spike)
Operate on `episode.tool_events` (post-turn, synchronous, no LLM). Severity 0=info,1=minor,2=notable,3=significant. Confidence note: these are syntactic — semantic cases (StaleDocs, TaskContradiction) are best-effort until the LLM-assisted seam (`04:246`) lands.

| Category (paper map) | Heuristic | Severity |
|---|---|---|
| `Residue` (code+file-residue) | `write_file` to glob `{debug_*,tmp_*,*.bak,scratch*,*.orig,test_scratch.*}` → **2**; file written but never re-read/edited/referenced in a later `tool_event` same turn → **1**; commented-out block ≥10 lines added via edit → **1** | 1–2 |
| `TestWeakening` (test) | `edit_file` on path matching `{*_test.*,*spec*,test_*,tests/*}` where new_string removes ≥1 `assert*`/`expect(`/`#[test]` OR adds `#[ignore]`/`.skip(`/`xit(`/`@pytest.mark.skip` → **3**; net assertion-line count decrease (count `assert`/`expect` tokens old vs new) → **2** | 2–3 |
| `StaleDocs` (documentation) | `edit_file` whose new_string changes a `fn`/`def`/`function` signature line but leaves the immediately-preceding doc block (`///`,`/**`,`"""`,`#`) unchanged → **1**; doc comment deleted with no replacement → **2** | 1–2 |
| `DependencyChurn` (dependency) | within one turn, a dep added then removed in `{Cargo.toml,package.json,pyproject.toml}` edits → **2**; dep added but no source file in the turn references it (import/use scan) → **1** | 1–2 |
| `BoundaryViolation` (architecture) | `write_file`/`edit_file` to a path outside `TaskState.scope` (requires adding `scope: Vec<String>` to `TaskState`, currently missing — see systems-architect N1a) → **3**; write crossing a declared crate/layer boundary not named in the task → **2** | 2–3 |
| `TaskContradiction` (workflow) | added comment/string literal contains a negation of an active `TaskState.goal` keyword (lexical overlap + negation token) → **1** (raise to **2** only under LLM-assisted seam) | 1 |

`delta = -Σ severity`. `UnsafeInvalid` triggers (per `05:252`) on any `TestWeakening`/`BoundaryViolation` finding with severity ≥2 — consistent with the paper's unsafe_invalid definition ("tests are weakened, unrelated destructive edits occur, or the task is bypassed").

### v1 sketch — M-HIR (mark: v1 intent)
```
M-HIR(window) = count(interventions where avoidability != "benign") / denom
denom         = count(turns)   # RK unit = turn; DIVERGES from paper "episodes" — see ADR(N6)
```
- **Only missing-harness interventions count** the numerator. Add to `InterventionRecord`:
  `avoidability: Avoidable | Unavoidable | Benign`, `harness_gap: String` (which of the 11 responsibilities). `Benign` (e.g. user types a normal follow-up that the agent handled fine) is excluded — this is what makes it *M*-HIR, not raw HIR.
- **Denominator reset:** never auto-resets; `trend: Vec<f64>` (`04:129`) is rate *per session* for the sparkline; cumulative rate is all-time. State both explicitly.
- **Double-counting:** one user action → at most one intervention record. If `/task` override *and* `unverified_followup` would both fire on the same message, record the more specific (`task_override`) only; dedupe by `(ts, source_message_id)`.
- **Session boundary:** `total_turns` from `EvidenceJournal::count_turns()` (`04:134`) spans all sessions in `.rustykeys/`; per-session denominator needs a session marker in the journal (cross-ref systems-architect N5 session identity).

### v1 sketch — versioned episode package (mark: v1 intent)
```jsonc
{
  "schema_version": 1,
  "episode_id": "ep_<task_id>",            // groups turns of ONE task (fixes N6)
  "turn_id": "turn_20260527_143022_abc",
  "task_id": "...", "harness_level": "h3",
  "initial_state": {"commit": "...", "workspace": "..."},
  "ts": 1748346622.5,
  "action_trace":   [ ... ],               // read_file, edit_file, run_tool, write_report, update_task_state, declare_complete
  "tool_trace":     [ {"name","status","exit_code","duration_ms","timeout","recovered"} ],
  "context_trace":  [ {"artifact","contribution","influenced_decision": true} ],  // PAPER trace, currently MISSING
  "verification_trace": [ {"type","method","result","covers":["req-1"],"interpretation"} ],
  "attribution_log":[ {"observed","expected","failure_type":"F_verify","layer","evidence","alternatives","next_action"} ],
  "reproduction_log": {"check","observed","expected"},
  "verification_report": {"requirements":[{"requirement","met","evidence"}], "limits": "..."},
  "intervention_log":[ {"kind","avoidability","harness_gap","burden":0} ],
  "entropy": {"delta": -2, "findings":[{"category","severity","description","evidence"}]},
  "outcome": "autonomous_verified_success"
}
```
Pin serde `rename_all="snake_case"` for `EpisodeOutcome`/`FailureType`/`EntropyCategory` (overlaps systems-architect N4).

## 6. Faithfulness assessment (grounded in the extracted PDF text)

**Matches the paper (keep as-is):**
- Conceptual frame `C_system = F(C_model, C_harness, C_environment, T)` — `00:19-25` is verbatim-faithful to the paper's Eq.
- The four-verb decomposition maps onto the paper's 11 responsibilities (constrain≈permissions+task-interface; feed≈context+tools+memory+task-state; observe≈observability+intervention+entropy; compose≈failure-attribution+verification). Sound; the faithfulness map should state this mapping explicitly.
- **Outcome taxonomy** — 5 labels and their definitions (`05:237-253`) match the paper's adjudication rules nearly exactly. Faithful.
- **H3 workflow spine** (reproduce→attribute→fix→verify→report) — `00:208`, Figure 4 in the paper. Faithful except the missing back-edge.
- **Deterministic-check dual role** and **full-regression timeout discipline** — paper §Outcome adjudication / §Full regression handling; `05:230`/`05:84` are faithful in spirit.

**Drifts (must become ADRs or be corrected):**
- **M-HIR denominator: turns ≠ episodes.** Paper: per *episode* (one task attempt). RK: per *turn* (`04:88`, `06:224`). Largest metric drift. → ADR + eval-plan note.
- **Episode = turn, not task** (N6). Paper's unit of evaluation is the whole task attempt with `episode_id`, `initial_commit`, `final_outcome`. RK emits per-turn packages. → ADR; add `episode_id` grouping.
- **Intervention model: kinds vs. avoidability/harness-gap** (N2, N3). Paper classifies by avoidability + the responsibility gap; RK enumerates 7 UI-derived kinds and counts them all. → add fields + ADR.
- **Entropy: 6 categories vs. paper's 7** (N1). → category map + ADR.
- **Failure attribution: free strings vs. fixed 8-type taxonomy** (N7). The paper's F_* taxonomy is a closed set; RK's `(category, layer)` matrix is a *different* (and reasonable) decomposition but should be reconciled — ideally `category` ⊇ maps to one F_* type.
- **H0 unreachable / visibility not controlled** (N4, N5). The ladder is described but not instantiated as the paper's controlled-visibility ablation.

**Must be verified against the PDF by a human (extraction caveats):** my extraction stripped inter-word spaces and ligatures (`fi`→`\002`), so exact figure/table numbers and any equation subscripts should be re-confirmed against the rendered PDF. Specifically confirm: (a) the paper's *exact* entropy category list and that severity is 0–3 (I read "code, documentation, dependency, test, file residue, architecture, workflow … 0–3 severity" — verify the 7th is "workflow"); (b) M-HIR denominator wording ("total episodes" — verify it is not "total interventions opportunities"); (c) the intervention-log fields (avoidability/burden/harness-gap — verify "burden level" is a separate field). These three drive P0 edits, so the canonical-doc author should eyeball the PDF pages on a machine with poppler before freezing.

## 7. Cross-persona dependencies
- **AI engineer** (`ai-engineer.md`): owns recall-scoring formula and consolidation JSON contract; my "close the loop" edit (feed Attribution into consolidation, boost failure-born skills) sits *on top of* their `§5` recall/consolidation sketches — co-author. Their eval-plan and mine are the **same file** — merge: they own live dashboards + progression gates, I own the paper's metric definitions + faithfulness gates.
- **Systems architect** (`systems-architect.md`): episode-package + intervention + entropy *schemas* are their data-model SSOT (their G4/G5/G6, N4); I define the *fields/semantics*, they define DDL/versioning/serde. `TaskState.scope` (their N1a) is a prerequisite for my BoundaryViolation heuristic. Session identity (their N5) is a prerequisite for per-session M-HIR.
- **Integration engineer** (`integration-engineer.md`): their secret-redaction rule (their §2) changes what `attribution_log`/`tool_trace` may contain — confirm redaction doesn't strip evidence the attribution/verification traces depend on.
- **Product/roadmap persona**: the *thresholds* in entropy severities and the H0–H3 progression gates are product calls; I sketch the shape, they set the numbers and own the BACKLOG sequencing of the H0-instantiation and controlled-visibility work.
