---
category: Added
changelog: Mill Board ADR-0006 proposes observation-only external execution refs (design only, awaiting approval).
---
## 2026-11-11 - Mill Board external execution refs design

- **Added:** ADR-0006, a proposed design for observation-only `external_refs` on Mill Board cards. These are pointers to Codex Cloud, GitHub PR and Actions executions that do not run under Rusty Orch. It covers the schema, the `card_execution_observe` API, validation, authorization, idempotency, stale and future observations, retention, compatibility, rollback and consumers. No code yet: implementation waits for design approval.
- **Added:** Let Em Loose decisions D122 (approved in principle) and D123 (design defaults).
