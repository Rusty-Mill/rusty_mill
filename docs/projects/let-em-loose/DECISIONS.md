# Let Em Loose: decisions

Each entry gives the decision, why it was made, and what was rejected. **Agreed** means Nano has approved it. **Proposed** means it is the working direction but still awaits a design or confirmation. From D46 on, entries carry a project-handoff tag instead ([handoff/DECISIONS.md](handoff/DECISIONS.md)): `[stated]` or `[agreed]` (Nano's; ask before changing) or `[default]` (an agent's choice; revise with a new entry saying why), plus the remind-me id. Only the `let-em-loose-expert` subagent appends entries. Dates are when the decision was made. Where an earlier entry and a later one conflict, the later one wins. The detailed BBP rules are in the claims addendum, `crates/libs/protocol/rusty_bbp/docs/spec/bbp-claims-addendum.md` (PR #1386).

| # | Decision | Status |
| --- | --- | --- |
| D1 | Lenses, not stages | Agreed 2026-11-01 |
| D2 | Converge on claims, not conversation | Agreed 2026-11-01 |
| D3 | The board computes confidence | Agreed 2026-11-01 |
| D4 | Budget is the price of certainty | Agreed 2026-11-01 |
| D5 | Nano writes intent and judges exceptions | Agreed 2026-11-01 |
| D6 | Keep dissent honest | Agreed 2026-11-01 |
| D7 | Several blind candidates, including a rebel | Agreed 2026-11-02 |
| D8 | The board is the context | Agreed 2026-11-02 |
| D9 | Planning, building and testing are concurrent | Agreed 2026-11-02 |
| D10 | Research: learn, don't inherit | Agreed 2026-11-01 |
| D11 | BBP is a pure protocol | Agreed 2026-10-11, reaffirmed 2026-11-02 |
| D12 | orch is model-independent, via ports and adapters | Agreed 2026-11-02; providers refined in D34 |
| D13 | No agent-to-agent calls | Agreed 2026-11-02 |
| D14 | Tool-using agents join through the board over MCP | Proposed 2026-11-02 |
| D15 | Concurrent, event-driven evidence posting | Agreed 2026-11-02 |
| D16 | Value-driven scheduling is a shadow experiment | Agreed 2026-11-02 |
| D17 | Build in the rusty_mill monorepo | Agreed 2026-11-02 |
| D18 | The dedup experiment gets a fresh repo and intent only | Agreed 2026-11-01 |
| D19 | The Factory repo holds process and config only | Agreed 2026-11-02 |
| D20 | No fourth executor: extend orch | Agreed 2026-11-02 |
| D21 | Thin slice first, then let it build itself | Agreed 2026-11-01 |
| D22 | Ground truth sits outside the agents' loop | Agreed 2026-11-02; designed in D37 |
| D23 | Defer existing code and the smaller gaps | Agreed 2026-11-02 |
| D24 | Audit remind-me before building | Agreed 2026-11-01 |
| D25 | Design first on every card | Agreed 2026-11-02 |
| D26 | The name is Let Em Loose | Agreed 2026-11-02 |
| D27 | Claims sit beside the stage lifecycle | Agreed 2026-11-02 (#593) |
| D28 | Replay determinism, not order independence | Agreed 2026-11-02 (#593) |
| D29 | Scoring and budget defaults | Agreed 2026-11-02 (#593) |
| D30 | A failing test blocks acceptance | Agreed 2026-11-02 (#593) |
| D31 | Superseding a claim | Agreed 2026-11-02 (#593) |
| D32 | When a dependency collapses | Agreed 2026-11-02 (#593) |
| D33 | Action marks and held reruns | Agreed 2026-11-02 (#593) |
| D34 | Lenses, providers and local models | Agreed 2026-11-02 (#594) |
| D35 | The inbox is a read view over BBP in Mill Board | Agreed 2026-11-02 (#595) |
| D36 | Value scheduling lives in orch, with a pre-registered rule | Agreed 2026-11-02 (#596) |
| D37 | Holdouts, protected tests, golden sets, calibration | Agreed 2026-11-02 (#597) |
| D38 | Phase B intents and spend | Agreed 2026-11-02 (#596) |
| D39 | remind-me gets an explicit project parameter | Agreed 2026-11-02 (#592) |
| D40 | Build the slice before designing more | Agreed 2026-11-02 |
| D41 | Gate on code erosion, not only behaviour | Agreed 2026-11-03 (#704) |
| D42 | Erosion limits come from our own history | Agreed 2026-11-03 (#704) |
| D43 | No erosion override; inbox Approve is the override | Agreed 2026-11-03 (#704) |
| D44 | Scheduled cleanups pay erosion down | Agreed 2026-11-03 (#704) |
| D45 | Rust only; unmeasured code is reported, not failed | Agreed 2026-11-03 (#704) |
| D46 | Three-tier escalation for design questions | [stated] 2026-11-03 (handoff) |
| D47 | Ask the project expert before Nano | [stated] 2026-11-03 (handoff) |
| D48 | Never invent a decision; unsourced goes to OPEN_QUESTIONS | [stated] 2026-11-03 (handoff) |
| D49 | The handoff pack is scoped to the project folder | [default] 2026-11-03 (handoff) |
| D50 | One decision log, D-numbering continues | [default] 2026-11-03 (handoff) |
| D51 | The expert is the single writer of settled answers | [default] 2026-11-03 (handoff) |
| D52 | rusty_erosion location and core/adapter module split | [default] 2026-11-03 (#704) |

## D1. Lenses, not stages

**Decision.** Roles (planner, tester, reviewer, security, operator) become critic lenses that attack one shared artefact at the same time.
**Why.** Factory roles exist because humans can't hold everything at once. For agents, the handoffs only add queues, latency and lost context. The perspectives are the valuable part, not the sequence.
**Rejected.** Sequential role pipelines, including the earlier Factory design of one Claude Code thread per role with approval gates.

## D2. Converge on claims, not conversation

**Decision.** Agents post structured claims and evidence. They do not exchange prose.
**Why.** Prose debate is a human limitation. Structured claims are cheaper, auditable, and can be executed where possible.
**Rejected.** Chat-style debate between agents. Abandoning text altogether: tokens remain the native interface across vendors, and claims stay human-readable for audit.

## D3. The board computes confidence

**Decision.** Confidence is computed by the board from the evidence, as a deterministic Bayesian-style update. Executable evidence outweighs opinion, and agent track records weight the evidence.
**Why.** Agents self-reporting confidence is ungrounded and can be gamed. A deterministic rule is testable and replayable.
**Rejected.** Agents asserting their own confidence. Majority votes.

## D4. Budget is the price of certainty

**Decision.** Allowances scale with the stakes and reversibility of each decision. Work stops early once evidence stops moving confidence. A global ceiling and a circuit breaker cap total spend (#598).
**Why.** A flat token budget overspends on trivial calls and underspends on one-way ones.
**Rejected.** A flat token budget per task (Nano's earlier framing). Spending the whole budget just because it exists.

## D5. Nano writes intent and judges exceptions

**Decision.** Nano writes the intent. The board escalates only irreversible actions, taste calls, deadlocks the evidence can't settle, and budget overruns.
**Why.** Nano said he mostly clicks approve without reviewing. A rubber-stamp gate adds delay and false assurance.
**Rejected.** An approval gate at each stage.

## D6. Keep dissent honest

**Decision.** Adversarial lenses run on different models. Each lens forms its view blind before seeing the others. Agreement must cite a test, property or measurement. A lens that finds nothing is pushed to look harder.
**Why.** Agents sharing a model and a context drift into agreeing with each other.
**Rejected.** One model playing several personas. "Looks good" counting as support.

## D7. Several blind candidates, including a rebel

**Decision.** Open questions go to several models blind. At least one candidate must deliberately depart from the known approach. A question with only one candidate counts as more uncertain. Attacks start only once several candidates exist, and target all of them equally.
**Why.** Nano raised the groupthink concern. If one lens proposes and the others only attack, everything anchors on the first idea.
**Rejected.** One research answer that everyone else attacks.

## D8. The board is the context

**Decision.** Lenses are stateless for each question. For every question, orch assembles: the cached intent and rules, the claim with its evidence and dependencies, relevant settled claims at one line each, and the lens's track record. Builders may keep a short session within one task. Cross-project lessons go in remind-me.
**Why.** This avoids context rot and self-anchoring, makes blind first passes enforceable by construction, and keeps cost low.
**Rejected.** Agents keeping long conversations. Resetting context only at phase boundaries.

## D9. Planning, building and testing are concurrent

**Decision.** Planning, building and testing are activities, not phases. They run together on every claim. The only ordering: intent comes first, irreversible actions last.
**Why.** Phases are a human scheduling device.
**Rejected.** A plan, build, test sequence.

## D10. Research: learn, don't inherit

**Decision.** A research phase runs between the intent interview and the prototypes, and a research lens stays active throughout. Prior art is evidence and a baseline to beat, never a template. Copying code is out. At least one first-principles candidate competes on the same evidence.
**Why.** Nano wanted lessons already learned to be used, without forcing the work into existing designs. The square peg applies to dedup algorithms as much as to the process.
**Rejected.** Starting from a blank page. Following existing tools' designs.

## D11. BBP is a pure protocol

**Decision.** BBP owns the board's rules: claims, evidence, transitions, thresholds, budgets and escalation. Roles, rosters, lenses and model choices live in orch. bbp-core stays independent of orch-core, and the two compose (2026-10-08).
**Why.** In October BBP had hard-coded four roles, which made it one pipeline dressed up as a protocol.
**Rejected.** Roles in BBP. Rebasing BBP onto orch-core's board.

## D12. orch is model-independent, via ports and adapters

**Decision.** orch core defines one port: given a lens's context and claims, return structured evidence. There is one adapter per provider: Claude, OpenAI/Codex, Gemini, local llama-swap. Config binds each lens to an adapter, a model, a tier and fallbacks. Adapters validate output and reject non-conforming replies. JARVIS is the face, not the engine.
**Why.** Model diversity only works if no single model owns the engine. Swapping a model should be a config edit.
**Rejected.** A single-vendor engine. JARVIS as the engine. Vendor logic in orch core.
**Later.** D34 replaced the planned Gemini adapter: v0 runs Claude, Codex and local models.

## D13. No agent-to-agent calls

**Decision.** Every exchange goes through the board.
**Why.** Direct calls would bypass blind first passes, the evidence rules and the audit trail.
**Rejected.** Peer messaging between agents.

## D14. Tool-using agents join through the board over MCP

**Decision.** Agents that use tools (Claude Code, Codex) connect to the board through an MCP server and post claims and evidence themselves. orch calls the simple lenses directly.
**Why.** Tool-using agents don't fit a single request and reply.
**Rejected.** Squeezing tool-using agents into the lens port.
**Open.** How this fits whichever executor D20 settles on.

## D15. Concurrent, event-driven evidence posting

**Decision.** The claims model is built on event-driven, concurrent evidence posting (BBP Phase 2). Any agent posts when it has evidence; subscribed lenses wake and read the delta. The control plane stays serialized.
**Why.** Turn-taking throttles the flow the claims model depends on.
**Rejected.** A turn-based loop. Designing claims on top of turn-taking.
**Prerequisite.** Confirm on main whether Phase 1 is live and how much of Phase 2a has landed (#593).

## D16. Value-driven scheduling is a shadow experiment

**Decision.** Try value scoring, the board asking targeted questions, and fork-and-collapse together, as one system. Run it in shadow mode against plain event-driven scheduling, comparing tokens, time to settle and decision quality (#596).
**Why.** "Loop or event" only answers when an agent acts, not what is worth doing. The value approach is unproven, so it must beat a baseline before it becomes the default.
**Rejected.** Treating loop versus event as the only options. Adopting value scheduling without evidence.

## D17. Build in the rusty_mill monorepo

**Decision.** BBP, orch and Mill Board stay where they are, and the cards land there.
**Why.** They already live in rusty_mill and are tested together. The cards change the contracts between them. There is no forcing function for extraction.
**Rejected.** A new Let Em Loose repo.

## D18. The dedup experiment gets a fresh repo and intent only

**Decision.** The experiment's agents get only the intent. They never see `rusty_fclone`. Studying public tools such as fclones is allowed; copying code is not. `rusty_fclone` is the control.
**Why.** The experiment has to test the process, not reproduce existing code.
**Rejected.** Running the experiment inside rusty_mill. Handing over `rusty_fclone` to rebuild.

## D19. The Factory repo holds process and config only

**Decision.** `baileyrd/Factory` holds the intent template, lens definitions and model bindings. No code.
**Why.** The process can change without a code release.
**Rejected.** Code in the Factory repo.

## D20. No fourth executor

**Decision.** rusty_mill already has three executors: the orch dispatcher with BBP, mill-runner (worktree plus rusty_sandbox), and the super-board robot. The lens runner extends or replaces one of them.
**Why.** A fourth would deepen the duplication the consolidation audit is trying to remove.
**Rejected.** A new standalone executor.
**Resolved 2026-11-02.** The reconnaissance found super-board already retired (2026-10-28), so two executors remain. The lens runner extends orch, next to the dispatcher on orch's existing model port. mill-runner is left alone.

## D21. Thin slice first, then let it build itself

**Decision.** The order is: the claims model, two or three lenses, a bare inbox, then the dedup experiment. Once that works, it helps build the rest.
**Why.** End-to-end evidence comes before polish, and the slice is the honest test of the idea.
**Rejected.** Building the UI first.

## D22. Ground truth sits outside the agents' loop

**Decision.** Holdout tests are written from intent by a lens that builders never see. A small golden set is signed off by Nano. Calibration is tracked: claims accepted at about 90% should hold about 90% of the time (#597).
**Why.** Agents producing and judging their own evidence can be confidently wrong. A false 92% is worse than an honest 50%.
**Rejected.** Trusting agent-written tests alone.

## D23. Defer existing code and the smaller gaps

**Decision.** Existing code, sandbox and data boundaries, intent versioning and runaway cost go on #598. Lessons, gameable track records, maintenance and replay go on #599. Neither blocks v0, but existing code is the biggest risk for real projects.
**Why.** Further gap-hunting had diminishing returns. The remaining gaps will show up when the system runs.
**Rejected.** Designing every gap before building anything.

## D24. Audit remind-me before building

**Decision.** Audit the existing `project` field and the `remind_me_project` tool for active-project filtering before building anything new (#592).
**Why.** The capability may already partly exist. Writes from the remote connector come back with `project: null`.
**Rejected.** Building a new mechanism straight away.

## D25. Design first on every card

**Decision.** Each card posts a design or report for Nano's approval before any implementation. Schema and API changes are flagged.
**Why.** This matches Nano's working agreement, and these contracts are expensive to change.
**Rejected.** Implementing straight from the card text.

## D26. The name is Let Em Loose

**Decision.** The project is called Let Em Loose. A suggested tagline: loose, but on evidence.
**Why.** Nano chose it.
**Rejected.** "Beyond the Software Factory", which is kept as the concept doc's subtitle.

## D27. Claims sit beside the stage lifecycle

**Decision.** BBP's task lifecycle (planning, build, test, review) is itself the factory pipeline. In v0, claims are an opt-in mode per task: claims-mode tasks skip the stage lifecycle, and stage-mode tasks are unchanged. Evidence posts need no turn; the one-live-turn rule stays for state changes, budgets and approvals.
**Why.** Additive, so nothing existing breaks. Retiring the lifecycle is a later, separate decision once claims have proven themselves.
**Rejected.** Replacing the lifecycle in v0. Lifting the one-live-turn rule for every write.

## D28. Replay determinism, not order independence

**Decision.** The guarantee is that the same log always replays to the same outcome. Settling depends on arrival order, which the log records. Settled claims keep taking evidence: an executed contrary result reopens a claim into a fresh epoch, where it must re-earn its settlement, and a claim already acted on escalates instead. A passing run reopens a dropped claim the same way. A claim that has been superseded stays closed.
**Why.** Order independence could not hold once claims settle at a threshold. Refusing evidence after settling would stop holdouts and golden sets from catching a wrong settlement.
**Rejected.** Order independence. Refusing evidence once a claim settles.

## D29. Scoring and budget defaults

**Decision.** Confidence is a whole-number score.

| Setting | Value |
| --- | --- |
| Accept | 8 (90%) for low and medium stakes; 12 (about 96%) for high stakes and irreversible claims |
| Drop | -8 (10%) |
| Executed test | pass +4, fail -8; strength fixed at 1 |
| Cited source | weight 2, through a `source` artifact |
| Opinion | can never reach accept |
| Authors | at least 2 distinct authors to accept |
| Stuck | the last 6 evidence posts moved the score by 2 or less in total: escalate |
| Budget unit | one evidence post or one candidate run |
| Budget per claim | 8, 16 or 32 units for low, medium or high stakes; doubled if irreversible |
| Track records | recorded, not used as weights in v0 |

Storing a candidate that names a claim starts its run, which costs one unit and runs only in the sandbox. `sole_candidate` is a fact recorded in BBP; what to do about it is orch's decision.
**Why.** Two independent passing tests reach accept; one counterexample weighs as much as two passes, because a failure falsifies and a pass only supports. High-stakes claims need a third test.
**Rejected.** Probability floats with author-rated strength (one author could make a failure worth -24). A single accept line for every stake.

## D30. A failing test blocks acceptance

**Decision.** A claim cannot be accepted while the current candidate's latest run in the epoch failed. Only a passing run that includes the failed profiles clears it. The host freezes the profile set when a task opens, in a host-only file, and records its hash in `task_opened`; the runner and the report check refuse any mismatch. A new candidate inherits the block until its own passing run includes those profiles. A reopened claim starts out blocked. A candidate whose diff changes or deletes listed test files cannot clear a block unless a different principal endorses the test change with cited support.
**Why.** Without the block, enough passes from other authors could accept a claim while a real failure stood. The rest closes the ways around it: dropping the test from the rerun, editing the profile, or editing the test.
**Rejected.** Score-only treatment of failures. Tracking individual tests, which would need a report schema change.
**Known limit.** Rust unit tests inside source files are not caught by the path check. The holdout suite (D37) covers this.

## D31. Superseding a claim

**Decision.** Any claim can be superseded, open or settled. The replacement takes effect only once the new claim is itself accepted; until then both stay live. Superseding a claim that was already acted on needs Nano's accept in the inbox, and the action record is kept.
**Why.** A badly worded open claim needs to be replaceable, but posting a replacement must not let an agent escape a failing test.
**Rejected.** Immediate supersession. Superseding settled claims only.

## D32. When a dependency collapses

**Decision.** Dependents that nothing has acted on reopen automatically. If the dependency was dropped, those dependents are voided in the same step and stay visible, so they can be re-proposed with new dependencies. Only dependents already acted on escalate. A new claim cannot depend on a dropped one.
**Why.** Escalating every collapse would flood the inbox with decisions that don't need Nano.
**Rejected.** Escalating every collapse. Automatically voiding claims that were already acted on.

## D33. Action marks and held reruns

**Decision.** Before acting on a claim, the host records a mark ("about to merge X") against the epoch it saw. If contrary evidence got in first, the mark is refused and nothing happens. A marked claim counts as acted on even if the host crashes; a mark with no recorded outcome after 1 hour (configurable) escalates for reconciliation. When the budget is spent, a rerun requested by an agent is refused, and one requested by a human becomes a budget escalation in the inbox. The old run stays valid either way.
**Why.** Without marks, a merge racing a failing test could lose the record that the claim was used. A human should never be silently blocked by a budget.
**Rejected.** Acting without a mark. Refusing human reruns outright.

## D34. Lenses, providers and local models

**Decision.**
- **Providers in v0:** Claude, Codex and local. The hard-coded provider list in orch core becomes a config-driven registry. A new `orch-openai` crate (one crate per adapter, as orch already does) talks to llama-swap, and plain HTTP is allowed only to loopback and private addresses. Gemini joins later through the same adapter over HTTPS. Stored records keep old provider names; readers map them.
- **Lenses:** each new question gets three blind Architect proposals on three models (Claude, local, and a Codex rebel; which model is the rebel is config). Research, Adversary and Operator follow. The lens asked about a claim rotates across the eligible lenses and models, and is never the claim's author.
- **Local models:** Operator on Qwen3.6-35B-A3B; gpt-oss-120b as the local high tier for sensitive or contested claims; 32k context to start.
**Why.** The Gemini CLI was discontinued, and every existing adapter is a CLI. One OpenAI-compatible adapter covers llama-swap now and Gemini later. Rotation and author exclusion keep the old "the reviewer is never the builder" rule.
**Rejected.** A Gemini adapter in v0. A fixed lens per kind of question.

## D35. The inbox is a read view over BBP in Mill Board

**Decision.** The inbox is a read view over BBP served by Mill Board. Nothing is copied into rusty_tick, because an escalation is a decision, not a task (ADR-0012). Mill Board reads only through the host's `bbp` command, never BBP's files, so in v0 it runs on the orch host. Each card shows the claim, confidence, budget, reason, the top two supporting and contrary evidence items, and a dry run computed by the pure engine. Actions:
- **Approve:** accepts. On a budget escalation, or on a whole-task budget, it adds one base allowance for the claim's tier, never more. On an irreversible claim it adds no budget.
- **Reject:** drops. If the claim was already acted on, it also creates a "Remediate" card carrying the dry-run diff.
- **Choose:** for taste escalations, Nano picks one of several candidates; the chosen one is accepted and the rest dropped, in one step.
- **Refine intent:** a human answer that the lenses see in their context. Recorded so it can become an intent version under #598.
No push notifications in v0. Sorting uses BBP facts: irreversible first, then oldest.
**Why.** One source of truth, and Mill Board already has admin gates, MCP and a web UI.
**Rejected.** Storing escalations in rusty_tick (this reverses the earlier recommendation in QUESTIONS Q21). Approve and Reject only.

## D36. Value scheduling lives in orch, with a pre-registered rule

**Decision.** The value score, "the board asks" and fork-and-collapse all live in orch behind a `Policy` trait, with no BBP change. Fork-and-collapse only starts after #600, on reversible claims, with two sandboxed branches capped at 2 units and 30 minutes each. On a tie, the simpler or cheaper branch wins and the reason is recorded; it goes to Nano as `choose` only if the intent marks it as taste. Evaluation runs in shadow mode first, then paired runs on five intents. Value scheduling becomes the default only if (a) its holdout result is no worse on all five (at most one more failing holdout test than the baseline), (b) it uses at most 80% of the baseline's tokens on at least four of five, and (c) total escalations are no higher than the baseline.
**Why.** The score is a scheduling policy, not a board rule, so it can change without touching replayed logs. Five runs is a small sample, so quality and Nano's time are not traded for savings.
**Rejected.** Exposing the score from BBP for sorting the inbox. A rule that allows worse quality on one intent.

## D37. Holdouts, protected tests, golden sets, calibration

**Decision.**
- **Holdouts:** a second frozen profile suite, written from the intent alone by a holdout lens on a model no builder uses (config is refused otherwise). Only the holdout lens can run it, on the same base and diffs as the current candidate. Every claim with a candidate needs a passed holdout run to be accepted. Builders see only profile names and pass or fail counts.
- **Protected test paths:** a candidate whose diff touches one is refused, with the paths named. A real test fix goes in as its own `tests` claim, which a different lens must support. This replaces the interim rule in D30.
- **Holdout versions:** a new version needs Nano only when an existing holdout test is removed or changed; pure additions are adopted automatically.
- **Golden sets:** in the Factory repo at `golden/<intent>/`, with a SIGNOFF file only Nano can commit (CODEOWNERS). The host refuses to open a task with an unsigned set. Private fixtures from Nano's drive are recorded as hashes only and mounted read-only. Golden cases run as `golden:` profiles inside the holdout suite.
- **Calibration:** orch-store records each accepted claim as held, reversed or unknown, at a 14-day horizon, with a hold rate per score band and nothing proposed below 20 claims per band. It is informational in v0. `rusty_orch calibrate` proposes changes that Nano approves, and they apply only to tasks opened afterwards.
**Why.** Agents that write and judge their own tests can be confidently wrong. Quietly restoring tests would hide the fact that an agent tried to change them.
**Rejected.** Restoring protected paths from base. Using track records as weights in v0.

## D38. Phase B intents and spend

**Decision.** The five Phase B intents are `dedup-lite`, `tick-lite`, `vt-grid`, `gate-board` and `frag-check`, each a cut-down version of an existing project. `dedup-lite` is a benchmark, not the #600 experiment. Acceptance lines describe observable behaviour, never method: for example, "two concurrent writers never corrupt the store and neither write is lost", not "temp file then rename". The `dedup-lite` timing target is 2 seconds on Nano's desktop, warm cache, median of 5, both arms on the same host. Taste domains: `cli-output` in tick-lite and `api-shape` in gate-board. Spend: $200 in total for Phase B, a $25 hard cap per run, and a pause to report if the first paired intent costs more than $40.
**Why.** Small, checkable intents with exact reference implementations keep a five-sample comparison meaningful.
**Rejected.** Method-prescribing acceptance lines. Swapping `vt-grid` for a fuzzier intent.

## D39. remind-me gets an explicit project parameter

**Decision.** `remind_me_add`, search and capture get an explicit `project` parameter, and the remote connector sends it. The proposed write to existing records is on hold until Nano has seen exactly which records it changes.
**Why.** The project was guessed from the caller's working folder; the remote connector has none, so it came out null.
**Rejected.** Guessing the project from other fields.

## D40. Build the slice before designing more

**Decision.** After #597, implementation starts: the #593 engine first, test-first from the claims traces, on its own PR. #598 and #599 are not designed yet.
**Why.** They don't block v0 (D23), and the slice will show which gaps actually matter.
**Rejected.** Designing every card before building any.

## D41. Gate on code erosion, not only behaviour

**Decision.** A small, pure metric, `rusty_erosion`, scores two things in basis points: how much cyclomatic complexity sits in functions above CC 10 (concentration) and how many tokens sit in repeated, normalized 50-token windows (duplication). It runs as an `erosion` profile in the task's frozen profile set and compares the candidate with the pinned baseline, not the absolute level. A candidate over the limit fails that profile, so the existing failing-test block (D30) stops acceptance. There is no BBP schema change. A claims task opens only with an erosion profile or a recorded opt-out, both covered by the profile digest.
**Why.** Every other gate checks behaviour, so a candidate could pass them all while the code got worse. SlopCodeBench (arXiv 2603.24755) found that happens in about 3 of 4 agent trajectories, and that quality prompts lower the starting point but not the rate of decay. Measuring against the baseline means existing code is not punished for mess it already had.
**Rejected.** Relying on the Architect lens prompt alone. A new BBP evidence kind for erosion.

## D42. Erosion limits come from our own history

**Decision.** The limits are calibrated from erosion deltas on rusty_mill's merged human PRs. The gate is report-only until then: the profile always passes, and the deltas still go in the log. Report-only lasts 2 weeks or 10 tasks, whichever comes first. The numbers then go to Nano, who sets the limits.
**Why.** Fixed numbers would be guesses. The repository's own human history is the fair baseline. The time box stops report-only from becoming permanent.
**Rejected.** Starting with fixed limits.

## D43. No erosion override; inbox Approve is the override

**Decision.** v0 has no in-band override. A blocked claim follows the existing path: a better candidate, or a budget escalation to the inbox (D35), where Approve is the override. An erosion escalation shows the change in each metric and the three worst-affected functions.
**Why.** This reuses the inbox and needs no BBP change. The deltas and the worst functions let Nano judge without opening the diff.
**Rejected.** An `erosion` claim domain endorsed by another principal, like `tests` (D37). That stays available for v1 if overrides become common.

## D44. Scheduled cleanups pay erosion down

**Decision.** orch writes a `consolidate` intent when the cumulative erosion since the last cleanup reaches half the limit, or after 5 accepted claims, whichever comes first. A cleanup must keep the frozen suite and the holdouts (D37) green. Its erosion limits are negative, so it is accepted only if erosion actually goes down.
**Why.** Prompts don't slow the decay. Human repositories decay less because people refactor, so the schedule makes that habit explicit and gives it a budget.
**Rejected.** Waiting until a gate blocks before cleaning up.

## D45. Rust only; unmeasured code is reported, not failed

**Decision.** v0 measures Rust only. Other languages are added when a real task needs them. Non-code files (md, toml, yaml, json and similar) are ignored. Code in an unsupported language is marked `unmeasured` on the claim and in the report, and counted in calibration, but it does not fail the profile or escalate. Known limit: the metric can be gamed, for example by splitting functions or renaming. It is a heuristic, and calibration watches for gaming, such as function count or total CC rising while concentration falls.
**Why.** No speculative language support. Recording `unmeasured` keeps the blind spot visible without sending every mixed-language change to Nano.
**Rejected.** Treating unmeasured code as a failure. Supporting Python before a task needs it.

## D46. Three-tier escalation for design questions

**Decision.** A design question is answered when the record (this file, the handoff pack, remind-me, Nano's standing principles) settles it; implementation detail gets a logged `[default]`; only intent, scope or taste, or anything that would reverse a `[stated]` or `[agreed]` decision, goes to Nano.
**Why.** Nano was relaying design questions between Claude sessions by hand.
**Rejected.** Asking Nano whenever a thread is unsure.
**Tag.** [stated] · remind-me: mem_8d9722c4207840afa0df9e187897055d
**Source.** Nano's handoff prompt, 2026-11-03.

## D47. Ask the project expert before Nano

**Decision.** Threads and the orchestrator consult the `let-em-loose-expert` subagent before asking Nano any Let Em Loose design question.
**Why.** The expert holds the design context; Nano only sees what it can't answer.
**Rejected.** Threads asking Nano directly.
**Tag.** [stated] · remind-me: mem_9645385181204711adf0e4bb1b035c4d
**Source.** Nano's handoff prompt, 2026-11-03.

## D48. Never invent a decision

**Decision.** Anything that can't be sourced goes in `handoff/OPEN_QUESTIONS.md`, not into an answer or a decision.
**Why.** A made-up decision looks settled and is worse than an honest gap.
**Rejected.** Filling gaps with plausible answers.
**Tag.** [stated] · remind-me: mem_6c172130fa684069ba0f11f1cca63916
**Source.** Nano's handoff prompt, 2026-11-03.

## D49. The handoff pack is scoped to the project folder

**Decision.** The pack lives in `docs/projects/let-em-loose/handoff/`, the subagent in `.claude/agents/let-em-loose-expert.md`, and the CLAUDE.md rule applies only to Let Em Loose paths and cards.
**Why.** rusty_mill is a monorepo hosting many projects; the v1.2 template assumes one project per repo.
**Rejected.** `docs/handoff/` at the repo root, and a generic `project-expert` name.
**Tag.** [default] · remind-me: mem_5eae422a1d394844951db4a0446cf92c

## D50. One decision log, D-numbering continues

**Decision.** This file stays the only decision log. `handoff/DECISIONS.md` points here and new entries continue at D46.
**Why.** Two logs would drift.
**Rejected.** Copying D1–D45 into the pack.
**Tag.** [default] · remind-me: mem_7c60a2deca2b4004a90a58f221fb518d

## D51. The expert is the single writer of settled answers

**Decision.** Callers pass Nano's answers back to `let-em-loose-expert`, together with where he said them. The expert writes one `remind_me_add` and then one entry here, both carrying a stable key, and ticks `handoff/OPEN_QUESTIONS.md`. Callers never write either.
**Why.** A single writer with a key keeps each answer recorded once. This is best effort: concurrent sessions or a crash between the two writes can still duplicate an answer, and the key is how the next run reconciles it (#1445 review).
**Rejected.** Each thread recording its own answers.
**Tag.** [default] · remind-me: mem_df536a0f908b40db9fdb87196ba62e35

## D52. rusty_erosion location and core/adapter module split

**Decision.** The crate lives at `crates/tools/rusty_erosion` (`[package.metadata.rusty_mill] layer = "tools"`, added to the workspace `members` next to `crates/tools/rusty_baseline`). `src/core/` is the pure domain core: no I/O and no `tree_sitter` import. `core/mod.rs` holds the input types (functions with path, name and CC; token streams) and the integer basis-point results, `core/concentration.rs` the CC-above-10 share, `core/duplication.rs` the normalized 50-token-window share. `src/adapter/` is the tree-sitter side: `adapter/mod.rs` classifies files (Rust, non-code ignored, other code `unmeasured`) and skips `#[cfg(test)]` and `tests/`; `adapter/rust.rs` parses with `tree-sitter-rust` into the core's types (CC per function, identifier- and literal-normalized tokens). The `erosion` bin (`src/main.rs`: `git cat-file` reads and the baseline compare) comes with the profile slice, not slice 1.
**Why.** `rusty_baseline` is the nearest analog, a measurement lib plus bin under `crates/tools`. The split mirrors the design's "pure core plus a tree-sitter adapter" (erosion-gate-design §2), keeps the core testable on fixed fixtures (§10), and lets #705 add `adapter/python.rs` without touching the core. A path or module move is cheap to reverse.
**Rejected.** `crates/libs/` (no existing family fits; it is a tool run as a profile). Two crates for core and adapter (more workspace churn for no consumer). Putting it under `rusty_bbp_host` (the metric is independent of the host).
**Tag.** [default] · remind-me: mem_9d2be473f22f458a89532eac5e3b6363
**Key.** lel:704:rusty-erosion-layout
**Source.** Caller question from the #704 slice 1 build thread, 2026-11-03; erosion-gate-design §2, §9, §10; card #704 Build 1.
