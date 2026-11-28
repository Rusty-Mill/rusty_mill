---
category: Added
changelog: Mill Board Settings > Agents shows registered-agent slots used per connector, marks each agent active or stale, and previews "Revoke all stale"; revoking an agent now releases its card claims.
---
## 2026-11-28 - Mill Board: registered-agent slots, stale marking, and claim release on revoke

- **Added:** Settings > Agents shows how many of a connector's 64 registered-agent slots are used, marks each agent active or stale by the same rule revoke-stale uses (24 hours, with the 1-hour last-use lag after a restart), and "Revoke all stale" lists the agents and the claims it would release before it revokes anything (#1643).
- **Changed:** revoking one registered agent releases the card claims it held, with a handoff comment on each card by the revoking admin. An unknown id changes nothing. Revoke-stale's preview lists the stale agents and the claims it would release: theirs, and any left by agents revoked earlier (for example after a failed card write). The preview is in Settings even when no agent is stale or registered. Confirming sends exactly the previewed agents and claims, and each claim is re-checked when it is written, so a claim orphaned after the preview, or one held by a live agent or someone else, is never released (#1644). Registering again never restores claims.
- **Known limitations:** none known. A failed release, or a failed registry write, leaves the claims for the next preview; no restart or re-registration is needed.
