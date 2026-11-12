---
category: Added
changelog: Mill Board ADR-0006 sets out observation-only external execution refs (design accepted; implementation follows).
---
## 2026-11-11 - Mill Board external execution refs design

- **Added:** ADR-0006 (Accepted), the design for observation-only `external_refs` on Mill Board cards. These are pointers to Codex Cloud, GitHub PR and Actions executions that do not run under Rusty Orch. It covers the schema, the `card_execution_observe` API, validation, authorization, idempotency, stale and future observations, retention, compatibility, rollback and consumers. No code in this change.
- **Added:** Let Em Loose decisions D122 (approved in principle), D123 (design defaults), D124 (design approved as written) and D125 (clarifications).
