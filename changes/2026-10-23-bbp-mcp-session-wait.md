---
category: Added
changelog: `bbp mcp` can serve a whole task as a session (`BBP_SESSION=yes`) with a `wait` tool that blocks until the role is granted a turn.
---
## 2026-10-23 - BBP session servers and the `wait` tool

- **Added:** `bbp mcp` with `BBP_SESSION=yes` (instead of `BBP_TURN`) serves its principal for the whole task. Its sixth tool, `wait`, blocks until the role holds a turn (`turn`, or `held` when already shown), the task is closed or cancelled (`ended`), or `timeout_s` passes (default 60, at most 600). It acts only in the turn `wait` last showed, with that turn's token. Step 3a of `docs/bbp-event-based-plan.md`.
- **Unchanged:** per-turn servers, their tool list and refusals; provider launchers still require `BBP_TURN`.
- **Known limitations:** nothing launches sessions yet (3b), and they are not yet confined or cost-accounted (3c). Validated with scripted MCP clients; no live-model run.
