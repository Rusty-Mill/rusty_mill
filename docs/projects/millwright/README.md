# Millwright

Millwright is the programme that came out of the 2026-11-10 ecosystem audits: improve
the tools the rest of Rusty Mill runs on, then use them. It is a label for a programme
(Nano, 2026-11-11). It is not a crate, a repository or a Mill Board project.

Status: **draft, recommendations only.** Nothing here is approved until Nano answers
the questions in [QUESTIONS.md](QUESTIONS.md). Settled answers move to
[DECISIONS.md](DECISIONS.md).

## Where we are

- Current phase: **0, clear the ground** (not started).
- Next gate: 0 to 1. See [ROADMAP.md](ROADMAP.md#phase-gates).
- Waiting on: Nano's answers to the seven first questions.

## The plan in five phases

Each phase improves a tool that the next phase depends on. A phase is finished when
its exit test passes, not when its cards close.

| Phase | Improves | Uses | Exit test |
| --- | --- | --- | --- |
| 0. Clear the ground | The board and its backlog | The audits and the questions | Paused projects hold no open cards; every remaining project has a priority from Nano; the entry rule is in AGENTS.md |
| 1. Make the controls real | Identities, roles, review rules; remind-me dormancy | A lean board with priorities | An agent is refused at a gate and at a self-merge; a routine PR merges on green CI plus a cross-family review with no action from Nano |
| 2. First real runs | Let Em Loose and Portfolio Intelligence | Enforced review and automatic merge | A smoke run settles one toy claim; a live sync lists the real repositories |
| 3. Turn the tools on the ecosystem | Storage under remind-me and Mill Board | The two engines that now run | The overlap report names what the audits found by hand; the duplicate-file experiment is judged; the benchmark has a winner |
| 4. Let the tools build the tools | Whatever the evidence ranks next | The whole stack | One real change ships from intent and the inbox alone |

## Rules of engagement

- **Exit test before the next phase.**
- **One change in flight per system.** Systems move in parallel with each other.
- **Run before you design more.** No new design on a system that has not run end to end.
- **Checked, not claimed.** Done means a test, a run or a merged commit another agent can verify.
- **A retro at each gate**, saved to remind-me, with four numbers: decisions asked of Nano
  per week, cards flowed versus logged after the fact, open backlog cards, and time from
  intent to merge.
- **Ask-first still applies:** migrations, deletions, dependency and toolchain bumps,
  public API changes.
- **AGENTS.md is in force as written.** The merge rule changes only once machine-verified
  evidence checks and per-agent credentials exist (DECISIONS.md, M-merge). G1 builds them.

## What waits until Phase 3 passes

New repositories, new Let Em Loose design, the knowledge pipeline, the Converge kernel,
the remind-me UI overhaul and graph view, the Mill Board live flow view.

## Where the work lives

Mill Board, Let Em Loose (BBP, orch) and the shared crates are in `rusty_mill`. remind-me
(`rusty-remind-me`) and Portfolio Intelligence (`portfolio-intelligence`, with its own
`portfolio-intelligence-expert`) live in their own repositories.

## Files

- [ROADMAP.md](ROADMAP.md): 30 milestones, phase gates, critical path.
- [QUESTIONS.md](QUESTIONS.md): open questions for Nano, with recommendations.
- [DECISIONS.md](DECISIONS.md): what Nano has decided, with date and source.
- [architecture.svg](architecture.svg): agents, shared systems, and what is off the main path.
- [handoff/](handoff/SPEC.md): the pack behind `millwright-expert`, which answers programme
  questions (phase, gate, sequencing, which milestone, what waits) before Nano is asked.

## Sources

The audits and working drafts live outside the repo: remind-me audit, Mill Board audit,
Portfolio Intelligence audit, the five-repo ecosystem audit and the Assistant Operating
Model. Their links are in remind-me under the tags `audit` (2026-11-10) and `millwright`.
