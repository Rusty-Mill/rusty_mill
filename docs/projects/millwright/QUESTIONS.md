# Millwright: open questions for Nano

Every question from the 2026-11-10 audits that needs Nano's answer, grouped by system,
each with a recommendation. An answered question moves to [DECISIONS.md](DECISIONS.md)
with the date and where he said it, and its row is removed here. Nothing below has been
acted on.

## Answer these seven first

1. **L1 and L2:** commit Let Em Loose's first milestone and authorise its deployment. This gets a smoke run, the first time the system runs end to end.
2. **M1:** approve the minimal role fix on Mill Board. Today any builder agent can pass every gate.
3. **O2:** give Claude its own GitHub identity. Cross-family review cannot be enforced without it.
4. **P1:** decide Portfolio Intelligence's shorter MVP path. Nothing there is dispatchable until then.
5. **R1:** approve the remind-me order of work. The dormancy fix protects a store in daily use.
6. **X1 to X3:** close the three experiments against their written conclusions.
7. **E2:** start the backlog triage pass, beginning with the 113 cards of the paused projects.

## Let Em Loose

These are also tracked by `let-em-loose-expert`; its pack is the authority for that project.

| # | Question | Recommendation |
| --- | --- | --- |
| L1 | Commit the first milestone ("ready for a live run": host, breaker wiring, deployment, lens prompts, spend readiness, smoke run)? All six cards are in the backlog. | Yes. Move them to Ready. |
| L2 | Authorise deploying the protocol host, orch and the inbox, and confirm the host? The record says BaileyAi; one card proposes probing MS-01 first. | Yes, on BaileyAi. Skip the probe unless BaileyAi lacks headroom. |
| L3 | Is the smoke run (one toy claim, local model, no builders, no spend) the first target, ahead of the full experiment? | Yes. |
| L4 | Approve the first live run and its spend ceiling? Today's rule is flat-rate subscriptions and local models only. | Flat-rate and local only for the first run; revisit paid calls after it. |
| L5 | Which credential lets the Claude CLI work inside the builder jail, which is meant to hold no secrets? This blocks the builder work. | Nano's call. Simplest: a subscription login on a dedicated account used only by this jail. |
| L6 | Accept a first experiment run that launches builders from the existing Codex container, adding jailed terminals afterwards? | Yes. It takes most of the second milestone off the critical path. |
| L7 | Two approved designs (builder session, gaps) are lost. Are there copies, or are versions rebuilt from the decision log acceptable for re-approval? | Accept rebuilt versions. |
| L8 | Approve the first milestone's designs as a batch, instead of one approval per card? | Yes. Most of it is deployment, prompts and configuration. |
| L10 | Sign the golden set once it is drafted? Agreed 2026-11-04; no draft exists yet. | Nothing to do until the draft arrives. |
| L11 | Update the project's docs, which list "agents merging PRs" as a non-goal, when the new merge rule takes effect? | Yes, in the same change that lifts the rule. |

## Mill Board, gates and the operating model

