---
category: Added
changelog: Mill Board agents holding the new `enrol` capability can mint and revoke credentials for their own workers.
---
## 2026-11-13 - Mill Board: agent_enrol

- **Added:** capability `enrol` (12 capabilities) and `POST /api/agent_enrol {label, agent_role, agent_projects?}` / `agent_enrol_revoke {label}`. A caller holding `enrol` mints a credential named `<its actor>.<label>`, token shown once, kept in the credentials file with a `parent` field. Ceilings: the role's capabilities must be a subset of the caller's and never `enrol` or `admin`; projects must lie within the caller's (a project-limited caller must name them); at most 8 per lead. Removing a lead removes its minted agents, and so does narrowing the lead's role or projects (or editing roles) so they no longer fit; a lead can revoke only its own. Both calls take the agent's own bearer token: an OAuth sign-in is refused. Each mint and revoke is logged. `settings_get` shows `parent`.
- **Changed:** the credentials file may carry `"parent"` on minted entries. An older binary refuses a credentials file that holds them and a roles file that names `enrol`, so remove minted agents and the `enrol` role before rolling back.
- **Known limitations:** removing a minted agent does not revoke OAuth grants made under its name, so minting the same label again revives them (as with an admin remove-then-add). HTTP `/api` only, not an MCP tool, and the claude.ai connector cannot use a minted token. Design: `docs/projects/millwright/agent-enrol-design.md`. The audit is a log line, not a card event (events belong to cards). No `enrol` role exists until an admin creates one.
