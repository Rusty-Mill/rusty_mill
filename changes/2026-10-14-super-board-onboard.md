---
category: Added
changelog: **super-board board config and docs.** Config for GitHub Project #1 (base `staging`), AGENTS.md and the issue-tracker and project notes. The pipeline tooling itself is not committed yet.
---
## 2026-10-14 - super-board board config and docs

- **Added:** config `.claude/super-board/configs/rusty-mill.json` (project `Rusty-Mill/projects/1`, base `staging`, merge policy auto, verify `cargo check --workspace`).
- **Added:** `AGENTS.md` (with `CLAUDE.md` as an `@AGENTS.md` pointer), `docs/agents/issue-tracker.md`, `docs/super-board/PROJECT.md` and `pre-flight.md`.
- **Known limitations:** the `.claude/` skills, scripts, workflows and hooks stay local until review findings on the merge gate, planner and hooks are fixed. Do not run the pipeline from a checkout without them.
