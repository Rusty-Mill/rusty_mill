# Millwright: roadmap

Thirty milestones across six workstreams and five phases. Workstreams run in parallel
inside a phase; phases run in order. No dates until Nano has answered the Phase 0
questions and set priorities. Sizes are rough: S is one pull request or a configuration
change, M is a handful of pull requests, L is a multi-step effort with a migration or an
experiment. "Needs Nano" refers to question numbers in [QUESTIONS.md](QUESTIONS.md).

Status: draft. It assumes the recommendations in QUESTIONS.md are accepted. A different
answer to M1, O2, L6 or R2 changes the shape of G1, L3 or R3.

## Overview

| Workstream | Phase 0 | Phase 1 | Phase 2 | Phase 3 | Phase 4 |
| --- | --- | --- | --- | --- | --- |
| Governance | G0 Priorities | **G1 Identities** | G2 Coordinator | G3 Review | G4 Evidence gates |
| Mill Board | **B0 Triage** | B1 Lean cards | B2 Store design | B3 Migration | B4 Claims view |
| remind-me | R0 Notes | R1 Dormancy fix | R2 Storage port | R3 Benchmark | R4 Refinement |
| Let Em Loose | L0 Commit | L1 Deploy | **L2 Smoke run** | **L3 Experiment** | **L4 Real intent** |
| Portfolio Intelligence | P0 MVP path | P1 Slim down | **P2 Live path** | **P3 Overlaps** | P4 Agent access |
| Side projects | S0 Close out | S1 Hygiene | S2 Trials | S3 Jujutsu | S4 Converge |

Bold marks the critical path. remind-me and Portfolio Intelligence work happens in their
own repositories; everything else is in `rusty_mill`.

## Critical path

B0, then G1, then L2 and P2 together, then L3 and P3 together, then L4. Everything else
has slack and can slip a phase without delaying the end state.

- Phase 0 and G1 are small, and every later milestone lands through the merge flow G1 creates.
- L3 is the longest single item and the only one that needs sustained time from Nano
  (golden set, shadow run, judging). Protect it from new design work.

## Phase gates

| Gate | Passes when | Who checks |
| --- | --- | --- |
| 0 to 1 | The paused projects have no open cards; every remaining project has a priority from Nano; the entry rule is written into AGENTS.md | Duetius reports; Nano confirms |
| 1 to 2 | A builder agent is refused at a gate and at a self-merge; one routine PR merges on green CI plus a cross-family review with no action from Nano; an untouched decision appears in default search | Dottius demonstrates on a test PR |
| 2 to 3 | The smoke run completes with a replayable log; Portfolio Intelligence lists the real repositories from a live sync | A reviewer from the other model family re-runs both |
| 3 to 4 | The overlap report names the overlaps the 2026-11-10 audits found; the experiment has a written verdict against rusty_fclone; the storage benchmark has a winner; the board runs on its own store | Nano on the experiment verdict; agents on the rest |

## Governance

| Code | Deliverable | Done when | Depends on | Lane | Size | Needs Nano |
| --- | --- | --- | --- | --- | --- | --- |
| G0 | Project priorities and the entry rule | Every active project has a priority; "notes first, cards on commitment" is in AGENTS.md | B0 | Nano, Design | S | M4, E1, E3 |
| G1 | Agent identities, least-privilege roles, cross-family review, and the machine-verified evidence checks M-merge requires; only then does the new merge rule take effect | The Phase 1 gate demonstration passes on rusty_mill and remind-me | G0 | Build, Merge steward | M | M1, M2, M5, O1 to O4 |
| G2 | Coordinator lane and a metrics baseline | Duetius posts a daily digest; the four numbers are recorded for the first time | G1 | Coordination | S | O3 |
| G3 | Review of the stopgap | A written comparison of review-gated merges against what the first runs showed, with changes proposed | L2, P2 | Coordination, Research | S | Read and react |
| G4 | Evidence gates take over | Work routed through Let Em Loose merges on its evidence; review-based gating stays for everything else | L3, G3 | Build, Merge steward | M | Inbox only |

## Mill Board

| Code | Deliverable | Done when | Depends on | Lane | Size | Needs Nano |
| --- | --- | --- | --- | --- | --- | --- |
| B0 | Backlog triage | Paused projects hold no open cards; each remaining project's proposal is approved and applied | X answers | Coordination, Design | M | E2, X1 to X6, approve each list |
| B1 | Lean cards and per-agent logins | Agents list summaries by default; each agent signs in as itself; adding a card can set priority and links in one call | G1 | Build | M | M1, M2 |
| B2 | Own-store design | A design note for the SQLite store, migration and rollback is approved | B0 | Build, Review | S | M3, approve the design |
| B3 | Migration to the board's own store | Dry run shows no difference; production migration done; the rusty_tick and engine adapters are removed; rusty_tick archived | B2 | Build, Merge steward | L | Authorise the migration and the deletions |
| B4 | Claims view | The board shows Let Em Loose claims and the inbox as its main agent-facing surface; columns stay for work that does not run through it | L3, B3 | Build | M | Inbox only |

## remind-me

