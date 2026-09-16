*Point-in-time working document — Round 3 of the multi-persona review, **AI/memory-engineer** lens. Audits Rusty Keys' fidelity to *AI Harness Engineering* (Zhong & Zhu, arXiv 2605.13357v1) for the five model-/memory-facing responsibilities I own: **project memory, task state, context selection, entropy auditing, failure attribution**. Unlike Round 2 (which assessed external sources), Round 3 reads the paper as the authoritative source — now cleanly extracted to [`docs/research/2605.13357v1.txt`](../research/2605.13357v1.txt) (16 pp). Page citations below are to that file. Additive only — no canonical doc is edited here. Supersedes the "pending PDF confirmation" caveats in [`round2-ai-engineer.md`](./round2-ai-engineer.md) for the items the rendered text now settles.*

# Round 3 — AI / memory engineer review

## 0. Three-line summary

1. **Both previously-unconfirmed taxonomies now match the paper EXACTLY**: entropy = 7 categories (code, documentation, dependency, test, file residue, architecture, workflow) × 0–3 severity (p.10), and failure = 8 types Fcontext…Funknown (p.3). RK's `FailureType` is 1:1; RK's 6-category entropy enum is a *sound, documented* 6→7 reconciliation (ADR-0020).
2. **ADR-0019 and ADR-0020 can be frozen Proposed→Accepted** — their only open condition was "confirm against the rendered PDF," which the clean extraction now satisfies. (ADR-0018 stays Proposed: its open condition is product ratification, not PDF confirmation.)
3. **One real gap**: "context selection" is a named H2 *level* and has a `context_trace`, but RK has no explicit **context-selection protocol** artifact the way the paper does (p.6, Table 3) — recall scoring serves it implicitly. The memory **learning loop** is a deliberate, well-marked extension *beyond* the paper and should stay flagged as such.

---

## 1. Scope & method

My five responsibilities, mapped to where RK realizes them:

| Paper responsibility (p.4 Table 1) | RK home | Doc |
|---|---|---|
| Project memory | `Store` (long-term graph) + consolidation + recall | [PRD 03](../prd/03-feed.md) §Memory |
| Task state | `TaskState` / `task.json` | [PRD 03](../prd/03-feed.md) §Task State; [data-model §8](../architecture/data-model.md) |
| Context selection | recall scoring + `context_trace` | [PRD 03](../prd/03-feed.md) §Recall; [data-model §5](../architecture/data-model.md) |
| Entropy auditing | `EntropyAuditor` | [PRD 04](../prd/04-observe.md) §EntropyAuditor; [ADR-0020](../adr/0020-entropy-categories-six-reconciled-to-seven.md) |
| Failure attribution | `Attribution` + `attribute_failure` | [PRD 05](../prd/05-compose.md) §Failure attribution; [ADR-0021](../adr/0021-fixed-failuretype-taxonomy.md) |

Method: read the paper text, extract the exact taxonomies, then diff RK against them line by line. Where a Round 2 / ARCHITECTURE §12 note said a detail was "pending PDF confirmation," I treat the clean extraction as the confirmation and say so explicitly.

---

## 2. Confirmed paper facts (the audit's ground truth)

These are the three facts the brief flagged as "verify, then audit," quoted from the extraction:

- **Entropy audit = 7 categories × 0–3 severity.** p.10 (Trace schemas): *"The entropy audit records categories of agent-introduced maintenance burden—code, documentation, dependency, test, file residue, architecture, workflow—together with a 0–3 severity."* The five design principles (P5, p.5) and Table 1's "Entropy auditor" row corroborate the *kinds* of burden (stale docs, dependency churn, residue, test weakening, boundary violations). **CONFIRMED — this was the previously-unconfirmed detail.**
- **Failure taxonomy = 8 types.** p.3 (Failure taxonomy): *"We use eight: Fcontext … Ftool … Ffeedback … Fverify … Frecovery … Fentropy … Fmodel … and Funknown."* **CONFIRMED.**
- **Context selection is a first-class responsibility.** p.4 Table 1 lists "Context manager" (contract: *"Select and expose task-relevant project content"*); p.6 H2 adds a "context-selection protocol"; Table 3 (p.7) lists "Context-selection protocol" as an H2-visible artifact; p.9 Table 4 ties the `context trace` to Fcontext. **CONFIRMED as an owned responsibility with its own protocol artifact.**

