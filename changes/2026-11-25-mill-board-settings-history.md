---
category: Added
changelog: Mill Board keeps a settings history (old and new value of each changed section), read by the settings_history op and the Settings > History tab.
---
## 2026-11-25 - Mill Board settings history

- **Added:** every settings change that succeeds is recorded in `settings-history.jsonl` beside `settings-audit.jsonl`: when, who, the operation, the section (agents, registry, roles, github, rules, workflow, project_status, registered_agents) and its old and new value. It is written from the one function every settings call passes, so a new operation is covered without its own code. Values come from the redacted settings view, so no token or key is stored. The newest 5000 entries are kept, within 2 MiB. An entry over 64 KiB is kept without its values (`omitted_bytes`), a response holds at most 256 KiB (`truncated` says it was cut), and a read looks at the last 2 MiB of the file only. The History tab says when older entries were left out of a response and shows an omitted value as too large to keep. New admin op `settings_history` (`limit` 1-500, optional `section`) and a Settings > History tab. Card #1100.
- **Known limitations:** token rotation changes no listed setting, so it is in `settings_audit` only. Apps signed in through OAuth are not recorded. The caps are applied when an entry is appended, so each append reads and rewrites a file of at most 2 MiB.
