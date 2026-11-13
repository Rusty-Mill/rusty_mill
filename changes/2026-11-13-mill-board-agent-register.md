---
category: Added
changelog: Mill Board `agent_register`; claude.ai connector writes now need a registered agent id and key, so each agent on a shared connector is unique.
---
## 2026-11-13 - agent_register
A connector sign-in (OAuth) calls `agent_register` once and gets `agent_id` and `agent_key`; every write passes both. Bearer-token agents are unchanged. Registered agents expire after 30 days idle, 64 per connector; Settings lists and revokes them. See docs/projects/millwright/agent-register-design.md.