---

## 3. Question 1 — Do RK's entropy categories EXACTLY match the paper's 7 (+0–3 severity)?

**Verdict: the paper's 7 are confirmed; RK's 6→7 map is exact and sound. No mismatch.**

RK ships **6** variants ([PRD 04](../prd/04-observe.md) §EntropyAuditor): `Residue`, `TestWeakening`, `StaleDocs`, `DependencyChurn`, `BoundaryViolation`, `TaskContradiction`. The paper has **7**. The reconciliation map (PRD 04 "Paper → RK category map", ADR-0020) is:

| Paper (p.10) | RK | Assessment vs. paper text |
|---|---|---|
| code | `Residue` | ✅ sound merge — see below |
| file residue | `Residue` | ✅ sound merge |
| test | `TestWeakening` | ✅ 1:1 |
| documentation | `StaleDocs` | ✅ 1:1 |
| dependency | `DependencyChurn` | ✅ 1:1 |
| architecture | `BoundaryViolation` | ✅ 1:1 |
| workflow | `TaskContradiction` | ✅ rename only |

Findings:

- **The map is now fully validated.** PRD 04 line ~295 and ADR-0020 both carry a caveat that "the exact seven paper categories and the 0–3 severity scale are pending human confirmation against the rendered PDF." The clean extraction (p.10) confirms **both** the 7 category names *and* the 0–3 severity. **That caveat is now dischargeable.** RK's `severity: u8 // 0–3` (PRD 04 `EntropyFinding`) matches verbatim.
- **The code+file-residue merge is a legitimate, paper-aware divergence, not drift.** The paper deliberately lists *code* (redundant/dead code) and *file residue* (debug scripts, temp files) as **two** categories; RK folds both into one `Residue` variant. PRD 04's heuristic table even keeps them distinguishable in practice (commented-out blocks → code-ish; `*.bak`/`tmp_*` globs → file-ish), and the map translates back to 7 for any paper comparison. This loses no information for cross-paper metric comparison (entropy-delta is `-Σ severity`, category-agnostic) — it only collapses the *labeling* granularity. Acceptable and documented.
- **One naming-precision nit (cosmetic, not a fidelity break).** The paper writes "file **residue**" (two words) and the `entropy.jsonl` example in [data-model §4.4](../architecture/data-model.md) uses the RK wire name `test_weakening` — which is RK's own snake_case enum, *not* the paper's category. That is correct (RK persists RK enums, ADR-0025), but the round of "is this the paper's vocabulary?" confusion is avoidable: the data-model example would read more clearly if it carried a one-line `// RK category; paper category = "test"` pointer like PRD 04's map does. Minor.

**Action**: freeze ADR-0020 (see §7).

---

## 4. Question 2 — Do RK's failure types EXACTLY match the paper's 8?

**Verdict: EXACT match. No mismatch.**

Paper (p.3): Fcontext, Ftool, Ffeedback, Fverify, Frecovery, Fentropy, Fmodel, Funknown.
RK `FailureType` ([PRD 05](../prd/05-compose.md), [data-model §5](../architecture/data-model.md), ADR-0021): `FContext`, `FTool`, `FFeedback`, `FVerify`, `FRecovery`, `FEntropy`, `FModel`, `FUnknown` → wire `f_context … f_unknown`.

All eight present, same order, same semantics; ADR-0021 is already **Accepted**. Two observations worth recording:

- **RK's per-failure-type *meaning* tracks the paper closely.** Compare RK's enum doc-comments (PRD 05 lines ~126–134) to the paper's gloss (p.3): e.g. paper Ffeedback = *"feedback is unavailable or not interpretable"* vs RK `FFeedback // tool result/observation not surfaced or misread`. Faithful.
- **The frozen `(category, layer) → FailureType` matrix is an RK addition that *strengthens* fidelity.** The paper names the 8 types but does not prescribe how a deterministic check maps to one; RK's matrix (PRD 05 §"Frozen (category, layer) → FailureType matrix") is the operationalization that makes attribution *aggregatable* — exactly what the paper's metric family (p.10, "failure attribution completeness") needs. Note the matrix only auto-emits 4 of the 8 types from deterministic checks; `f_context` / `f_feedback` are reserved for the H3 `attribute_failure` tool, and `f_unknown` is the fallback. That is internally consistent and matches the paper's framing that Fcontext/Ffeedback are diagnosed during reproduction, not by a pass/fail check.

**No drift.** This is the cleanest of the five responsibilities.

---

## 5. Question 3 — Is "context selection" a real, owned responsibility (recall/scoring) or implicit?

**Verdict: half-owned. The *mechanism* exists and is traced; the *named protocol artifact* the paper specifies does not. This is the one genuine gap in my lens.**

What RK has:
- **Recall as the selection mechanism** (PRD 03 §Recall): a scoring formula (`0.55·rel_norm + 0.25·recency + 0.20·importance`), top-k + 1-hop neighbor expansion, a token-capped output block. This is a real, owned, *scored* context-selection engine — arguably richer than anything the paper prescribes.
- **The `context_trace`** ([data-model §5](../architecture/data-model.md), PRD 05 `ContextEntry { artifact, contribution, influenced_decision }`): the paper's exact Fcontext-diagnosing trace (p.9 Table 4), and the eval plan's "context-trace meaningfulness" metric reads it. This was explicitly *added* after it was found missing (ARCHITECTURE §12) — good.
- **"Context selection" named at H2** in ARCHITECTURE §3, the glossary, and the eval-plan H2 gate ("cross-session recall surfaces the planted fact").

What RK is **missing** vs the paper:
- **An explicit "context-selection protocol" artifact.** The paper's Table 3 (p.7) lists "Context-selection protocol" as a *named, agent-visible H2 artifact* — a sibling of `AGENT_GUIDE`, `TESTING guide`, `TASK_STATE`, `KNOWN_FAILURES`. In RK there is no agent-facing document/registry telling the agent *how to select context*; the selection is done *for* the agent by `recall()` and surfaced as the "## Relevant memory" block, with only a passing H2 system-prompt line ("trust recalled lessons"). So RK realizes the *outcome* of context selection (relevant content in front of the model, traced) but not the paper's *protocol-as-artifact* shape. For a strict H2 visibility-matrix audit (Table 3), RK would be "context-selection: mechanism present, protocol artifact absent."
- **Knock-on for the ladder**: ARCHITECTURE §12's H2 row doesn't call this out. The faithfulness map flags H0 reachability and monotonic visibility (ADR-0028) but is silent on the missing context-selection *protocol* artifact. Worth a row.

**Recommendation (additive, for a future doc edit by the owner — not applied here):** either (a) add a short `harness/context-selection.md` (or a system-prompt sub-section) that states RK's selection contract to the agent — "recall gives you scored memory; verify it against the task before relying on it; prefer primary-contribution artifacts" — so H2 has the paper's named artifact; or (b) explicitly record in ARCHITECTURE §12 that RK substitutes an *automatic, scored, traced* recall for the paper's *agent-followed protocol*, as a deliberate divergence (the harness selects rather than instructs). (b) is cheaper and is arguably the more honest description of what RK does. Either way, the gap should be *named*, not left implicit.

---

## 6. Question 4 — Does the memory/consolidation design add anything BEYOND the paper, and is the extension clearly marked?

**Verdict: yes, RK adds a substantial learning-memory system beyond the paper; it is *mostly* well-marked as deliberate, but the divergence is not captured in the one place a reader looks for divergences (ARCHITECTURE §12).**

