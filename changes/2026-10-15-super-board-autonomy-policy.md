---
category: Added
changelog: **Super-board autonomy policy accepted and active.** The board robot may merge its own PRs into `main` through the merge gate and queue; no migrations; no committed permission rules. CONTRIBUTING and AGENTS.md amended.
---
## 2026-10-15 - Super-board autonomy policy accepted

- **Added:** `docs/super-board/autonomy-policy.md` (owner-accepted, 10 decisions, two-step rollout) and the operational config `.claude/super-board/configs/rusty-mill.json` (base `main`, merge `auto`, method `merge`, `allowed_envs: []`, `required-gate` required, 2 workers).
- **Changed:** CONTRIBUTING.md exempts board robot PRs from the independent-review expectation; AGENTS.md, CLAUDE.md and the super-board docs describe the active state. The narrow permission set is documented, not committed.
- **Known limitations:** the merge gate has not run against a real card, and it is unverified that it enqueues on the queue-protected `main`. A robot PR touching crypto, TLS or auth can merge without a human reading it.
