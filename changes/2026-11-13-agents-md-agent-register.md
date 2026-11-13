---
category: Changed
changelog: AGENTS.md: connector agents register with agent_register instead of self-chosen labels.
---
## 2026-11-13 - AGENTS.md identity rule
Connector sessions call `agent_register` and pass `agent_id` and `agent_key`; bearer-token agents may still label their actor.
