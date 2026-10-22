---
category: Security
changelog: Hold Codex worker activation and publish validated patches from a separate exact-head checked job.
---
## 2026-10-22 - Isolate Codex writing and publication

- **Fixed:** remove PATs and persisted credentials from generation; hand off bounded patches to a clean publisher with atomic expected-head checks.
- **Fixed:** resume existing same-repository worker issue PRs, reject forks/unknown identities and serialize requests without rewriting history.
- **Security:** disable all jobs with literal gates and pin actions; require explicit future identity, cost and credential decisions.
- **Known limitations:** no publishing identity is configured and no provider runtime was tested; orphan branches and uncertain API outcomes require manual recovery rather than automatic recreation.

- **Activation hold:** keep the complete design in an inert template and a disabled live hold stub because current repository action policy blocks provider actions even in skipped jobs. A separate allowlist decision is required; no rules were changed.
