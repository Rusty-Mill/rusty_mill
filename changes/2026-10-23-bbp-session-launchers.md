---
category: Added
changelog: `bbp mod` can keep one provider conversation per role across turns (`"session": true` in the agents file); each turn is still its own process.
---
## 2026-10-23 - BBP session launchers

- **Added:** a launcher with `"session": true` gives every turn of its role the same provider session id (`{session}`, `BBP_SESSION_ID`, a UUID keyed by the store's master and derived from task and role, so stores reusing the same names never share one) and `BBP_SESSION_RESUME` (`yes` after the role's first launched turn), so the harness can resume its conversation. Each turn is still its own process with its own `bbp mcp`, stopped when the turn ends, and the next turn's harness starts only after the last one has exited. Step 3b of `docs/bbp-event-based-plan.md`.
- **Unchanged:** launchers without `session` (the default).
- **Known limitations:** a session launcher is refused under a cost cap or with a `sandbox`, and may not use `{turn}`; both come with 3c. The provider launchers do not use the session id yet. Validated with scripted shell agents; no live-model run.
