# Design note: `agent_register` (a unique, server-issued identity per agent on a shared connector)

Status: **built in this PR** (Nano, 2026-11-13: the connector token identifies *what* is
connecting; each agent needs its own unique identity). Belongs with Millwright M1/O4. Follows
step 1 (instance labels) and `agent_enrol` ([agent-enrol-design.md](agent-enrol-design.md)),
which only reaches agents that hold their own bearer token.

## Problem
Every claude.ai session on one connector shares one OAuth grant, so the board sees one actor.
Step 1 labels (`claude-cloud/7x5uhx`) are chosen by the agent: not unique, not checked, and
another agent can claim the same name. The connector token should say what connects and cap
its permissions; it should not be the agent's identity.

## Proposal
- `agent_register {}` over MCP, allowed to any authenticated connector credential. The board
  returns a unique `agent` id (`<credential>#<random 6 hex>`) and a secret `agent_key`,
  shown once. The registry stores the id, key hash, connector credential and created time.
- Writes from a connector credential pass `agent` + `agent_key`. The board checks them and
  attributes claims, comments and moves to that agent. Enforcement is on at deploy (Nano: enforce, no migration switch); `actor` labels are ignored on a connector.
- Ceiling: the agent's role and projects are the connector's, never wider. It gets the connector's role unchanged (so an admin connector's agents act as admin, as the connector did before); it is not narrowed further.
- Registry in the settings store, capped per connector (suggest 64 live; oldest idle expires
  after 30 days). Settings > Agents lists agents under their connector; revoke one or all.
- Audit: register and revoke are `rusty_log` lines (board events are card-bound). The key is
  never logged.

## Tradeoffs
- The agent must keep its key for the session; a context reset means re-register (new id).
  Claims made under the old id need a release by admin or the same connector.
- A key in the conversation can be leaked by prompt injection: the blast radius is one agent
  id inside the connector's ceiling, revocable alone.
- Not real isolation: a hostile agent can still register more ids up to the cap.

## Alternatives rejected
- One connector per instance: manual, does not scale.
- Trusting MCP session ids: the connector gives no stable per-agent session binding.

## Open questions for Nano
1. ~~Enforce `agent` + key on all connector writes, or keep labels optional?~~ **Decided (Nano, 2026-11-13): enforce.** Connector writes without a registered agent id and key are refused.
2. ~~Expiry~~ **Decided (Nano, 2026-11-13, "go"): 30 days idle, plus manual revoke.**
3. ~~Cap~~ **Decided: 64 live agents per connector.**

## As built
- Args are `agent_id` and `agent_key` (not `agent`: `settings_agent_add` already uses that name).
- Enforced for OAuth sign-ins only (`Identity.connector`, set by the web layer). Bearer-token
  credentials are already unique, need no id, and get a refusal from `agent_register`.
- `actor` labels are ignored on a connector; the agent id is the actor name. The id uses `#`, which no actor name or step-1 label may contain, so a bearer-token holder cannot write under a registered id (tested).
- Keys are stored in clear (not hashed), like identities tokens; the file is private. Hashing would need `rusty_sha2` as a new dependency: left out for now.
- Registry file: `agents.json` in the settings dir, else `<identities stem>.agents.json`; private (0600), keys in clear like identities tokens.
- Admin: `settings_get` lists `registered_agents` (no keys); `settings_registered_agent_revoke {id}`. No UI panel yet.
- Rollback: an older binary ignores `agents.json` and stops enforcing, so connector writes fall back to step 1 labels.

## Per-agent role and projects (follow-up; Nano chose "Both", 2026-11-13)
A registered agent acts with its connector's role and projects unless it has its own grant.
- **Narrow yourself:** `agent_register {agent_role?, agent_projects?}`. The role may hold no capability the connector's role lacks; projects must be a subset of the connector's. Anything wider is refused.
- **Admin sets any:** Settings > Agents > Registered agents has a role and project picker per agent (`settings_registered_agent_set {id, agent_role?, agent_projects?}`). An admin may widen beyond the connector (for example an agent allowed to close cards); sending neither returns the agent to its connector's grant.
- The grant is stored in `agents.json` with the agent. A role removed from Settings later makes that agent's writes fail until an admin resets it.

