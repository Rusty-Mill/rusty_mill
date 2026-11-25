---
category: Added
changelog: Mill Board keeps a settings history (old and new value of each changed section), read by the settings_history op and the Settings > History tab.
---
## 2026-11-25 - Mill Board settings history

- **Added:** every settings change that succeeds is recorded in `settings-history.jsonl` beside `settings-audit.jsonl`: when, who, the operation, the section (agents, registry, roles, github, rules, workflow, project_status, registered_agents) and its old and new value. It is written from the one function every settings call passes, so a new operation is covered without its own code. Values come from the redacted settings view, so no token or key is stored. The newest 5000 entries are kept. New admin op `settings_history` (`limit` 1-500, optional `section`) and a Settings > History tab. Card #1100.
- **Known limitations:** token rotation changes no listed setting, so it is in `settings_audit` only. Apps signed in through OAuth are not recorded. The cap is applied when an entry is appended.
