# Let Em Loose: handoff spec

Design chat: https://claude.ai/share/a3210fa6-8f48-4b63-bb9f-0f07c4e3771b (2026-11-03; the share page renders without the conversation, so nothing here is sourced from it)
remind-me tag: let-em-loose (entity "Let Em Loose"; also project-handoff, project-expert)
Tags: [stated] [agreed] [default] [open]. Pack built with project-handoff v1.2.0.

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

## Current state (2026-11-03)
- #593, #597: done. #595: all slices merged; follow-ups #703 (decision ids), Remediate diffstat deferred.
- #594: building. Merged through #1442 (lens call step); next is the `rusty_orch lens` subcommand. Paid providers disabled; v0 is flat-rate and local only.
- #704 erosion gate: approved, not built. D42's expiry and D44's zero-floor gaps were settled on 2026-11-03 (QA.md § Erosion gate); the corrected rows landed in #1449. #596, #598, #599, #600: not built. Their designs are approved except the #598 §6 and #599 §6 schema/API items (approval not recorded for #599) and the #600 builder-session design, which still need Nano's approval (OPEN_QUESTIONS Q3, Q5, Q7).

## Acceptance for this pack
- [ ] The expert answers erosion-gate questions from D41–D45 with a cited source.
- [ ] Implementation detail gets a logged `[default]`, not an escalation.
- [ ] Anything reversing a `[stated]`/`[agreed]` row escalates to Nano.
- [ ] Each settled answer lands once in ../DECISIONS.md and once in remind-me, best effort: a stable key reconciles duplicates from concurrent sessions or a crash between the two writes.
- [ ] A conflict between sources is settled by date and provenance (newest attributable Nano decision; merged contract for behaviour), or escalated.
