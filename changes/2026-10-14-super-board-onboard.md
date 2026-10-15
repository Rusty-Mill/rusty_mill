---
category: Added
changelog: **Inactive super-board proposal and docs.** Document GitHub Project #1 without activating automation; agent instructions retain the CONTRIBUTING review, green-CI and merge-commit/queue workflow.
---
## 2026-10-14 - Inactive super-board proposal and docs

- **Added:** `docs/super-board/proposed-config.md`, an explicitly inactive fenced configuration example outside `.claude/super-board/configs/`. Automatic merges, the `staging` target and test/staging migrations remain unapproved proposals.
- **Added:** `AGENTS.md`, its `CLAUDE.md` entry point, the standalone issue template, project notes and deferred pre-flight documentation.
- **Fixed:** agent guidance follows CONTRIBUTING's reviewed, green-CI merge-commit/queue workflow; removed super-board commands, mandatory `/super-review`, helpers and templates are marked deferred/unavailable.
- **Known limitations:** tooling is not shipped or activated. Local copies are not certified. Activation requires a separately reviewed implementation and an explicit bounded owner policy decision; this PR grants no permissions or merge/migration authority.