| # | Question | Recommendation |
| --- | --- | --- |
| M1 | Approve the minimal role fix: one credential per agent, the builder role loses admin, merge and triage, and "close" is gated or replaced by a narrow "record merged work" capability? | Yes. It is configuration, not code. |
| M2 | Which role should Claude chat and voice sessions hold? Today they sign in as the human admin. | A design role: add and comment on cards, no gates, no merge. |
| M3 | Start the design for the board's own SQLite store? | Yes. Design first; Nano approves before any migration. |
| M4 | With Nano out of the gates, who commits work from the backlog? | Nano, by setting priorities per project or milestone; agents pull from that. |
| M5 | Does the ask-first list (data migrations, deletions, dependency and toolchain bumps, public API changes) still reach Nano once agents may merge and deploy? | Yes, keep it. |
| M6 | Where do the evidence gates live? | In Let Em Loose. Do not build richer gate machinery into Mill Board's columns. |
| M7 | Drop the Hermes Kanban pilot agreed in the 2026-11-10 Mill Board audit? | Yes. One executor fewer to keep honest. ("No fourth executor" is Let Em Loose D20, which Nano reopened for mux-hosted agents on 2026-11-04, so it is not by itself a reason.) |
| O1 | Update the Assistant Operating Model with the audit findings (lanes bound to credentials, design produces notes not cards, research enters as claims, the board's own store, deploys)? | Yes. |
| O2 | Create a GitHub identity for Claude's work, so its pull requests and reviews stop posting as Nano? | Yes. Dottius already has one; GitHub's own review rules can then enforce cross-family review. |
| O3 | Confirm Duetius as coordinator, including choosing between two agents' positions on reversible changes? He is Claude-family, like the primary builder. | Yes for routing and chasing. For disputes over Claude-authored work, Dottius or Codex decides. |
| O4 | Add one design-first card, "enforce the operating model", covering identities, roles and the switch to the new merge rule? | Yes. |

## remind-me

| # | Question | Recommendation |
| --- | --- | --- |
| R1 | Approve this order: (1) stop decisions and preferences going dormant, (2) storage behind one port plus a SQLite benchmark, (3) refinement as a background worker? | Yes. |
| R2 | Which stores go in the benchmark? | The current engine, plain SQLite and Ledger's evidence store. Leave Kuzu and DuckDB out unless SQLite falls short. |
| R3 | Is multimodal DB a product or a research vehicle? | A project in its own right. It returns as remind-me's store only if it wins the benchmark. |
| R4 | Keep capturing full conversations, or gate what is stored? | Keep full capture, kept out of ranking. The originals are the evidence. |
| R5 | Agent memory or personal knowledge hub? | Both, with one store: importers become external adapters; the core stays agent memory. |
| R6 | Hold the knowledge pipeline until refinement is running again? | Yes. |
| R7 | Which of Ledger's ideas are carried in: immutable originals with digests, idempotent writes, separate "true at" and "learned at" times, model output admitted only by policy, summaries rebuilt from facts? | All five, in that order of value. |
| R8 | Commission a security review of the in-house OAuth server crate? remind-me is internet-facing. | Yes. |
| R9 | Does the deployed server run the 11-tool core profile? | A check to run; not yet verified. |

## Portfolio Intelligence

| # | Question | Recommendation |
| --- | --- | --- |
| P1 | Approve, change or reject the shorter MVP path that repo's own scope document proposes? | Approve it, then go smaller still for the first step (P2). |
| P2 | Make the next pull request a live path: a real GitHub transport, a sync command and a repository query? Shell out to the `gh` CLI (no new dependency) or add a small HTTP client? | Yes, with `gh` first. |
| P3 | Ratify the 14 decisions agents defaulted on Nano's behalf? | A 20-minute pass: keep, change or don't care for each. |
| P4 | Write the threat model as single owner, local disk, read-only token, and drop the mid-flight revocation and leak-equivalence tests from the MVP gate? | Yes. Keep fail-closed sync and provenance. |
| P5 | Collapse to the newer registry as the only model the CLI reads, and convert the two `.docx` fixtures to text so two dependencies can go? Both involve deletions. | Yes, after P2 proves the registry fills from live data. |
| P6 | Replace the append-only status documents with one short current-state file, and retire the stale workflow file? | Yes. |
| P7 | Portfolio Intelligence stays read-only, and a separate export job posts its findings into remind-me? | Yes. |
| P8 | Make its registry the authority for project identity, with board and remind-me projects carrying the repository id? | Yes. The board has one project under two spellings today. |
| P9 | Build Cargo impact analysis once, there, with Converge consuming it? | Yes. |

## Experiments and side projects

| # | Question | Recommendation |
| --- | --- | --- |
| X1 | Ledger: fold its evidence store into the remind-me benchmark, stop its recall, MCP and inference roadmap, and close its 37 open cards against a written conclusion? | Yes. |
| X2 | Conflux: pause it and close its 30 cards? Anything to port into BBP? | Pause and close. BBP already has epochs, action marks, budgets and replay. |
| X3 | Swarm OS: archive it and relabel it on the board as a prototype instead of 47 shipped? | Yes. |
| X4 | Converge Continuum: has the rebase and CI cost been measured? Finish the baseline, then trial Jujutsu with the merge queue for two weeks before building the kernel? | Measure first, then the trial. |
| X5 | Communications: run one bounded real-model trial, write a one-page conclusion and close? | Yes. No optimisation or Protobuf arms. |
| X6 | Archive rusty_tick once the board has migrated to its own store? | Yes, as already decided; confirm the timing. |

## Ecosystem-wide rules

| # | Question | Recommendation |
| --- | --- | --- |
| E1 | Adopt an entry rule: a new idea starts as a design note in remind-me, and a repository and cards are created only after an overlap check? | Yes. Deliberate experiments are allowed if labelled, with a harvest at the end. |
| E2 | Run the backlog triage pass (an agent proposes commit, fold or shelve per project; nothing moves until Nano approves), starting with Conflux, Ledger and Converge? | Yes. |
| E3 | Is any of the five audited repos carrying work committed for this month? | Anything that is not can leave the board. |
| E4 | Settle one Rust toolchain pin across the repos, and move Ledger and Conflux to the newer SQLite binding? Both are ask-first changes. | Yes to both. |
