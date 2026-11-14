---
category: Added
changelog: Mill Board settings calls are audited, and an agent can only grant what it holds.
---
## 2026-11-14 - Mill Board: guard rails and audit for settings (PR 4 of 5)

- **Added:** an actor may grant a role, capability or project only if it holds it (`agent_add`, `agent_update`, `registered_agent_set`, `roles_set`). `admin` and `enrol` are granted only by a person's own token, never through a shared connector. Every settings call, including refused and denied ones, is appended to `settings-audit.jsonl` (actor, operation, result, reason; never the arguments), and admins read it with `settings_audit {limit}`.
- **Fixed:** rotating a token is refused through a connector, and resetting a registered agent checks the connector role it falls back to.
- **Fixed:** a connector agent's `agent_id` and `agent_key` are no longer rejected as unknown arguments by settings operations.
- **Known limitation:** the audit is a file, not `board_events`, because events are bound to a card. The last-admin guard is the existing `keeps_an_admin` on token credentials; connector identities are not in that file.
