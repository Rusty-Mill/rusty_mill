---
category: Added
changelog: Mill Board registered agents can have their own role and projects: narrowed at agent_register, or set by an admin in Settings.
---
## 2026-11-13 - Per-agent role and projects
`agent_register` takes optional `agent_role` and `agent_projects` (never wider than the connector's). Settings > Agents > Registered agents sets any role and projects per agent (`settings_registered_agent_set`). Without a grant an agent acts as its connector.