| Code | Deliverable | Done when | Depends on | Lane | Size | Needs Nano |
| --- | --- | --- | --- | --- | --- | --- |
| R0 | Notes and conclusions stored | Each experiment has a one-page conclusion and each folded project a design note, searchable by project | B0 | Design, Coordination | S | X1 to X3 |
| R1 | Dormancy fix | Decisions and preferences never drop out of default search by age; a test proves it; the deployed tool profile is confirmed | None | Build | S | R1, R9 |
| R2 | Storage port and a test set | The mirrored storage layers are one interface; a 50-query golden set with expected results runs in CI | R1 | Build, Research | M | R2 |
| R3 | Benchmark and store decision; OAuth review | The current engine, SQLite and Ledger's store are measured on real data; a winner is chosen; the OAuth review has a findings list | R2 | Build, Review | M | R3, R7, R8, choose the store |
| R4 | Background refinement and intake | Store migrated if the winner changed; refinement runs as a background worker; pipeline tools leave the agent-facing surface; knowledge intake resumes | R3, L3 | Build | L | R4 to R6, authorise the migration |

## Let Em Loose

| Code | Deliverable | Done when | Depends on | Lane | Size | Needs Nano |
| --- | --- | --- | --- | --- | --- | --- |
| L0 | First milestone committed | Its six cards are in Ready with batch-approved designs; the lost designs are rebuilt; the design freeze is recorded | None | Nano, Design | S | L1, L7, L8 (L9 decided: lel-freeze) |
| L1 | Deployment and prompts | The protocol host, orch and the inbox run on BaileyAi; the four lens prompts and the cost breaker wiring are merged | L0, G1 | Build | M | L2, L4 |
| L2 | Smoke run | One toy claim settles on the local model with a log that replays to the same outcome | L1 | Build, Review | S | L3 |
| L3 | Duplicate-file experiment, stage one | Built from intent alone using the Codex container; golden set signed; shadow run done; a verdict against rusty_fclone on the five success criteria | L2 | Build, Research, Nano | L | L5, L6, L10, sign the golden set, judge the result |
| L4 | First real intent | One real change to the ecosystem is built through it and merged on its evidence; jailed terminals and value scheduling re-enter scope | L3, G4 | All lanes | L | Intent and inbox only |

Let Em Loose questions go to `let-em-loose-expert`, not to this folder.

## Portfolio Intelligence

| Code | Deliverable | Done when | Depends on | Lane | Size | Needs Nano |
| --- | --- | --- | --- | --- | --- | --- |
| P0 | Scope decided | The shorter MVP path is approved or changed; the 14 defaulted decisions are each kept, changed or waved through | None | Nano | S | P1, P3 |
| P1 | Slim down | A five-line threat model replaces the revocation criteria; one short status file replaces the ledger; the stale workflow file is retired | P0, G1 | Build | S | P4, P6 |
| P2 | Live path | One PR adds a GitHub transport, a sync command and a repository query; the output lists the real repositories | P1 | Build, Review | M | P2 |
| P3 | Overlap report into remind-me | An extractor reads Cargo manifests and README headings at pinned commits; the report flags the overlaps the audits found; findings post into remind-me; the registry becomes the authority for project identity | P2, R0 | Build, Research | L | P5, P7, P8 |
| P4 | Agent access and prioritisation | Agents can query it directly; the board checks it when a card is added; Cargo impact analysis and scoring follow, ranked by use | P3, B3 | Build | L | P9 |

## Side projects

| Code | Deliverable | Done when | Depends on | Lane | Size | Needs Nano |
| --- | --- | --- | --- | --- | --- | --- |
| S0 | Experiments closed out | Ledger, Conflux and Swarm OS each have a conclusion in remind-me and no open cards; Swarm OS is labelled a prototype | B0 | Design, Coordination | S | X1 to X3 |
| S1 | Shared hygiene | One Rust toolchain pin across the repos; Ledger and Conflux on the newer SQLite binding, or archived instead | S0 | Build | S | E4 |
| S2 | Two bounded trials | Communications has run one real-model trial and has a one-page conclusion; Converge has baseline numbers for rework, integration latency and redundant builds | G1 | Research, Build | M | X4, X5, a call budget |
| S3 | Jujutsu trial | Two weeks of real work with Jujutsu, the merge queue and the affected-crate planner, measured against the baseline | S2 | Build, Merge steward | M | None |
| S4 | Converge decision | A go or no-go on the kernel, written from the measured numbers | S3 | Nano | S | Decide |

## Putting this on the board

The board holds committed work only, so the roadmap does not become 30 cards.

1. One milestone per phase in each affected project, undated until Nano sets priorities.
2. Card only Phase 0 and Phase 1 now. Existing cards that already cover a milestone are linked, not duplicated.
3. A later phase's cards are written when the gate before it passes, using the previous phase's retro.
4. Cards made obsolete by this roadmap are marked superseded during the Phase 0 triage.
5. Use a `millwright` tag and phase tags. Do not create a Millwright project.

## Cadence

- Daily: Duetius's digest of what merged, what is blocked and what waits on Nano.
- At each gate: the demonstration in the gates table, then a short retro saved to remind-me.
- After Phase 2: dates are added to Phases 3 and 4.

## Not on this roadmap

New repositories, new Let Em Loose design, the remind-me UI overhaul and graph view, the
Mill Board live flow view, value scheduling, the alien lens, the Hermes Kanban pilot and
the Converge kernel. Each can return at Phase 4, ranked by evidence from the runs.

## Assumptions to check

- Sizes are estimates from reading the repositories and decision records, not measured velocities.
- The BBP and orch code was not read in depth and their tests were not run when this was
  written; L1 and L2 are where that gets checked.
