# Millwright: open questions for Nano

Every question from the 2026-11-10 audits that needs Nano's answer, grouped by system,
each with a recommendation. An answered question moves to [DECISIONS.md](DECISIONS.md)
with the date and where he said it, and its row is removed here. Nothing below has been
acted on.

## Answer these seven first

1. ~~**L2:** authorise Let Em Loose's deployment and confirm the host.~~ Answered 2026-11-13; see DECISIONS.md l2-baileyai-host.
2. ~~**M1:** approve the minimal role fix on Mill Board.~~ Answered 2026-11-12; see DECISIONS.md m1-role-fix.
3. ~~**O2:** give Claude its own GitHub identity.~~ Answered 2026-11-12; see DECISIONS.md o2-claude-github-app.
4. **P1:** decide Portfolio Intelligence's shorter MVP path. Nothing there is dispatchable until then.
5. **R1:** approve the remind-me order of work. The dormancy fix protects a store in daily use.
6. ~~**X1 to X3:** close the three experiments against their written conclusions.~~ Answered 2026-11-12; see DECISIONS.md close-ledger, pause-conflux, archive-swarm-os.
7. ~~**E2:** start the backlog triage pass, beginning with the 113 cards of the paused projects.~~ Answered 2026-11-12; see DECISIONS.md backlog-triage. Which projects count as paused: see DECISIONS.md project-priorities.

## Let Em Loose

These are also tracked by `let-em-loose-expert`; its pack is the authority for that project.

L1, L7 and L8 are answered (2026-11-12) and L2 (2026-11-13); see DECISIONS.md lel-m1-commit, lel-lost-designs, lel-m1-design-batch, l2-baileyai-host.

| # | Question | Recommendation |
| --- | --- | --- |
| L3 | Is the smoke run (one toy claim, local model, no builders, no spend) the first target, ahead of the full experiment? | Yes. |
| L4 | Approve the first live run and its spend ceiling? Today's rule is flat-rate subscriptions and local models only. | Flat-rate and local only for the first run; revisit paid calls after it. |
| L5 | Which credential lets the Claude CLI work inside the builder jail, which is meant to hold no secrets? This blocks the builder work. | Nano's call. Simplest: a subscription login on a dedicated account used only by this jail. |
| L6 | Accept a first experiment run that launches builders from the existing Codex container, adding jailed terminals afterwards? | Yes. It takes most of the second milestone off the critical path. |
| L10 | Sign the golden set once it is drafted? Agreed 2026-11-04; no draft exists yet. | Nothing to do until the draft arrives. |
| L11 | Update the project's docs, which list "agents merging PRs" as a non-goal, when the new merge rule takes effect? | Yes, in the same change that lifts the rule. |

## Mill Board, gates and the operating model

| # | Question | Recommendation |
| --- | --- | --- |
| M6 | Where do the evidence gates live? | In Let Em Loose. Do not build richer gate machinery into Mill Board's columns. |
| M7 | Drop the Hermes Kanban pilot agreed in the 2026-11-10 Mill Board audit? | Yes. One executor fewer to keep honest. ("No fourth executor" is Let Em Loose D20, which Nano reopened for mux-hosted agents on 2026-11-04, so it is not by itself a reason.) |

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
| X4 | Converge Continuum: has the rebase and CI cost been measured? Finish the baseline, then trial Jujutsu with the merge queue for two weeks before building the kernel? | Measure first, then the trial. |
| X5 | Communications: run one bounded real-model trial, write a one-page conclusion and close? | Yes. No optimisation or Protobuf arms. |
| X6 | Archive rusty_tick once the board has migrated to its own store? | Yes, as already decided; confirm the timing. |

## Ecosystem-wide rules

None open.

M1, M2, M5, O1, O2, O3 and O4 are answered (2026-11-12); see DECISIONS.md m1-role-fix,
m2-design-role, m5-ask-first-stands, o1-operating-model-update, o2-claude-github-app,
o3-duetius-coordinator, o4-enforce-operating-model-card. Still open under O2: which of
g1-identity.md's options a/b/c covers cloud Claude acting as `baileyrd` (handoff/OPEN_QUESTIONS.md Q10).

E2 (run the triage) is answered; see DECISIONS.md backlog-triage. Which projects count as
paused is answered too: see DECISIONS.md project-priorities (Converge is not paused).

E1 (entry rule) and E3 (committed work this month) are answered; see DECISIONS.md
entry-rule and audited-repos-this-month; X1 to X3 still stand (e3-x-reconcile).

E4 (toolchain pin and SQLite binding) is answered; see DECISIONS.md s1-hygiene-scope.
