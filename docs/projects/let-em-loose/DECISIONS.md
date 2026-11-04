# Let Em Loose: decisions

Each entry gives the decision, why it was made, and what was rejected. **Agreed** records the original author's report of Nano's approval; it is not independent verification or an operational authorization. **Proposed** means it is the working direction but still awaits a design or confirmation. From D46 on, entries carry a project-handoff tag instead ([handoff/DECISIONS.md](handoff/DECISIONS.md)): `[stated]` or `[agreed]` (Nano's, with an attributable reference; ask before changing) or `[default]` (an agent's choice; revise with a new entry saying why), plus the remind-me id and a key. Only the `let-em-loose-expert` subagent appends entries; recording is best effort and duplicates are reconciled by key. Dates are when the decision was made. Where an earlier entry and a later one conflict, the later one wins. The detailed BBP rules are in the claims addendum, `crates/libs/protocol/rusty_bbp/docs/spec/bbp-claims-addendum.md` (PR #1386).

## Authority, provenance and current contracts

The original author reported the design approvals below in [#1384](https://github.com/Rusty-Mill/rusty_mill/pull/1384). The external [source concept](https://claude.ai/artifact/JoCPd4weXwkJ7Zy8qMjiJJ) and Mill Board card numbers are provenance pointers, not independently verified approval records in this bundle. Where no attributable approval is linked, the reported approval remains subject to owner verification. In particular, D41–D45 originated as the author's report of #704 approval. The later D42 expiry and D44 zero-floor choices are explicitly recorded by the coordinator in [the owner-decision record](https://github.com/Rusty-Mill/rusty_mill/pull/1384#issuecomment-6614620497), including its source/confirmation reference.

Repository rules in [AGENTS.md](../../../AGENTS.md) and [CONTRIBUTING.md](../../../CONTRIBUTING.md) govern contributions. Merged BBP and orch contracts govern their implemented behavior; this concept bundle and the mutable external doc cannot supersede them. Later decisions supersede earlier concept text only within that authority boundary. This documentation correction authorizes no implementation starts, live provider calls, spending, credentials, deployment or settings changes. Separately approved implementation work retains its own authorization and review gates.

Precise contracts used by this guide:

- [BBP claims addendum](../../../crates/libs/protocol/rusty_bbp/docs/spec/bbp-claims-addendum.md): C-R4 and **Runs and candidates** for authenticated executable evidence; **Reads, delta and subscriptions** for tool-read visibility; **Failing runs block**, C-R8 and **Action marks** for human overrides and fenced state changes; **Budget** for claim allowances. These are merged contracts, rather than new permission from this guide.
- [Provider IDs, ADR-0019](../../../crates/apps/rusty_orch/docs/adr/0019-provider-ids.md): save-time conversion and rollback limits.
- [Lens core, ADR-0021](../../../crates/apps/rusty_orch/docs/adr/0021-lens-runner-core.md) and [operator assertions](../../../crates/apps/rusty_orch/crates/rusty_orch/src/lens/assertions.rs): zero-rate v0 admission and operator-controlled no-overage/host-clearance checks.
- [Inbox action planning](../../../crates/apps/rusty_mill_board/src/inbox.rs) (`plan`, `plan_task`, `Total::extend`) and [BBP CLI adapter](../../../crates/apps/rusty_mill_board/src/bbp_cli.rs): revision-fenced previews/actions, per-reason behavior and whole-task totals.

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
| D14 | Tool-using agents join through the board over MCP | Proposed 2026-11-02; decided 2026-11-04, after v0 (D60) |
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
| D43 | No erosion-specific override; budget approval is separate | Agreed 2026-11-03 (#704) |
| D44 | Scheduled cleanups pay erosion down | Agreed 2026-11-03 (#704) |
| D45 | Rust only; unmeasured code is reported, not failed | Agreed 2026-11-03 (#704) |
| D46 | Three-tier escalation for design questions | [stated] 2026-11-03 (handoff) |
| D47 | Ask the project expert before Nano | [stated] 2026-11-03 (handoff) |
| D48 | Never invent a decision; unsourced goes to OPEN_QUESTIONS | [stated] 2026-11-03 (handoff) |
| D49 | The handoff pack is scoped to the project folder | [default] 2026-11-03 (handoff) |
| D50 | One decision log, D-numbering continues | [default] 2026-11-03 (handoff) |
| D51 | The expert is the single writer of settled answers | [default] 2026-11-03 (handoff) |
| D52 | rusty_erosion location and core/adapter module split | [default] 2026-11-03 (#704) |
| D53 | One host: everything deploys on BaileyAi | [stated] 2026-11-03 (#598, #599, #594) |
| D54 | Lens shell reaches BBP through one `bbp mcp` session per lens | [default] 2026-11-04 (#594) |
| D55 | Orch-originated intents are parked | [stated] 2026-11-03 (general) |
| D56 | Windows-specific #600 behaviour is tested by a Windows shadow run and golden cases | [stated] 2026-11-03 (#600) |
| D57 | #596 adds an erosion pass rule: no worse than baseline on 4 of 5 runs | [stated] 2026-11-04 (#596) |
| D58 | #599 §6 schema and API changes approved | [stated] 2026-11-04 (#599) |
| D59 | #598 §6: task_paused, intent_revised and data classes approved | [stated] 2026-11-04 (#598) |
| D60 | Tool-using agents join over MCP after v0 ships | [stated] 2026-11-04 (general) |
| D61 | #708 alien review lens parked until v0 runs (superseded by D67) | [stated] 2026-11-04 (#708) |
| D62 | The rest of #598 §6 approved | [stated] 2026-11-04 (#598) |
| D63 | #600's Windows GUI is judged only on Windows | [stated] 2026-11-04 (#600) |
| D64 | #600 builders are Claude CLI only in v0 | [stated] 2026-11-04 (#600) |
| D65 | A Windows result that contradicts a Linux pass reaches the claim by Nano's reopen | [stated] 2026-11-04 (#600) |
| D66 | The #600 builder-session design is approved | [stated] 2026-11-04 (#600) |
| D67 | #708 alien review lens unparked (supersedes D61) | [stated] 2026-11-04 (#708) |

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
**Status.** Decided by Nano 2026-11-04: yes, but not before v0 ships. See D60, which supersedes "Proposed".

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

**Decision.** Automatic acceptance is blocked while failed profiles remain for the current candidate in the epoch; explicit human `accept` or `choose` can override this block under C-R8. Only a passing run that includes the failed profiles clears it. The host freezes the profile set when a task opens, in a host-only file, and records its hash in `task_opened`; the runner and the report check refuse any mismatch. A new candidate inherits the block until its own passing run includes those profiles. A reopened claim starts out blocked. A candidate whose diff changes or deletes listed test files cannot clear a block unless a different principal endorses the test change with cited support.
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
- **Providers in v0:** Claude, Codex and local. The hard-coded provider list in orch core becomes a config-driven registry. A new `orch-openai` crate (one crate per adapter, as orch already does) talks to llama-swap, and plain HTTP is allowed only to loopback and private addresses. Gemini joins later through the same adapter over HTTPS. Per ADR-0019, old provider variants decode to IDs without rewriting stored history; new snapshots, including a resumed goal saved again, write `Id(String)` under schema v1. Older binaries cannot read those new snapshots and report them corrupt. Rollback therefore means not resuming, or discarding, goals saved by the newer version; read compatibility is not write/rollback compatibility.
- **Lenses:** each new question gets three blind Architect proposals on three models (Claude, local, and a Codex rebel; which model is the rebel is config). Research, Adversary and Operator follow. The lens asked about a claim rotates across the eligible lenses and models, and is never the claim's author.
- **Local models:** Operator on Qwen3.6-35B-A3B; gpt-oss-120b as the local high tier for sensitive or contested claims; 32k context to start.
**Why.** The Gemini CLI was discontinued, and every existing adapter is a CLI. One OpenAI-compatible adapter covers llama-swap now and Gemini later. Rotation and author exclusion keep the old "the reviewer is never the builder" rule.
**Rejected.** A Gemini adapter in v0. A fixed lens per kind of question.

## D35. The inbox is a read view over BBP in Mill Board

**Decision.** The inbox is a read view over BBP served by Mill Board. Nothing is copied into rusty_tick, because an escalation is a decision, not a task (ADR-0012). Mill Board reads only through the host's `bbp` command, never BBP's files, so in v0 it runs on the orch host. Each card shows the claim, confidence, budget, reason, the top two supporting and contrary evidence items, and a dry run computed by the pure engine. Actions:
- **Approve:** behavior depends on the escalation reason, as `plan` and `plan_task` specify. A claim-budget approval adds one `base(c)` (stakes and reversibility included), then reopens the claim into a fresh epoch; it does not accept it or clear failed profiles. A held rerun approval adds one `base(c)` and starts that rerun. A whole-task approval adds each exhausted total's own opened allowance to its current limit, then resumes the task; it uses no claim-tier amount and accepts no claim. Totals already replenished on a partial retry are not extended again. An irreversible-action approval accepts without adding budget. An unconfirmed action's Approve records the mark's outcome as `done`.
- **Reject:** drops a claim, withdraws a held rerun, records an unconfirmed action as `aborted`, or cancels a whole task with a required reason, according to the escalation. Dropping an acted-on claim also creates a "Remediate" card carrying the dry-run diff.
- **Choose:** for a stuck question with candidates (including a taste call), Nano picks one; the chosen one is accepted and the rest dropped, in one step.
- **Refine intent:** records a human clarification linked to the displayed brief, then reopens the claim; for a paused whole task it extends exhausted totals and resumes instead. Recorded so it can become an intent version under #598.

Every preview and mutation uses the task revision read for the displayed escalation. The CLI adapter supplies `--rev` and advances the expected revision only after each committed step; stale work is refused and must be re-read/previewed. Approval concerns that exact claim/action snapshot and its displayed brief, candidates, evidence and dry-run result. It is not append-only confidence evidence or blanket permission for later actions or a refined intent. C-R8 human acceptance and C-R9 epoch-fenced action marks remain distinct from executing an external action. Formal intent versioning is still #598; this guide does not claim a new intent-version field or separate dry-run signature exists.

No push notifications in v0. Sorting uses BBP facts: irreversible first, then oldest.
**Why.** One source of truth, and Mill Board already has admin gates, MCP and a web UI.
**Rejected.** Storing escalations in rusty_tick (this reverses the earlier recommendation in QUESTIONS Q21). Approve and Reject only.

## D36. Value scheduling lives in orch, with a pre-registered rule

**Decision.** The value score, "the board asks" and fork-and-collapse all live in orch behind a `Policy` trait, with no BBP change. Fork-and-collapse only starts after #600, on reversible claims, with two sandboxed branches capped at 2 units and 30 minutes each. On a tie, the simpler or cheaper branch wins and the reason is recorded; it goes to Nano as `choose` only if the intent marks it as taste. Evaluation runs in shadow mode first, then paired runs on five intents. Value scheduling becomes the default only if (a) its holdout result is no worse on all five (at most one more failing holdout test than the baseline), (b) it uses at most 80% of the baseline's tokens on at least four of five, and (c) total escalations are no higher than the baseline.
**Why.** The score is a scheduling policy, not a board rule, so it can change without touching replayed logs. Five runs is a small sample, so quality and Nano's time are not traded for savings.
**Rejected.** Exposing the score from BBP for sorting the inbox. A rule that allows worse quality on one intent.

## D37. Holdouts, protected tests, golden sets, calibration

**Decision.**
- **Holdouts:** a second frozen profile suite, written from the intent alone by a holdout lens on a model no builder uses (config is refused otherwise). Only the holdout lens can run it, on the same base and diffs as the current candidate. For a task opened with holdouts, automatic acceptance of a claim with a candidate also needs a passed holdout run bound to the current candidate tree and suite digest; explicit human accept/choose remains the override. Builders see only profile names and pass or fail counts.
- **Protected test paths:** a candidate whose diff touches one is refused, with the paths named. A real test fix goes in as its own `tests` claim, which a different lens must support. This replaces the interim rule in D30.
- **Holdout versions:** a new version needs Nano only when an existing holdout test is removed or changed; pure additions are adopted automatically.
- **Golden sets:** in the Factory repo at `golden/<intent>/`, with a `MANIFEST` and `SIGNOFF` containing `digest`, `signer` and `date`. As **Golden sets** in the claims addendum specifies, the host checks the manifest's file hashes and the signoff's exact set digest, refusing `bad_golden` on mismatch. This is human digest-attestation: Nano's endorsement must be obtained through the human review process. CODEOWNERS routes reviews and may require them when branch protection is configured; it does not prevent another writer creating the file or authenticate its author. The host's digest check detects changed content or a stale mismatched signoff, but neither file presence nor a matching copied/forged signer label proves Nano's identity. Cryptographic signer authentication is not part of the accepted v0 contract. Private fixtures from Nano's drive are recorded as hashes only and mounted read-only. Golden cases run as `golden:` profiles inside the holdout suite.
- **Calibration:** orch-store records each accepted claim as held, reversed or unknown, at a 14-day horizon, with a hold rate per score band and nothing proposed below 20 claims per band. It is informational in v0. `rusty_orch calibrate` proposes changes that Nano approves, and they apply only to tasks opened afterwards.
**Why.** Agents that write and judge their own tests can be confidently wrong. Quietly restoring tests would hide the fact that an agent tried to change them.
**Rejected.** Restoring protected paths from base. Using track records as weights in v0.

## D38. Phase B intents and spend

**Decision.** The five Phase B intents are `dedup-lite`, `tick-lite`, `vt-grid`, `gate-board` and `frag-check`, each a cut-down version of an existing project. `dedup-lite` is a benchmark, not the #600 experiment. Acceptance lines describe observable behaviour, never method: for example, "two concurrent writers never corrupt the store and neither write is lost", not "temp file then rename". The `dedup-lite` timing target is 2 seconds on Nano's desktop, warm cache, median of 5, both arms on the same host. Taste domains: `cli-output` in tick-lite and `api-shape` in gate-board. Planning ceilings, pending separate spending authorization: $200 in total for Phase B, a $25 hard cap per run, and a pause to report if the first paired intent costs more than $40. These ceilings and BBP budget extensions authorize no billed provider use. ADR-0021 rejects any nonzero configured v0 rate; subscription calls also require the operator-controlled no-overage assertions and host admission checks linked above. A ceiling does not bypass admission, authorize a live call, or permit enabling credits/overages.
**Why.** Small, checkable intents with exact reference implementations keep a five-sample comparison meaningful.
**Rejected.** Method-prescribing acceptance lines. Swapping `vt-grid` for a fuzzier intent.

## D39. remind-me gets an explicit project parameter

**Decision.** `remind_me_add`, search and capture get an explicit `project` parameter, and the remote connector sends it. The proposed write to existing records is on hold until Nano has seen exactly which records it changes.
**Why.** The project was guessed from the caller's working folder; the remote connector has none, so it came out null.
**Rejected.** Guessing the project from other fields.

## D40. Build the slice before designing more

**Decision.** The reported build order after #597 was the #593 engine first, test-first from the claims traces, on its own PR. This is a historical sequence, not an instruction or permission to start implementation. Check current main and the separately approved per-card contracts before selecting work; #598 and #599 were not designed at this concept checkpoint.
**Why.** They don't block v0 (D23), and the slice will show which gaps actually matter.
**Rejected.** Designing every card before building any.

## D41. Gate on code erosion, not only behaviour

**Decision.** A small, pure metric, `rusty_erosion`, scores two things in basis points: how much cyclomatic complexity sits in functions above CC 10 (concentration) and how many tokens sit in repeated, normalized 50-token windows (duplication). It runs as an `erosion` profile in the task's frozen profile set and compares the candidate with the pinned baseline, not the absolute level. A candidate over the limit fails that profile, so the existing failing-test block (D30) stops acceptance. There is no BBP schema change. A claims task opens only with an erosion profile or a recorded opt-out, both covered by the profile digest.
**Why.** Every other gate checks behaviour, so a candidate could pass them all while the code got worse. SlopCodeBench (arXiv 2603.24755) found that happens in about 3 of 4 agent trajectories, and that quality prompts lower the starting point but not the rate of decay. Measuring against the baseline means existing code is not punished for mess it already had.
**Rejected.** Relying on the Architect lens prompt alone. A new BBP evidence kind for erosion.

## D42. Erosion limits come from our own history

**Decision.** The limits are calibrated from erosion deltas on rusty_mill's merged human PRs. During report-only, the profile always passes and the deltas still go in the log. Report-only lasts 2 weeks or 10 tasks, whichever comes first. At expiry, if thresholds are not approved, pause opening new affected tasks and send the calibration numbers to Nano for approval; do not extend report-only indefinitely or invent limits. Tasks already open retain their settings, frozen profile definitions and profile digests, including report-only behavior if that is what they opened with. Approved thresholds apply to newly opened tasks. The expiry behavior is the recorded owner choice linked above.
**Why.** Fixed numbers would be guesses. The repository's own human history is the fair baseline. The time box stops report-only from becoming permanent.
**Rejected.** Starting with fixed limits.

## D43. No erosion-specific override; budget approval is separate

**Decision.** v0 adds no erosion-specific override mechanism. A better candidate with a qualifying passing run can clear the failed profile. If its claim budget is exhausted, inbox budget Approve funds another attempt and reopens the claim (D35); the failed-profile block remains. Overriding the failure instead requires a separate explicit, revision-fenced human `accept` or `choose` under C-R8, with the failing profiles visible. Budget approval alone cannot do that. An erosion escalation shows the change in each metric and the three worst-affected functions.
**Why.** The existing human acceptance override needs no BBP change; it is a different action from extending a budget. The deltas and the worst functions let Nano judge without opening the diff.
**Rejected.** An `erosion` claim domain endorsed by another principal, like `tests` (D37). That stays available for v1 if overrides become common.

## D44. Scheduled cleanups pay erosion down

**Decision.** orch writes a `consolidate` intent when the cumulative erosion since the last cleanup reaches half the limit, or after 5 accepted claims, whichever comes first. A cleanup must keep the frozen suite and the holdouts (D37) green. The owner-approved cleanup predicate is `delta_concentration <= 0` and `delta_duplication <= 0`, with at least one delta strictly negative. One metric must improve and neither may worsen; requiring both deltas to be negative would be impossible at a zero floor. If both baseline metrics are already zero, skip the scheduled cleanup and record the reason rather than open an impossible task. This cleanup predicate is separate from ordinary calibrated erosion limits and does not change existing tasks' frozen profiles.

Worked examples (concentration, duplication, in basis points):

| Baseline | Candidate | Cleanup outcome |
| --- | --- | --- |
| (100, 0) | (80, 0) | Pass: concentration improves, duplication stays at zero |
| (0, 100) | (0, 80) | Pass: duplication improves, concentration stays at zero |
| (100, 100) | (80, 101) | Fail: duplication worsens |
| (100, 0) | (100, 0) | Fail: neither improves |
| (0, 0) | — | Skip cleanup; both are already zero |

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

## D53. One host: everything deploys on BaileyAi

**Decision.** The BBP host, claims runs, shadow-run tooling, the daily watcher, `orch-runner` and the `lel-codex` container all run on BaileyAi. The agentos container on the MS-01 (LXC 107) is dropped. Supersedes the card #598/#599 note of 2026-11-02 22:08Z (mem_f7c9785174124786b33031d3cc1407bc).
**Why.** Nano resolved the conflict between that card note and his 2026-11-03 voice answer by choosing BaileyAi, where llama-swap already runs.
**Rejected.** agentos on the MS-01 with BaileyAi as fallback.
**Tag.** [stated] · remind-me: mem_5862c484286b46c9a3ff25d6ff2fde7e
**Key.** lel:general:one-host-baileyai
**Source.** Nano's saved record (cap_58e514f9f3d94b3da5d26c0b8a118dd3; dialog mem_77a1f7b27e1340d8a644b8b2062b8f6d, 11-03 "Nano chose all on BaileyAi"; summary mem_eb16b0dea2394f46990c384f69a2ac41 "all deployment on BaileyAi"), pointed to in thread message cmsg_01WgyiM31RsSnZTgNuPX2VE4E2HeirtwhX9Qh6porRyDds (2026-11-04). Staged as R1 in PR #1446.

## D54. Lens shell reaches BBP through one `bbp mcp` session per lens

**Decision.** The `rusty_orch lens run` shell spawns one `bbp mcp` stdio JSON-RPC child per lens for the run (`BBP_SESSION=yes`, `BBP_PRINCIPAL=<lens id>`) and uses its claims tools: `claims_delta` to wake, `claim_card` and `claim_line` for the card and settled lines, `post` for claims and evidence. `claim_card`'s `EvidenceView` carries author, direction, kind, delta, epoch and scoring but no note, so the shell fills each evidence note from an incremental `read(after)` on the same session, cached by message id (one read per wake, not per claim).
**Why.** Design rev 3 §5 makes each lens a BBP principal with one session credential, and §6 steps 1 and 7 call `claims_delta` and post with that credential; `bbp mcp` is the only existing credentialed claims path, so this needs no BBP change and keeps the session's `read_messages` filtering. A child process per lens is cheap to replace.
**Rejected.** New finite `bbp` CLI subcommands for claims views and lens posts (new public CLI surface, ask-first). Linking `rusty_bbp` and opening `FsStore` in-process (bypasses the host's credential and visibility path). Adding `note` to `claim_card` (public API change to the addendum's card, ask-first; open question).
**Tag.** [default] · remind-me: mem_fc34b139927341aabc53a249d2427f81
**Key.** lel:594:lens-shell-bbp-transport
**Source.** Caller question from the #594 shell slice, 2026-11-04; 594-lens-runner-design.md §5, §6; bbp-claims-addendum.md § Reads, delta and subscriptions and R11; `rusty_bbp/src/claims.rs` `EvidenceView`/`ClaimView`; `rusty_bbp_host/src/mcp/claims.rs`.

## D55. Orch-originated intents are parked

**Decision.** Neither the narrowed v0 nor the full design in orch-intents-design.md is approved. D44 cleanups open as already planned: orch writes the `consolidate` intent at half the drift limit or after 5 accepted claims, with no origin header, no `rules.toml` modes, no host rule check and no `would_open` table. Revisit when a second task-opening rule appears. The implementation defaults proposed for that design (key `lel:general:orch-intents-defaults`) were never adopted and don't apply.
**Why.** Only D44 opens tasks today. #597 re-checks, #599 maintenance and #598 seeding all post into tasks that already exist, so an origin/mode mechanism would have one user.
**Rejected.** The narrowed v0 (origin only for `consolidate`, through a separate host entry with modes off|auto). The full design (general origin header, rules engine, observe mode with a `would_open` table and `rusty_orch rules report`).
**Tag.** [stated] · remind-me: mem_8727494da22f471998b4df97302c8de6
**Key.** lel:general:orch-intents-parked
**Source.** Nano chose "Neither, park it" on a decision card in thread "Discuss Let Em Loose design", 2026-11-03T22:54:11Z; mem_b6859bf1682b4b8c804edc23342f6746 (dsh-superagent review).

## D56. Windows-specific #600 behaviour is tested by a Windows shadow run and golden cases

**Decision.** Windows-specific behaviour of the #600 dedup intent (mapped drives, UNC paths) is covered by a read-only shadow run on Nano's Windows machine plus golden cases he signs off there. The Linux sandbox covers everything else. Each Windows-specific intent line must map to one of these, or it counts as untested.
**Why.** Claims runs execute only in the Linux sandbox, which can't exercise Windows path behaviour.
**Rejected.** Treating the Linux sandbox as enough for Windows paths.
**Tag.** [stated] · remind-me: mem_b33951241b4740a2b44f9cd73a09e2fe
**Key.** lel:600:windows-shadow-run-golden
**Source.** Nano's saved record (cap_58e514f9f3d94b3da5d26c0b8a118dd3; dialog mem_77a1f7b27e1340d8a644b8b2062b8f6d, 11-03 "a Windows shadow run plus golden cases for #600"; summary mem_eb16b0dea2394f46990c384f69a2ac41), pointed to in thread message cmsg_01WgyiM31RsSnZTgNuPX2VE4E2HeirtwhX9Qh6porRyDds (2026-11-04). Staged as R2 in PR #1446.

## D57. #596 adds an erosion pass rule: no worse than baseline on 4 of 5 runs

**Decision.** The #596 pre-registered decision rule gains a fourth criterion: the treatment arm's erosion delta is no worse than the baseline arm's on 4 of 5 runs. It sits beside holdout 5/5, tokens ≤80% on ≥4/5 and escalations ≤ baseline (erosion-gate-design §6).
**Why.** Value scheduling must not win by writing messier code. Pre-registering it before Phase B runs keeps the comparison honest.
**Rejected.** Leaving erosion out of the #596 rule.
**Tag.** [stated] · remind-me: mem_bff2870445d746e7b1b11f34cd9bb88e
**Key.** lel:596:erosion-criterion
**Source.** Nano, thread message cmsg_01WgyiM31RsSnZTgNuPX2VE4UtGWmEuydpHpmL6uyyvn7t (2026-11-04T11:58:44Z), answering the numbered list in cmsg_01WgyiM31RsSnZTgNuPX2VE48CSJZ2CFK5Xom7dXjiyN1X, item 1 "Yes". OPEN_QUESTIONS Q2.

## D58. #599 §6 schema and API changes approved

**Decision.** The changes listed in 599-gaps-design.md §6 are approved: BBP `claim_history` and its `Step` type (read-only, no log change); `bbp why --claim/--at/--task`; a `canary` run on a released ref; `rusty_orch lessons`, `rusty_orch watch` and `rusty_orch lens why`; orch-store tables `lesson_written`, `lens_record` (replacing #594's unimplemented counts) and `watch_finding`; the `Watch` port with `cargo-deny`, `cargo-update`, `toolchain` and `canary` adapters (CLI calls, no new crates); Factory `apps/*.json`, the watched-versions file and `watch_usd`. remind-me needs no change, but check before building that its HTTP add route carries `project`; an additive fix there is flagged then.
**Why.** All additive and dependency-free; this clears the ask-first schema/API gate for #599.
**Rejected.** Holding §6 for further review.
**Tag.** [stated] · remind-me: mem_69ede4b656794a0d8301818a68ce1957
**Key.** lel:599:section6-approved
**Source.** Nano, thread message cmsg_01WgyiM31RsSnZTgNuPX2VE4UtGWmEuydpHpmL6uyyvn7t (2026-11-04T11:58:44Z), answering the numbered list in cmsg_01WgyiM31RsSnZTgNuPX2VE48CSJZ2CFK5Xom7dXjiyN1X, item 2 "Yes" (the question: "approve the new commands, tables and `Watch` port listed in its §6"). OPEN_QUESTIONS Q3.

## D59. #598 §6: task_paused, intent_revised and data classes approved

**Decision.** Approved from 598-gaps-design.md §6, as named in the question: the host write `task_paused { reason, detail }`, which moves the task to `escalated`; the human operation `intent_revised { brief, affects }` with the `intent_version` fold (omitted `affects` means `all`, an empty one is refused `bad_affects`); data-class labels, where `internal` may go to cloud models and `sensitive` stays local. The other §6 items weren't in the question; Nano approved them later the same day (D62).
**Why.** Schema and API changes are ask-first, so approval covers only what Nano was shown.
**Rejected.** Reading the "Yes" as approval of all of §6.
**Tag.** [stated] · remind-me: mem_eddc237646b843bd8069a0e7a86047a6
**Key.** lel:598:section6-named-items-approved
**Source.** Nano, thread message cmsg_01WgyiM31RsSnZTgNuPX2VE4UtGWmEuydpHpmL6uyyvn7t (2026-11-04T11:58:44Z), answering the numbered list in cmsg_01WgyiM31RsSnZTgNuPX2VE48CSJZ2CFK5Xom7dXjiyN1X, item 4 "Yes". OPEN_QUESTIONS Q5.

## D60. Tool-using agents join over MCP after v0 ships

**Decision.** D14 is decided: tool-using agents (Claude Code, Codex) join the board through MCP, but not before v0 ships. This supersedes D14's "Proposed" status. How it fits the D20 executor stays open.
**Why.** They don't fit a single request and reply (D14), and v0 ships first on the lens port.
**Rejected.** Adding the MCP path before v0.
**Tag.** [stated] · remind-me: mem_c6c6304576114e5fa962e49e720e081e
**Key.** lel:general:mcp-agents-after-v0
**Source.** Nano, thread message cmsg_01WgyiM31RsSnZTgNuPX2VE4UtGWmEuydpHpmL6uyyvn7t (2026-11-04T11:58:44Z), answering the numbered list in cmsg_01WgyiM31RsSnZTgNuPX2VE48CSJZ2CFK5Xom7dXjiyN1X, item 5 "Yes" (to "yes, but not before v0 ships"). OPEN_QUESTIONS Q6.

## D61. #708 alien review lens parked until v0 runs

**Decision.** The #708 alien review lens is parked until v0 runs: trigger policy, per-language defect lists, scoring and how reproduced findings enter BBP stay undesigned and unapproved, and nothing is built.
**Why.** Get v0 running before adding another lens kind.
**Rejected.** Designing #708 now.
**Tag.** [stated] · remind-me: mem_44c8bf81957240d8a0773acb5dc63f7f
**Key.** lel:708:alien-lens-parked
**Superseded.** By D67 (2026-11-04T20:16Z): Nano unparked #708.
**Source.** Nano, thread message cmsg_01WgyiM31RsSnZTgNuPX2VE4UtGWmEuydpHpmL6uyyvn7t (2026-11-04T11:58:44Z), answering the numbered list in cmsg_01WgyiM31RsSnZTgNuPX2VE48CSJZ2CFK5Xom7dXjiyN1X, item 7 "Park it until v0 runs". OPEN_QUESTIONS Q8.

## D62. The rest of #598 §6 approved

**Decision.** Every remaining item in 598-gaps-design.md §6 is approved as listed, completing D59. BBP: `task_opened.baseline_digest` (optional); `claim_reopened.by: intent`; the escalation reason `intent_changed`. Host: `bbp open --base` runs the baseline and can refuse `baseline_red`; the `base_test_changed` refusal; named sandbox profiles for lenses and builders, not just runs; the filtered view; the `shadow:` profile kind with read-only `real_data` mounts and a plan artifact. orch: the Survey and Intent lens kinds; `max_class` and `price` on providers, both required at load; orch-store tables for item classes, spend and reservations; `rusty_orch lens breaker`; `domain_classes` replacing #594's `sensitive_domains`. Factory: `domain_classes`, `sensitive_paths`, `real_data`, `price`, the ceilings and the breaker settings. No migration: no claims-mode logs exist outside tests.
**Why.** All additive, and they implement answers Nano already gave on #598 (Q1, Q4, Q6). This clears the ask-first schema/API gate for the #598 build.
**Rejected.** Going item by item (option B on the card).
**Tag.** [stated] · remind-me: mem_fc5ad74f9fac448d8d22a9ff42f12bc3
**Key.** lel:598:section6-remaining-approved
**Source.** Nano tapped option A "Approve all as listed" on decision card cmsg_01WgyiM31RsSnZTgNuPX2VE4Ppm26TJdNSuETWT9FqZp8E ("Approve the rest of #598 §6, not just the three I listed?"), 2026-11-04T17:15:20Z. OPEN_QUESTIONS Q5.

## D63. #600's Windows GUI is judged only on Windows

**Decision.** The #600 desktop GUI (Windows-first) is judged only through the Windows shadow run and golden cases (D56). Linux claims runs check the engine and the web UI.
**Why.** Claims runs are Linux-only and can't exercise a Windows GUI.
**Rejected.** Judging the GUI in the Linux sandbox.
**Tag.** [stated] · remind-me: mem_0793274b6343443fbbe7f34d9846e0bb
**Key.** lel:600:windows-gui-judged-on-windows
**Source.** Nano, thread message cmsg_01WgyiM31RsSnZTgNuPX2VE4MwQzj2Aawhe7Q5axWeGqTY (2026-11-04T17:16:45Z), "01 Yes. 02. Concur. 03. Concur with recommendation", answering cmsg_01WgyiM31RsSnZTgNuPX2VE497rAQtVYv9PGh6V7rgu8pq and 600-builder-session-design.md §11, O1 "Yes".

## D64. #600 builders are Claude CLI only in v0

**Decision.** In v0, #600 builder sessions run the Claude CLI only. Codex in `lel-codex` doesn't build.
**Why.** The holdout lens then keeps a model that no builder uses (D37), and isn't left with only local models.
**Rejected.** Codex as a second builder in v0.
**Tag.** [stated] · remind-me: mem_44f3236c741d4f3383370ac59299d664
**Key.** lel:600:claude-only-builders
**Source.** Nano, thread message cmsg_01WgyiM31RsSnZTgNuPX2VE4MwQzj2Aawhe7Q5axWeGqTY (2026-11-04T17:16:45Z), "01 Yes. 02. Concur. 03. Concur with recommendation", answering cmsg_01WgyiM31RsSnZTgNuPX2VE497rAQtVYv9PGh6V7rgu8pq and 600-builder-session-design.md §11, O2 "Concur".

## D65. A Windows result that contradicts a Linux pass reaches the claim by Nano's reopen

**Decision.** When the Windows shadow run contradicts a Linux pass, Nano reopens the claim with a human `decision reopen` (C-R8), after `budget_extended` only if the claim's budget is exhausted (claims_engine.rs, cl_48). It isn't posted as `executed` evidence, since it has no runner report bound to the claim's candidate (C-R4).
**Why.** A real-data failure should stop acceptance, and only a reopen can do that. `cited` evidence never reopens a claim.
**Rejected.** Posting it as `cited` evidence.
**Tag.** [stated] · remind-me: mem_ed23f39443c2424d96b224556a54303f
**Key.** lel:600:windows-contradiction-human-reopen
**Source.** Nano, thread message cmsg_01WgyiM31RsSnZTgNuPX2VE4MwQzj2Aawhe7Q5axWeGqTY (2026-11-04T17:16:45Z), "01 Yes. 02. Concur. 03. Concur with recommendation", answering cmsg_01WgyiM31RsSnZTgNuPX2VE497rAQtVYv9PGh6V7rgu8pq and 600-builder-session-design.md §11, O3 "Concur with recommendation" (option a).

## D66. The #600 builder-session design is approved

**Decision.** Nano approved the #600 builder-session design (`/mnt/project-files/let-em-loose/600-builder-session-design.md`) as a whole, together with D63–D65. Its §8 changes count as approved: the orch `Builder` job kind and its admission check; the config values `builders_per_claim`, `max_live_builders` and `builder_wall_secs`; the orch-store `builder_session` table; the open-time `untested` check for Windows intent lines; the orch-bbp builder launch profile with a pre-fetched dependency mirror; and a new local-only repo `dedup-manager` on BaileyAi. The #598 items it depends on are approved in D62. The §10 defaults (local-only repo, 45-minute wall clock, dependency mirror) stay `[default]`. #600 is built once #594 and #595 are in place.
**Why.** Design first on every card (D25). This closes OPEN_QUESTIONS Q7.
**Rejected.** Further changes before approval (the card's other option).
**Tag.** [stated] · remind-me: mem_c00104fb2fa0499c8f471bddcbb64cbc
**Key.** lel:600:builder-session-design-approved
**Source.** Nano tapped option A "Approve the design" on decision card cmsg_01WgyiM31RsSnZTgNuPX2VE4UkTT9YFWDhx1CsEYPh6YFg ("Approve the #600 builder-session design as a whole, with your three answers?"), 2026-11-04T18:08:52Z.

## D67. #708 alien review lens unparked (supersedes D61)

**Decision.** #708 is unparked, replacing D61. Next: finish the design (the value threshold on #596's score, per-language defect lists, scoring, and how reproduced findings enter BBP), run the egm Goldfish review, and bring the design to Nano for approval. Build and open a PR for dottiusmaximus only after he approves it. The design is not approved yet (D25).
**Why.** Nano chose this when asked whether his 20:11Z message "Spin off threads for each one with a design or partial design" (cmsg_014AqbZs58L565hMV2pFkCX59ZUnUW7emMuJZYMRh7Zqw9) covered #708.
**Rejected.** Keeping it parked until v0 runs (D61), and designing and reviewing it without building until v0 runs.
**Tag.** [stated] · remind-me: mem_80f2a194cf714d4586d27184e3522cc9 (supersedes mem_44c8bf81957240d8a0773acb5dc63f7f)
**Key.** lel:708:alien-lens-unparked
**Source.** Nano tapped option B "Unpark: design, review, then build" on decision card cmsg_014AqbZs58L565hMV2pFkCX59b5EzthujQuUWFyYCJrHdB ("Unpark the alien review lens (#708), which you parked this morning?"), 2026-11-04T20:16:21Z, thread cmsg_014AqbZs58L565hMV2pFkCX5PRArQZK27dSqRZa2ZYPeC1. Reopens OPEN_QUESTIONS Q8.
