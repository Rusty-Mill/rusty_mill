---
category: Added
changelog: The Codex and Claude BBP launchers can run inside the harness sandbox, checking their turn from moderator-written facts instead of the task directory.
---
## 2026-10-22 - Confinable provider launchers

- **Added:** `codex-bbp-harness` and `claude-bbp-harness` run under a launcher `sandbox`. A confined launcher reads its turn's launch facts from `turn-N.facts.json`, which the moderator writes beside the workspace and the harness may read but not write (`BBP_LAUNCH_FACTS`). The facts are the role's capabilities, the vendors, whether the turn is live, the review state and the repository path. The launcher applies the same checks as when it runs unconfined. The provider's MCP configuration names only the `bbp mcp` relay.
- **Added:** a confined Codex author's clone uses `git clone --shared`. It reads the repository's object store, which the sandbox grants read-only, and writes its own objects.
- **Changed:** the harness sandbox also grants read on `/dev/urandom`, which git and TLS clients read when they are not built to use `getrandom`.
- **Tests:** acceptance mode `launcher-confined` runs the Codex launcher confined, with a provider stub. Unit tests cover the shared checks and the relayed MCP configuration.
