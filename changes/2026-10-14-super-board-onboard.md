---
category: Added
changelog: **super-board onboarded.** Board automation config, lane scripts, safety guards and AGENTS.md for GitHub Project #1, with `staging` as the merge base.
---
## 2026-10-14 - super-board onboarded

- **Added:** `.claude/` skills, scripts, workflows and guard hooks for the super-board pipeline; config `.claude/super-board/configs/rusty-mill.json` (project `Rusty-Mill/projects/1`, base `staging`, merge policy auto, verify `cargo check --workspace`).
- **Added:** `AGENTS.md` (with `CLAUDE.md` as an `@AGENTS.md` pointer), `docs/agents/issue-tracker.md`, `docs/super-board/PROJECT.md` and `pre-flight.md`.
- **Known limitations:** the push guard blocks direct pushes to `main`, `master` and `staging`; `.claude/settings.json` allows `git push`, so lane agents can still push feature branches.
