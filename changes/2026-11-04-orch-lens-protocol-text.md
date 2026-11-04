---
category: Added
changelog: "`orch-lens` tells every model the reply protocol (`post::protocol`, in each lens prefix) and what its job may post (`post::allowance`, before the job's suffix)."
---
## 2026-11-04 - orch-lens: models are told how to reply

- **Added:** `post::protocol()` (Let Em Loose #594) describes the strict `{"posts": [...]}` reply: claim and evidence fields, references, bare-number ids and the limits `parse` checks (taken from the same constants). It ends with an example reply, and a test checks that the example parses. `context::prefix` now ends with this text under `# Reply`, so it stays byte-identical across a lens's jobs.
- **Added:** `post::allowance(&Allowed)` says what one job may post: evidence on which claim and in which directions, claims that must answer a given question, new claims, or nothing. `call` puts it before the job's own suffix (look harder, rebel), so it counts toward the context fit.
- Until now, no model was told the reply format. Every reply would have been refused as `Permanent`.
