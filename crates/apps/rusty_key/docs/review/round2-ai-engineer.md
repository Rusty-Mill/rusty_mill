*Point-in-time working document — Round 2 of the multi-persona review, AI-engineer lens. Assesses the nine external references (briefs in [`round2-sources.md`](./round2-sources.md)) for whether they should EXPAND SCOPE or REFINE APPROACH of Rusty Keys' model-facing intelligence. Hard constraint: stay Rust + AI-first — learn patterns, not code. Does not re-litigate Round 1 ([`ai-engineer.md`](./ai-engineer.md)); algorithmic specifics here are **v1 intent, revisit after a spike**. Superseded by the round-2 consolidated recommendations.*

# Round 2 — AI Engineer review

## 1. Scope & lens

The model-facing self-improvement machinery: the **consolidation → skill → recall loop** (PRD 03), skill `importance`/grooming and the failure-born-skill boost, the consolidation emit contract, subagent orchestration via `agent` + `SessionFactory` + plan mode (PRD 03), the entropy/intervention signals that feed learning (PRD 04), and the maturity metrics that prove the loop works (eval-plan). Round 1 already *pinned* the recall formula, consolidation contract, system-prompt producer, and the four-hop self-improvement loop. Round 2's job is narrow: do these nine sources **sharpen** that loop or **widen** it, without breaking Rust + AI-first?

Two sources land squarely in my lens (SkillOpt, ADHD) and get concrete proposals; one is memory-model adjacent (Rowboat); the rest I verdict briefly. I grounded SkillOpt and ADHD against their primary sources (web search; GitHub/arXiv 403'd) — mechanics below are confirmed, not just brief-derived.

## 2. Per-source verdict table

| source | verdict | what to take (pattern only) | target RK doc | priority |
|---|---|---|---|---|
| **SkillOpt** (MS, MIT, 1k★, arXiv) | **ADOPT (refine)** | Validation-gated skill promotion: separate **optimizer model** proposes structured edits; accept a skill edit **only if it improves on a held-out check set**; persist the winner as a deployable artifact exempt from decay. Rollout=forward pass, reflection=backward pass, bounded "textual learning rate." | PRD 03 (consolidation contract, grooming, recall floor); eval-plan §5 | **P0** |
| **ADHD** (skill on Claude Agent SDK) | **ADOPT (refine)** | Divergent→converge: fan out **N isolated** branches under structurally different *cognitive frames*, generator/critic split enforced **mechanically** (separate calls, opposite system prompts); critic scores every leaf, prunes traps, deepens top-K. | PRD 03 (`agent` tool strategy + a plan-mode option) | **P1** |
| **Rowboat** (TS/Electron, 14.6k★) | **PARTIAL (refine)** | Editable/inspectable Markdown memory over cold retrieval validates `direct_edit` + desktop memory browser; nudges skills/summaries toward human-legible Markdown bodies and a "memory is inspectable state" framing. | PRD 03 (skill/summary body shape); PRD 04 `direct_edit`; PRD 08 (browser, cross-dep) | P2 |
| **QMind** (experimental, 12★) | **INSPIRATION ONLY** | *Contradiction handling* on `update`/`supersedes` is the one transferable idea (a consolidation prompt rule). **Ignore** the 5-tier memory (our 3-tier is deliberate) and the quantum/meta-cognition framing. | PRD 03 (create-vs-merge prompt rule only) | P3 |
| **AI-Q** (NVIDIA, Apache, 670★) | **REFINE (small)** | Single orchestration node that classifies intent + sets depth in one step (≈ ADHD critic/router); per-message **source toggles** (context engineering) ≈ a recall "which memory types this turn" control. Config-as-data/eval-harness are systems/eval-lens, not mine. | PRD 03 (recall type-filter knob — v1 intent) | P3 |
| **EvalMonkey** | **DEFER (eval lens)** | Failure-trace→synthesised-test ties to our attribution→skill loop, but it's an eval-plan concern. Note the adjacency only. | eval-plan (other persona) | P3 |
| **opendocswork-mcp** | **N/A (not my lens)** | `rmcp` crate choice = systems/integration lens (PRD 07). GPL-3.0: reference only. | — | — |
| **Eino** (Go) | **N/A (arch lens)** | Typed graph + callback aspects + interrupt/resume = architecture lens. Its DeepAgent sub-agent model is a weaker analog than ADHD for my purposes. | — | — |
| **Anthropic "How we contain Claude"** | **NOTE (eval integrity)** | "Read git history to find a test's answers / gamed the benchmark" → **golden episodes must resist gaming**; relevant to attribution honesty, but containment is the security lens. | eval-plan §5 note (cross-dep) | P3 |

## 3. Concrete recommendations

### R1 — Validation-gate skill promotion (SkillOpt) — PRD 03 consolidation + recall + eval §5 — **P0**

**What.** Today consolidation *emits* a `skill` (importance ≥0.8) the instant a turn is UNVERIFIED, and that skill is immediately recall-eligible and `prune()`-exempt with a 0.6 importance floor. SkillOpt's central lesson: a skill edit should be **provisional until it demonstrably helps**. Introduce a two-state skill lifecycle and a promotion gate:

- **`skill` (candidate) vs `skill` (validated).** Add a `validated: bool` (v1 intent) to the skill memory. A freshly minted failure-born skill is a *candidate*: recall-eligible (so the lesson still surfaces next turn) but **not** yet floored at 0.6 and **still subject to grooming/decay**.
- **Promotion = the loop's own VERIFIED signal.** When a later turn whose attribution context matched that candidate skill (the existing `+0.15` boost match) goes **VERIFIED**, flip `validated = true` and apply the 0.6 floor + `prune()` exemption. This is RK's in-loop analog of SkillOpt's "accept the edit only if validation improves" — our held-out check is the *next real verified outcome on a matching condition*, not a separate batch. **This sharpens step 4 of the self-improvement loop**: VERIFIED no longer just bumps `importance`, it *promotes* the skill.
- **Grooming as the optimizer.** Reframe `refine`/`merge`/`split` (PRD 03 "Skill grooming") as SkillOpt's optimizer step: a `merge`/`refine` that produces a candidate body should be **gated on not regressing** the conditions its source skills already validated against (track each validated skill's matching `(failure_type, layer)` conditions; a merge that drops a previously-validated condition is rejected or downgraded to candidate). Bounded edits = SkillOpt's "textual learning rate."
- **`best_skill.md` analog.** Validated skills are RK's deployable, rollback-exempt artifact (SkillOpt persists the best skill as Markdown, exempt from rollback — we already exempt skills from `prune()`; this just makes promotion *earned*). Optionally let the desktop memory browser export the validated-skill set as Markdown (cross-dep with Rowboat R3 / PRD 08).

