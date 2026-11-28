---
category: Added
changelog: Mill Board Settings > Agents shows registered-agent slots used per connector, marks each agent active or stale, and previews "Revoke all stale"; revoking an agent now releases its card claims.
---
## 2026-11-28 - Mill Board: registered-agent slots, stale marking, and claim release on revoke

- **Added:** Settings > Agents shows how many of a connector's 64 registered-agent slots are used, marks each agent active or stale by the same rule revoke-stale uses (24 hours, with the 1-hour last-use lag after a restart), and "Revoke all stale" lists the agents and the claims it would release before it revokes anything (#1643).
- **Changed:** revoking a registered agent, one at a time or by revoke-stale, releases every card claim it held in the same call, with a handoff comment on each card by the revoking admin; the revoke-stale dry run lists those claims (#1644). Registering again never restores them.
- **Known limitations:** if the registry file cannot be written, the agents are revoked in memory but their claims are not released; they are released when a later revoke-stale run finds those agents stale again (after a restart reloads them).
