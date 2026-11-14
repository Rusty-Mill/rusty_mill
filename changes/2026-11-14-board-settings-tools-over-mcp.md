---
category: Added
changelog: Mill Board settings operations are now MCP tools, so a permitted agent can use them, except those that carry a secret.
---
## 2026-11-14 - Mill Board: settings tools over MCP (PR 5 of 5)

- **Added:** thirteen `settings_*` operations are MCP tools beside the board's own: `settings_get`, `settings_audit`, `settings_registered_agent_set` and `_revoke`, `settings_agent_update` and `_remove`, `settings_projects_set`, `settings_roles_set`, `settings_role_rename`, `settings_rules_set`, `settings_project_status_set`, `settings_workflow_set` and `settings_app_revoke`. Each needs `admin`, follows the subset rule and is audited (PR 4); a connector agent passes `agent_id` and `agent_key` like on any write.
- **Unchanged:** `settings_agent_add`, `settings_agent_rotate` (they return a raw token) and `settings_github_set` (it takes one) stay HTTP-only, as does `agent_enrol`.
- **Known limitation:** the tool schemas list each operation's arguments by hand, with objects such as `roles` and `workflow` left open; the operation checks them as before.