**Why.** Round 1 closed the loop but left it *credulous*: every failure mints a high-floor, prune-exempt skill on a single unverified turn, so a wrong or over-fit lesson is as durable and recall-privileged as a proven one. SkillOpt's validation gate is the missing quality bar — and it maps onto signals RK *already computes* (verification verdict + attribution match), so it costs no new LLM call in the hot path.

**Exact target.** PRD 03 → *Consolidation → importance rubric* (candidate vs validated emit), *Recall → failure-born skill boost* (floor applies only when `validated`), *Skill grooming* (non-regression gate), *Self-improvement loop* step 4 (promotion on VERIFIED). One line in eval-plan §5 (a golden episode that asserts a candidate skill **promotes** after a matching verified turn).

**Rust + AI-first note.** Pure prompt/scoring + a bool column — no SkillOpt code. The optimizer/actor split is conceptual: our "optimizer" is the consolidation model + grooming pass; our "actor" is the frozen kernel model. `validated`/condition-set live in the `memories` row (systems-architect owns the column — cross-dep).

**Credibility.** SkillOpt is high (MIT, MS, arXiv, 52/52 claim). Keep the gate mechanism; do **not** import its batch-training loop (RK learns online from real turns, not offline scored batches) — that's the deliberate adaptation.

### R2 — Divergent→converge as a subagent strategy + plan-mode option (ADHD) — PRD 03 `agent` tool — **P1**

**What.** ADHD is a concrete realization of "use the subagent machinery for *parallel divergent exploration*, then converge." It fits RK's existing primitives with **no new infra** — `SessionFactory::spawn_child` already gives isolated-history children sharing `.rustykeys/`. Propose two additive surfaces:

1. **A subagent *fan-out* strategy (additive to the `agent` tool).** The ADHD pattern = spawn N children via `SessionFactory`, each seeded with the **same task** but a **different cognitive-frame preamble** (the "focused subagent identity preamble" PRD 03 already floats as a v1 option — ADHD gives it teeth: *regulator / speedrunner / $0-budget / infinite-budget / 3am-on-call*), **no cross-branch context** (children already get isolated history), then a **critic pass** (the parent, or a dedicated critic child with the *opposite* system prompt) scores every result, prunes, and deepens the top-K. The generator/critic split is enforced *mechanically* (separate Sessions, opposite preambles) — which is exactly RK's isolation model, not a single-context promise.
2. **A plan-mode "explore" option.** Plan mode (PRD 06, `enter_plan_mode`) is the natural convergence gate: a divergent fan-out runs *inside* plan mode (writes/bash already blocked), produces K candidate plans, and the user/critic approves one before `exit_plan_mode`. Divergent-explore → converge → approve maps 1:1 onto plan mode's lifecycle.

