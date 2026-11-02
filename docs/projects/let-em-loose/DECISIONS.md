# Let Em Loose: decisions

Each entry gives the decision, why it was made, and what was rejected. **Agreed** means Nano has approved it. **Proposed** means it is the working direction but still awaits a design or confirmation. Dates are when the decision was made.

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
| D12 | orch is model-independent, via ports and adapters | Agreed 2026-11-02 |
| D13 | No agent-to-agent calls | Agreed 2026-11-02 |
| D14 | Tool-using agents join through the board over MCP | Proposed 2026-11-02 |
| D15 | Concurrent, event-driven evidence posting | Agreed 2026-11-02 |
| D16 | Value-driven scheduling is a shadow experiment | Agreed 2026-11-02 |
| D17 | Build in the rusty_mill monorepo | Agreed 2026-11-02 |
| D18 | The dedup experiment gets a fresh repo and intent only | Agreed 2026-11-01 |
| D19 | The Factory repo holds process and config only | Agreed 2026-11-02 |
| D20 | No fourth executor | Agreed 2026-11-02; which executor is still open |
| D21 | Thin slice first, then let it build itself | Agreed 2026-11-01 |
| D22 | Ground truth sits outside the agents' loop | Proposed 2026-11-02 (#597) |
| D23 | Defer existing code and the smaller gaps | Agreed 2026-11-02 |
| D24 | Audit remind-me before building | Agreed 2026-11-01 |
| D25 | Design first on every card | Agreed 2026-11-02 |
| D26 | The name is Let Em Loose | Agreed 2026-11-02 |

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
**Open.** Which executor. #594 starts with a reconnaissance report, and Nano decides.

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
