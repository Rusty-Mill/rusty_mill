---
category: Added
changelog: Mill Board Settings > Agents shows registered-agent slots used per connector, marks each agent active or stale, and previews "Revoke all stale"; revoking an agent now releases its card claims.
---
## 2026-11-28 - Mill Board: registered-agent slots, stale marking, and claim release on revoke

- **Added:** Settings > Agents shows how many of a connector's 64 registered-agent slots are used, marks each agent active or stale by the same rule revoke-stale uses (24 hours, with the 1-hour last-use lag after a restart), and "Revoke all stale" lists the agents and the claims it would release before it revokes anything (#1643).
- **Changed:** revoking a registered agent, one at a time or by revoke-stale, releases every card claim it held in the same call, with a handoff comment on each card by the revoking admin; the revoke-stale dry run lists those claims (#1644). Registering again never restores them.
- **Known limitations:** none known. The claims to release are worked out from the cards, not stored: any claim held by a registered agent id that is no longer in the registry. So a release that fails (a card scan error, a card write error, or a registry write failure, after which the agents are already revoked in memory) is released by the next revoke-stale run, even one with nothing stale. No restart or re-registration is needed. A bearer holder's claim and a live agent's claim are never released.
