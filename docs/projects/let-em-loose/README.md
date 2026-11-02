# Let Em Loose - Beyond the Software Factory

> Status: concept, 2026-11-02. Source of truth while the design settles: the [live doc](https://claude.ai/artifact/JoCPd4weXwkJ7Zy8qMjiJJ). Work is tracked on Mill Board cards #592 to #599, tagged `let-em-loose`.

A working proposal for agentic development that isn't modelled on the assembly line.

## The problem

The factory model (planner → coder → tester → reviewer) is a human org chart. It exists because one person couldn't hold everything at once, and handoffs let specialists catch each other's blind spots. Agents don't share those limits, so copying the structure imports its overhead: queues, handoffs, context loss, and stage-gated latency.

What's worth keeping is not the stages but the *perspectives* and the *verification*.

## Core shift: lenses, not stages

- **One shared artefact.** Intent, code, tests and evidence live together and evolve together. Nothing is "handed off."
- **Roles become critics.** Planner, tester, reviewer, security, performance are lenses that attack the artefact concurrently, each asking its own adversarial question.
- **Convergence, not completion.** Work is done when no lens can find a problem it cares about, not when the last stage signs off.
- **Search, not pipeline.** Where cheap, generate several candidates in parallel and let evidence choose.
- **Evidence, not approval.** Ship because properties pass, behaviour holds in a sandbox against realistic inputs, and no regressions appear.

### Lens questions

| Lens | Question it asks |
| --- | --- |
| Intent | Does this do what was meant, and nothing it must not? |
| Adversary (tester) | How does this break? |
| Reviewer | Is this simple, coherent and maintainable? |
| Security | How could this be abused? |
| Operator | How will this fail in production, and will we know? |

## Capturing intent better

Intent is the real artefact; code is regenerable. The hard part is that intent is usually tacit. Ways to surface it:

1. **Examples before rules.** Ask for concrete good and bad cases first; generalise into properties afterwards.
2. **Invariants and never-statements.** Explicitly capture what must always hold and what must never happen. These are the most durable part of intent.
3. **Interrogation loop.** An agent restates the intent in its own words and proposes edge cases; the human corrects. Misunderstandings show up as wrong examples, which are cheap to spot.
4. **Rationale with every decision.** Record *why*, not just *what*, so later agents can judge new cases the human never anticipated.
5. **Executable form.** Every intent statement should become a test, property, or check where possible. Unverifiable intent is a wish.
6. **Taste exemplars.** Point to code or designs that feel right, and say why. Taste transmits better by example than by rule.
7. **Living intent.** When a lens finds a gap the intent didn't cover, the fix goes into the intent first, then the code.

## Applying this to the expert agents work

- Keep the base roles, but run them as concurrent critics over one artefact rather than sequential workers.
- Make the intent document the hub every lens reads from and writes back to.
- Replace "stage complete" signals with "no open objections from any lens."
- Measure success by how rarely a human has to restate intent.

## Keeping dissent honest

Early, continuous involvement of reviewer and tester only helps if their disagreement stays real. Agents sharing a model and a conversation drift toward agreement. Four counters:

1. **Model diversity.** Run adversarial lenses on different models, so dissent comes from different priors, not one model in several hats.
2. **Blind first pass.** Each lens forms its view independently before seeing the others', then views are compared.
3. **Agreement costs evidence.** "Looks good" is not accepted; a lens must cite the test, property or measurement that convinced it. Objections stay cheap.
4. **Reward finding problems.** A lens that finds nothing is a signal to look harder, not a success.

The human is pulled in only for irreversible actions, budget limits, or deadlocks the evidence can't settle.

## When do planning, development and testing happen?

They happen together, continuously, at every scale. They are activities, not phases.

- **Within one claim:** proposing it is planning; an agent drafts code for it (development) while the adversary writes a test to break it (testing), often before the code exists. The loop takes minutes, then the next claim starts.
- **At larger scales:** app-level claims such as "one shared index" are coarse-grained planning, proven or disproven the same way, only more slowly.
- **The only real ordering:** intent comes first, because nothing can be judged without it; irreversible actions come last, because they need the most evidence. Everything between runs simultaneously.

## Converging without conversation

Prose debate is a human limitation. Agents converge on structured claims on the board instead, a live, auditable form of Bayesian updating.

1. **Claims, not messages.** A claim is a small proposition with a confidence, its evidence, and its dependencies. It starts neutral.
2. **Evidence moves confidence.** Agents attach support or counterexamples, each with a direction and strength. The board, not the agents, computes the update.
3. **Weighted evidence.** Executable proof (tests, benchmarks) outweighs opinion. Agents carry a track record; well-calibrated agents get more weight.
4. **Linked claims.** If a foundation claim collapses, dependent claims drop with it.
5. **Thresholds decide.** Above ~90% accept and proceed; below ~10% drop; stuck in between or out of budget, escalate to the human.
6. **Readable for audit.** Claims stay human-readable, not for chatting but so decisions can be traced.

## Beyond loop versus event

Loop-based and event-driven scheduling both only answer when an agent may act. Three further mechanisms, used together, decide what is worth doing next. Events remain the plumbing underneath.

1. **Value scoring.** Each claim carries a worth-resolving score, roughly uncertainty times stakes, divided by the cost to check. Effort is pulled to the highest-value claims.
2. **The board asks.** Control inverts: the board directs targeted questions about top-scoring claims to the lens best placed to answer. Agents act as instruments rather than opinion-givers, which also counters head-nodding.
3. **Fork and collapse.** When a claim is genuinely split and branching is cheaper than arguing, fork the artefact, run both briefly, and keep what the evidence favours.

This is an experiment: run it in shadow mode against plain event-driven scheduling and compare tokens spent, time to settle, and decision quality.

### Walkthrough: what happens after intent is handed over

Using the duplicate manager as the example. The board behaves like a detective choosing which question to ask next.

1. **Seeding.** A research lens and a builder turn the intent into open claims. Each never-rule becomes a claim that must be proven ("no file ever loses its last copy"); design ideas become candidate claims ("size check, then partial hash"). All start near 50%.
2. **Scoring.** The board scores every claim by uncertainty, stakes, and cost to check. "How do we spot duplicates fast?" scores high; "what colour is the GUI?" scores low.
3. **The board asks.** Top claims become specific questions routed to the best lens: "Research: fastest known way to rule out non-duplicates?" "Adversary: break the partial-hash idea with a file that changes mid-read." Independent questions run in parallel. No one is asked for a general opinion.
4. **Evidence shifts scores.** Answers post as evidence; settled claims drop toward zero and fade; new questions surface; attention moves to whatever is now most worth resolving.
5. **Stuck claims fork.** If two options (say an embedded database and an append-only log) both sit near 50%, the board builds both, the operator crashes each mid-scan, and evidence picks the winner.
6. **It stops on its own.** When nothing left is worth the cost of resolving, the work is done. If the top remaining claim is irreversible, it goes to Nano's inbox instead.

Throughout, nothing waits for a turn: events carry the answers, and the score decides what is asked next.

### Avoiding groupthink on open questions

If one lens answers and everyone else attacks, everything anchors on the first idea. For open questions the board instead:

1. **Asks several models the same question, blind,** so it gets several independent candidates.
2. **Requires a rebel:** at least one candidate must deliberately depart from the known approach.
3. **Treats a single answer as a warning:** a question with one candidate counts as more uncertain, keeping its score high until alternatives exist.

Only then do attacks begin, aimed at all candidates equally.

### Context: the board is the memory

Agents do not carry long conversations. For each question, orch assembles a fresh context:

1. A stable prefix: intent and rules (cached, nearly free).
2. The specific claim, its evidence, and its dependencies.
3. Relevant settled claims, compacted to one line each.
4. The lens's own track record.

The agent answers, posts evidence, and forgets. This avoids context rot, prevents agents anchoring on their own earlier views, and makes blind first passes trivial to enforce. Builders may keep a short working session within one task, but the code and results live on the board. Lessons across projects live in rusty-remind-me.

## Budget as the price of certainty

Tokens remain the unit, but allocation follows the decision, not a flat cap per task.

- **Scale by stakes and reversibility.** Cheap, reversible calls get a small allowance and a low bar; costly, one-way calls get a large allowance and a high bar.
- **Stop when settled.** If recent rounds of evidence barely move confidence, stop and act; don't spend the budget just because it exists.

## Saving tokens

1. **Read deltas.** Each agent sees only what changed since its last look, plus claims it subscribes to.
2. **Wake only relevant agents.** Lenses subscribe to claims in their domain and sleep otherwise.
3. **Tier the models.** Cheap models handle routine evidence; expensive models only for contested or high-stakes claims.
4. **Let tests argue.** Running a test costs compute, not tokens, and settles faster than debate.
5. **Compact settled claims.** Collapse a decided claim's history to one line plus a link to its evidence.
6. **Stable prefixes.** Put intent and rules first in every prompt so prompt caching makes them nearly free.

Waking only relevant agents and tiering models usually give the biggest savings.

## Biggest gap: ground truth

Most evidence is produced and judged by agents, so the board can be confidently wrong.

- **Who checks the checkers.** Builders and testers can share blind spots and produce code and tests that agree with each other and are both wrong. Model diversity helps less than hoped, since models share much of their training data.
- **Uncalibrated confidence.** A 90% threshold means nothing unless scores are checked against reality. A false 92% is worse than an honest 50%.

Fixes:

1. **Holdout tests,** written from intent by a lens the builders never see.
2. **A golden set** of real cases with known answers, signed off once by Nano.
3. **Calibration tracking:** check over time that claims accepted at about 90% hold about 90% of the time, and correct the update rule when they don't.

Second gap: intent capture is untested, and the whole process leans on it. The dedup experiment is the first real test. Tracked on card #597 (bbp_factory).

## Further gaps

Tracked on card #598 (bbp_factory).

1. **Existing code (the biggest).** The design assumes greenfield. Real work mostly changes existing code, so the current codebase must enter as evidence: claims seeded from what it already does, and "don't break existing behaviour" as an inherited never-rule.
2. **Sandboxing and data boundaries.** Agents run and deliberately crash code; nothing touches real files before the shadow run. Each adapter declares what data it may receive, so sensitive code stays on the local model.
3. **Intent versioning.** Each claim records the intent version it was proven against; an intent change reopens exactly the affected claims.
4. **Runaway cost.** Parallel candidates, forks and multiple models multiply spend. A global ceiling plus a circuit breaker trips when spend rises while confidence stays flat.

### Smaller gaps

None block the first experiment. Tracked on card #599 (bbp_factory).

1. **Learning across projects.** Something must decide what counts as a lesson and turn it into a reusable claim the next project's research lens can find.
2. **Gameable track records.** Reward lenses for being right on hard, contested claims, not for safe bets.
3. **Maintenance after shipping.** Watch shipped apps for dependency, platform and security changes and raise them as claims.
4. **Decision replay.** Replay which evidence moved which claim, and when, for debugging and audit.

## Open risk

This approach moves difficulty into intent capture. If intent is vague, concurrent critics will argue forever or converge on the wrong thing. The intent-capture practices above are the load-bearing part.

## Where this leads

The long-term shape is less a factory, more a garden you tend.

- **Retire files, pull requests and releases.** Agents work on a living semantic graph of behaviour, intent and evidence; code is rendered when a human wants to read it. Changes flow continuously, each proven in shadow against real traffic before it takes over.
- **Make proof cheap.** Formal verification, once too costly for humans, becomes routine for critical invariants.
- **Close the loop with reality.** Production behaviour and user friction feed back as new intent; the system proposes its own next changes and the human chooses which are worth it.
- **Personal software.** Generated to fit each user or team rather than one-size-fits-all.

## Worked example: rusty_fclone

A hypothetical run of this model on one feature: let rusty_fclone reclaim space by replacing duplicates with hardlinks.

### 1. Nano writes intent (about 10 minutes)

- **Goal:** reclaim disk space from duplicate files.
- **Always:** two files are only duplicates if their full contents match byte for byte.
- **Never:** lose the last copy of any file; change any file's contents; follow links outside the scanned root.
- **Examples:** two identical photos in different folders are merged; two files with the same size and name but different bytes are left alone.
- **Reversibility:** anything that removes data is one-way, so it needs Nano.

### 2. Lenses wake and work blind

Each lens reads only the intent and forms its view before seeing the others.

| Lens | Model | First contribution |
| --- | --- | --- |
| Intent | Claude | Turns the never-statements into executable property tests |
| Adversary | Codex | Proposes a file that changes while being hashed |
| Security | Gemini | Flags symlinks pointing outside the root |
| Operator | Small local model | Asks: what happens on a drive that doesn't support hardlinks? |
| Builder | Claude | Drafts two candidate designs in parallel |

### 3. Claims form on the board

- **C1:** compare by size, then a partial hash, then a full hash. Starts at 50%.
- **C2:** replace duplicates with hardlinks. Depends on C1. Starts at 50%.

### 4. Evidence moves confidence

1. A benchmark shows C1 is 8x faster than full-hashing everything. **C1 rises to 75%.**
2. The property tests pass on 10,000 generated file trees. **C1 rises to 92%: accepted.**
3. The operator lens shows hardlinks fail across drives and on some network shares. **C2 drops to 40%.**
4. The builder revises: hardlink within one volume, otherwise just report. Its tests pass, including the adversary's changing-file case. **C2 rises to 91%.**
5. The last two evidence rounds barely moved anything, so the board stops spending tokens.

### 5. The one interruption

C2 is accepted, but replacing files is irreversible, so the board escalates to Nano with one short card: the decision, the confidence, the evidence links, and a dry run listing exactly what would change. Nano reads one screen and decides, instead of reviewing four pull requests.

### 6. Ship and learn

The feature runs in shadow first, reporting what it would do. Real use shows people want an undo log, which arrives back on the board as new intent, not as a ticket.

### What Nano did

Wrote the intent, made one irreversible call, and read the dry run. Everything else, including the arguing, happened without him.

## User interface

*[Mockup: UI mockup: intent, claims map, decision inbox. Interactive version in the [source doc](https://claude.ai/artifact/JoCPd4weXwkJ7Zy8qMjiJJ).]*

Nano works in the intent panel. The claims map converges on its own, with settled claims fading. The inbox holds only escalations.

## Application walkthrough: a 20 TB duplicate manager

Most discussions stop at a single capability. People want complete applications, so this follows one from idea to daily use.

**The ask:** "I have a 20 TB drive with duplicates everywhere. I need an app with a GUI that is fast and smart."

### Phase 1: Intent interview (~15 minutes of Nano's time)

An agent interviews by example rather than by requirements questionnaire. Answers become product-level intent that every feature inherits.

| Kind | Example statements |
| --- | --- |
| Goal | Find and reclaim duplicate space across a 20 TB drive |
| Targets | Scan completes in an acceptable time on Nano's hardware; GUI stays responsive during scans |
| Always | Duplicates require a full content match; every destructive action has a dry run first |
| Never | Lose the last copy of a file; lose more than a few minutes of scan work on a crash |
| Examples | Same name, different bytes: leave alone. Twin photo folders: offer to merge |
| Taste | Point to tools whose feel Nano likes |

### Phase 2: Competing walking skeletons

Several agents on different models each build a thin, end-to-end prototype (scan, hash, store, show) with a different architecture, for example a Rust core with a native GUI, a Rust core with a web UI, or a design built around a persistent index. All run against a synthetic workload shaped like the real drive. Evidence picks the base, not a design review.

### Phase 3: The application emerges under attack

The chunks a human would whiteboard up front still appear, but as claims tested against the skeleton:

| Area | Claim the agents test | Lens that attacks it |
| --- | --- | --- |
| Scanning | Parallel directory walk saturates the disk without starving the GUI | Operator, UX |
| Hashing | Size, then partial hash, then full hash | Adversary (files changing mid-hash) |
| Storage | Embedded index (e.g. SQLite) vs append-only log | Operator (crash, resume speed) |
| Resilience | Checkpointing makes any crash resumable | Operator (kills the process mid-run) |
| Actions | Hardlink, move to quarantine, or delete, each with dry run | Intent, Security |
| GUI | Results grouped so 20 TB of findings is navigable | UX (uses it as Nano would) |
| Coherence | Features share one scanner and one index | Architecture, Coherence |

Nano's questions ("do we need a database?", "what if it craps out midway?") split two ways: what is acceptable is intent he states; how to achieve it is a design claim the agents prove.

### Phase 4: The few interruptions

1. Taste: choose between near-equal GUI directions.
2. Irreversible: approve the first action that changes files.
3. Reality check: run on the real drive in shadow mode, which reports what it would do.
4. Deadlock: only if the evidence cannot settle a claim.

### Phase 5: Living application

Usage feeds back as intent: folders Nano always skips become default exclusions, repeated manual choices become suggested rules. New features enter as claims under the existing product intent, so they cannot quietly break promises the app already makes.

### Capability vs application

|  | Capability view | Application view |
| --- | --- | --- |
| Intent | One feature's rules | Product intent inherited by every feature |
| Claims map | Flat | Zoomable: app-level claims over feature claims |
| Lenses | Feature lenses | Plus architecture and coherence lenses |
| Nano's role | Approve one call | Own the top-level intent and a handful of calls |

### Full-application interface

*[Mockup: Full-application UI mockup: 20 TB duplicate manager. Interactive version in the [source doc](https://claude.ai/artifact/JoCPd4weXwkJ7Zy8qMjiJJ).]*

Product intent governs every feature. The claims map starts at app level and zooms into each feature. The inbox holds only taste calls and irreversible actions.

## Research: learn, don't inherit

Every application gets a research step, so lessons already learned are leveraged. Prior art is evidence and a baseline, never a template.

- **Research phase.** Between the intent interview and the competing skeletons, agents survey known algorithms, existing tools, published benchmarks, and failure stories. Findings enter the board as sourced claims.
- **Research lens.** Stays active throughout; when a claim is contested it asks whether someone has already solved this, often settling it faster than another test round.
- **Learn, don't follow.** Existing tools set the bar to beat and the pitfalls to avoid. They do not set the design. Copying code is out.
- **A first-principles candidate.** At least one skeleton must deliberately depart from prior art and ask whether there is a different, faster, more effective way. It competes on the same evidence as the conventional candidates.
- **The parallel.** This is the same move the whole document makes: we know what software development looks like, but not what it looks like agentically. Likewise we know what deduplication looks like, but not necessarily its best form.

For the rusty_fclone experiment: agents may study existing deduplicators such as fclones, but get only the intent, never the rusty_fclone repo, and the existing tool serves as the control for comparison.

## Building it

Most pieces already exist; this is assembly plus two new parts. Nano agreed the ordering below.

| Piece | Role in the new process | Change needed |
| --- | --- | --- |
| Rusty BBP | The board | Claims with confidence, evidence and dependencies instead of messages |
| Hermes and JARVIS | Runtime: wake agents, model diversity, budgets | Wake by claim subscription; stakes-scaled budgets |
| Expert Agents | The lenses | Recast generic experts as critics, plus a research lens |
| Mill Board or rusty_tick | The decision inbox | Slim view of escalations only |
| rusty-remind-me | Lessons learned across projects | Active-project association and filtering (below) |
| New | Intent format | Goals, always, never, examples, taste |
| New | UI | Intent panel, claims map, inbox, as mocked up |

### Order

1. Claims model in BBP.
2. Two or three lenses running on different models.
3. A bare decision inbox.
4. First real job: the dedup experiment, run from intent only.
5. Once the thin version works, use it to build the rest of itself.

### Required change to rusty-remind-me

Memories need an association with an active project, so agents can filter for what is currently being worked on and tell it apart from historical or unrelated memories. For example, a project link plus an active or inactive status that searches can filter on.

Tracked on Mill Board as card #592 (rusty-remind-me project): audit the existing `project` field and `remind_me_project` tool for active-project filtering before building anything new.

Step 1, the BBP claims model, is tracked on Mill Board as card #593 (bbp_factory project): spec addendum adding claims, evidence, dependencies, thresholds, settling and stakes-scaled budgets, keeping roles in orch.

### Build cards

| Step | Card | Project | Depends on |
| --- | --- | --- | --- |
| 1. BBP claims model | #593 | bbp_factory | none |
| 2. Lens runner, 2-3 lenses on different models | #594 | bbp_factory | #593 |
| 3. Decision inbox, escalations only | #595 | mill_board | #593 |
| remind-me active-project audit | #592 | rusty-remind-me | none |

Experiment card: #596 (bbp_factory), value-driven scheduling beyond loop versus event; depends on #593 and #594.
