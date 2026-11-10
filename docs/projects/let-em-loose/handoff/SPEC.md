# Let Em Loose: handoff spec

Design chat: Nano's saved record in remind-me, capture `cap_58e514f9f3d94b3da5d26c0b8a118dd3` ("Let Em Loose: full design discussion, 2026-11-01 to 11-04"; paraphrased, every exchange in order; pointed to in thread message `cmsg_01WgyiM31RsSnZTgNuPX2VE4E2HeirtwhX9Qh6porRyDds`). The share link https://claude.ai/share/a3210fa6-8f48-4b63-bb9f-0f07c4e3771b renders without the conversation; don't cite it.
remind-me tag: let-em-loose (entity "Let Em Loose"; also project-handoff, project-expert)
Tags: [stated] [agreed] [default] [open]. Pack built with project-handoff v1.2.0; refreshed with project-expert v2.0.0 on 2026-11-10 @ d91956c.
Owner: Nano (GitHub `baileyrd`): the person whose words count as `[stated]`.

This pack is an index for the `let-em-loose-expert` subagent and for implementers. It does not restate the design. Full spec: [../SPEC.md](../SPEC.md). Decisions: [DECISIONS.md](DECISIONS.md) points to [../DECISIONS.md](../DECISIONS.md) (D1–D45, D46+). Concept: [../README.md](../README.md). Pre-answered questions: [../QUESTIONS.md](../QUESTIONS.md) (Q1–Q31) and [QA.md](QA.md) (build-time questions). Open: [OPEN_QUESTIONS.md](OPEN_QUESTIONS.md).

## Intent
Nano writes intent; critic lenses on different models post claims and evidence to one BBP board; the board computes confidence and converges. Nano is pulled in only for irreversible calls, taste, deadlocks and budget overruns. [agreed] D1–D5

## Scope
- In (v0): BBP claims mode (#593), orch lens runner (#594), Mill Board decision inbox (#595), ground truth (#597), erosion gate (#704), remind-me project param (#592), the dedup experiment (#600). [agreed] ../SPEC.md § v0
- Next: value scheduling (#596), gaps (#598, #599), SlopCodeBench spike (#705), alien review lens (#708, design not approved). [agreed]/[open]
- Out: agent-to-agent calls, stage pipelines, vendor logic in orch core, roles in BBP, a fourth executor, a new repo, autonomous irreversible actions, agents merging PRs. [agreed] ../SPEC.md § Non-goals

## Constraints (Nano's standing rules)
- Design before code: no implementation until Nano approves the design on the card. [stated] project instructions, D25
- Ask first: schema migrations, BBP schema bumps, public API changes, dependency or toolchain bumps, deletions, adding or removing an executor. [stated] project instructions
- Agents open PRs, request review from `dottiusmaximus`, never queue or merge. One small PR per slice, stacked. [stated] AGENTS.md; remind-me (Nano 2026-11-03)
- Rust: `Result` + `?`, no `unwrap()`/`expect()` outside tests; ports and adapters; no abstraction before two call sites; tests for non-trivial logic. [stated] ../SPEC.md § Constraints
- Commands handed to Nano are PowerShell; work on Nano's machines is a paste-ready Claude Code prompt in project files `mill-board/`. [stated] project instructions

## Shape
| Part | Path | Card |
|---|---|---|
| BBP protocol | `crates/libs/protocol/rusty_bbp` (rules: `docs/spec/bbp-claims-addendum.md`) | #593, #597 |
| BBP host (`bbp` CLI, runner, profiles) | `crates/apps/rusty_bbp_host` | #593, #595, #597, #704 |
| orch + lens runner | `crates/apps/rusty_orch` (`orch-lens`, `orch-openai`, adapters) | #594, #596 |
| Co-edit CRDT | `crates/foundation/rusty_crdt` | #593 |
| Inbox | `crates/apps/rusty_mill_board` | #595 |
| BBP docs | `docs/bbp` | — |
| Erosion metric (to build) | new crate `rusty_erosion` | #704 |

## Current state (2026-11-10 @ d91956c; verify live with `card_get` before relying)
Mill Board column per card, read 2026-11-10; decision numbers from ../DECISIONS.md (D121 is the latest row).
- Done: #592, #593, #594 (all slices merged; a live paid run needs Nano's approval), #595 (follow-up #703; Remediate diffstat deferred), #597, #598, #704 (all slices merged through #1544; erosion is report-only and the limits are Nano's to set, D115), #706, #708 (no live run yet; paid calls need Nano's approval).
- Building: #599 (track records merged; lessons, lens why and the canary re-run not built).
- Blocked: #703 (waits on Nano's approval of the bbp CLI and log change).
- Backlog: #596 (parked by Nano 2026-11-04), #600 (M2 epic, waits on M1 #1077), #705 (waits on #1320), #707 (waits on #1165).
- Unticked questions that still need Nano or an unreachable source are in OPEN_QUESTIONS.md (14 at this refresh).

## Acceptance for this pack
Run 2026-11-10 @ d91956c by a separate agent acting as the expert on a copy (no real writes). All passed.
- [x] The expert answers erosion-gate questions from D41–D45 with a cited source (QA.md § Erosion gate; D41, D43, D45, D30; addendum C-R8).
- [x] Implementation detail gets a logged `[default]`, not an escalation (when merged code already names it, it cites that and logs nothing).
- [x] Anything reversing a `[stated]`/`[agreed]` row escalates to Nano (an `--override` on `bbp erosion` escalated with `REVERSES: D43`).
- [x] Each settled answer lands once in ../DECISIONS.md and once in remind-me, best effort: a stable key reconciles duplicates (a repeated answer added no second row; a `[stated]` answer with no reference was refused).
- [x] A conflict between sources is settled by date and provenance (D115 over QA's "report-only" for what "approved" means).