**Why.** The `agent` tool today is described only as *decomposition* (focused subtask). ADHD adds a second, distinct use — *divergent ideation* on open-ended design — that the same `SessionFactory` already supports and that beats single-shot 5/6 on open-ended tasks. It's the difference between "spawn a worker" and "spawn a brainstorm." Worth naming so the implementer builds the frame-preamble + critic seam rather than re-inventing it.

**Exact target.** PRD 03 → *Agent tool* (add an "exploration / fan-out strategy" subsection: same task, N frame-preambles via the system-prompt producer's overridable identity layer, isolated history, critic-converge on top-K) and a forward-link from PRD 06 plan mode. Mark the frame list + K **v1 intent**.

**Rust + AI-first note.** Reuses `SessionFactory`, `AgentDepthPolicy`, and the layered system-prompt producer (the frame preamble is just an override of layer 1 "Identity"). Fan-out is `tokio::join!`/`JoinSet` over `spawn_child` — Rust concurrency is a strength here. The critic is one more aisdk call with an opposite-framed prompt. Watch the cost/depth budget (N parallel children × tokens) — gate behind an explicit strategy, not the default `agent` path.

**Credibility.** Method/skill, benchmarked on 6 tasks (small N, LLM-as-judge) — adopt the *pattern*, treat the win-rate as directional. It's a method to emulate, not a dependency.

### R3 — Push memory toward inspectable/editable Markdown (Rowboat) — PRD 03 + PRD 04 + PRD 08 — **P2**

**What.** Rowboat's thesis — *editable Markdown memory + inspectable graph beats cold retrieval* — validates two things RK already has (local-first store, `memory_edges` graph, `direct_edit` intervention, desktop memory browser in PRD 08) and refines one: **skill/summary bodies should be authored as human-legible Markdown**, not terse machine strings, so a user can read and *correct* a wrong lesson in the browser (closing a human-in-the-loop path on the self-improvement loop). Add a small note to PRD 03 that `body` for `skill`/`summary` is Markdown-shaped, and a forward-link from `direct_edit` (PRD 04) / the memory browser (PRD 08) that editing a skill body sets it back to **candidate** (un-validates — ties to R1) so a human correction must re-earn promotion.

**Why.** It's mostly *validation* of existing posture (don't expand), but the "human edit → re-validate" hook is a genuinely new, cheap link between Rowboat's editable-memory idea and R1's gate. Low priority because it's additive polish, not a gap.

**Exact target.** PRD 03 *Consolidation emit contract* (body is Markdown; one line). PRD 04 `direct_edit` and PRD 08 memory browser (cross-dep) — a human skill edit flips `validated=false`.

**Rust + AI-first note.** No code lift from Rowboat (TS/Electron). Markdown bodies are free (already strings); the re-validate hook reuses R1's bool.

**Credibility.** High-visibility (14.6k★, YC) but the *transferable* part is a design posture we mostly already hold — so the bar for adopting more of it (its KG-construction specifics) is "only if a concrete gap appears," which it doesn't here.

## 4. Cross-persona dependencies

- **Systems architect** (owns data-model §3): R1 needs a `validated: bool` + a per-skill validated-condition set on the `memories` row, and decay/`prune()` to respect `validated`. R3's "edit un-validates" is a write path on the same column. My R1/R3 edits are blocked on their column decision.
- **AI-harness engineer**: R1 changes the self-improvement loop's step 4 (VERIFIED → *promote*, not just bump) and adds an eval-plan §5 golden episode asserting promotion — overlaps their loop-faithfulness + eval ownership. R2's plan-mode-explore touches their plan-mode lifecycle (PRD 06).
- **Software/integration architect**: R2 fan-out is `JoinSet` over `SessionFactory::spawn_child` under `AgentDepthPolicy` + a cost/token budget — they own the concurrency/depth/cost guard so N parallel children don't blow the budget.
- **Product/research owner**: numbers stay product calls — R1's promotion still rides the existing `+0.15` match threshold; R2's N (branch count), the cognitive-frame list, and top-K are **v1 intent** for the owner to set.
- **Security persona** (Anthropic source): eval-integrity note — golden episodes (eval-plan §5) must resist git-history/benchmark gaming; their lens, flagged so attribution-completeness metrics aren't quietly gamed.