The paper's "project memory" (p.4 Table 1; p.6 H2; p.12 "Memory is auditable only when its use is traced") is **static, agent-readable knowledge**: AGENT_GUIDE, ARCHITECTURE, TESTING guide, KNOWN_FAILURES — authored once, *consulted* by the agent, with the context trace recording *which artifact was consulted and whether it influenced a decision*. The paper's contribution on memory is precisely **traceability of consultation**, not *generation* of memory.

RK goes materially further. Its memory is a **living, self-writing graph**:
- **Three-tier cognitive architecture** (short-term stream → consolidation → long-term graph + Task State), ADR-0008.
- **Tiered consolidation** (idle/sleep/explicit), ADR-0009 — an *aisdk call* that distills observations into facts/summaries/skills/entities with typed edges.
- **The self-improvement loop** (PRD 03 §Self-improvement loop): FAILURE → structured `Attribution` → `skill` (importance ≥0.8) → recall `+0.15` boost on matching `(failure_type, layer)` → BEHAVIOR. The agent *learns from its own failures across turns/sessions*.
- **Skill lifecycle**: floor-at-0.6, prune-exemption (ADR-0011), grooming (refine/merge/split).

This is well beyond the paper. Assessment:

- **It is a sound extension, and it does not *contradict* the paper** — it *consumes* the paper's primitives correctly. The loop's middle link is the paper's own `Attribution { failure_type, layer }` (the 8-type taxonomy of §4) fed into consolidation; the loop's trace surface is the paper's `context_trace`. RK is extending the paper's vocabulary, not bending it. The paper even gestures at this in "Outlook → Long-horizon evaluation" (p.13): *"project memory either ages well or rots"* — RK's grooming/decay is a direct answer to that open question. So the extension is *in the spirit of* the paper's research program.
- **It is marked as deliberate in the right local places.** PRD 04 says outright that the `EntropyAuditor` has "No equivalent exists in Claude Code or hermes-agent. This is a genuine capability improvement." PRD 03 frames the loop as "the feed and compose layers together close the paper's learning loop." The Round 2 consolidated rec 2 (validation-gated skills) is precisely the right next refinement (don't let the loop *un*-learn) and is already queued.
- **But the one canonical divergence register — ARCHITECTURE §12 — does not list it.** §12's "Project memory" line is absent entirely; the table maps the 11 responsibilities collectively to the four verbs and then enumerates divergences for episode/M-HIR/attribution/entropy — but **not** for "RK's project memory is a *generative, self-improving* system vs the paper's *static, consulted* knowledge." A reader auditing fidelity would not learn from §12 that RK's memory is a deliberate superset. **This is the gap to close**: §12 deserves one row — *"Project memory: paper = static agent-readable artifacts (AGENT_GUIDE/…); RK = generative 3-tier consolidating graph + failure-born skill loop → deliberate extension (ADR-0008/0009/0011), not a divergence-from but a superset-of."* Marking it makes the extension auditable instead of surprising.

Net: the extension is **good engineering and faithful in spirit**, but its status as a deliberate superset should be promoted from scattered prose into the faithfulness map.

---

## 7. Can any "Proposed" ADR now be frozen to Accepted?

The ADR README (`Status` is `Proposed` | `Accepted`) lists four Proposed ADRs: 0018, 0019, 0020, 0028. My lens covers the faithfulness-by-PDF-confirmation ones:

| ADR | Open condition (as written) | Now resolvable? |
|---|---|---|
| **0020** (entropy 6→7) | "Proposed pending human confirmation of the exact seven paper categories and the 0–3 severity thresholds against the rendered PDF" | **YES → freeze Accepted.** p.10 confirms all 7 categories *and* 0–3 severity verbatim (§2/§3 above). The only gate was PDF confirmation; it is met. |
| **0019** (intervention → avoidability/harness_gap/burden) | "Proposed pending human confirmation of the exact paper field names (avoidability / harness_gap / burden) against the rendered PDF" | **YES → freeze Accepted.** p.9 (Trace schemas): the intervention log records *"human assistance, its avoidability, its burden level, and the harness gap it corresponds to."* All three field names confirmed. |
| 0018 (episode = turn) | "Proposed: the **owner must ratify** whether turn-grained episodes … are an acceptable stand-in" | **NO — leave Proposed.** Its gate is *product ratification of a divergence*, not PDF confirmation. The paper (p.8) does define the episode as the task-level unit, which RK knowingly diverges from; that's a decision for the owner, not a fact-check. |
| 0028 (H0 selectable vs eval-only) | product decision | NO — outside my lens (eval/maturity); also a product call. |

**Concrete recommendation:** flip **ADR-0019** and **ADR-0020** to `Accepted`, update their lines in [`docs/adr/README.md`](../adr/README.md), and discharge the matching "PDF verification caveat" clauses in [ARCHITECTURE §12](../ARCHITECTURE.md) and [eval-plan §3](../dev/eval-plan.md) (the caveat block can drop entropy-categories and intervention-fields, leaving only the M-HIR-denominator wording — which is itself now confirmed too: p.4 defines M-HIR with denominator *"total episodes"*, so even that sub-clause of the caveat is dischargeable, though the turn-vs-episode *divergence* it documents remains ADR-0018's product call). I am **not** applying these edits (additive-only round); they are for the owner.

---

## 8. Drift / gaps / nits ledger (my lens only)

| # | Item | Severity | Where | Action |
|---|---|---|---|---|
| D1 | Entropy 7 categories + 0–3 severity confirmed against paper (p.10) | — (resolves a caveat) | PRD 04, ADR-0020, ARCH §12 | Freeze ADR-0020; drop caveat clause |
| D2 | Intervention fields avoidability/harness_gap/burden confirmed (p.9) | — (resolves a caveat) | PRD 04, ADR-0019 | Freeze ADR-0019; drop caveat clause |
| D3 | M-HIR denominator "total episodes" confirmed (p.4) | low | eval-plan §3, ARCH §12 | Caveat clause dischargeable; turn-vs-episode divergence stays ADR-0018 |
| G1 | **No explicit context-selection *protocol* artifact** (paper Table 3 lists one at H2) | **medium** | PRD 03, ARCH §3/§12, glossary | Add the artifact, *or* record the "harness-selects-not-instructs" divergence in §12 |
| G2 | **Project-memory extension (generative loop) not in the faithfulness map** | **medium** | ARCH §12 | Add a "Project memory: paper static / RK generative superset" row (ADR-0008/0009/0011) |
| N1 | `test_weakening` in data-model example reads as if it were the paper category | low (cosmetic) | data-model §4.4 | One-line "RK category; paper = test" pointer |
| N2 | "code" vs "file residue" merged into `Residue` — loses label granularity (not data) | low (by design) | PRD 04, ADR-0020 | None; documented & sound |

Items **D1–D3** are *good news* (caveats discharged). **G1** and **G2** are the two real fidelity gaps a memory engineer should chase — both are about **naming a divergence that is currently implicit**, not about reworking mechanism. Mechanism is faithful (failure taxonomy 1:1, entropy map exact, context_trace present, memory loop sound).

---

## 9. Cross-persona handoffs

- **AI-harness engineer / eval owner**: the §7 ADR freezes (0019, 0020) and the §8 caveat discharges touch ARCHITECTURE §12 and eval-plan §3, which are co-owned; G1's H2-gate phrasing ("context selection") is theirs to reconcile with the missing protocol artifact.
- **Systems architect** (owns data-model): N1 is a one-line comment on the `entropy.jsonl` example; no schema change. The `validated: bool` skill column from Round 2 rec 2 still lands here (unchanged by this round).
- **Software architect**: G2's faithfulness-map row references ADR-0008/0009/0011 (memory mental model) — their decomposition narrative.
- **Product/research owner**: ADR-0018 (episode=turn) and ADR-0028 (H0) stay Proposed by design — both are product ratifications, not fact-checks; this round does not move them.
