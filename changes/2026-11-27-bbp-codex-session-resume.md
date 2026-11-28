---
category: Added
changelog: rusty_bbp_host: `codex-bbp-harness` keeps a session by resuming the thread id captured from its first turn's log (`BBP_PROVIDER_SESSION`).
---
## 2026-11-27 - Codex session resume in BBP launchers

- **Added:** a `"session": true` Codex launcher no longer refuses to run (#432).
  The moderator captures Codex's thread id from the first turn's `--json` log,
  stores it beside the session marker as `session-<id>.provider`, and passes it to
  resumed turns as the new launcher variable `BBP_PROVIDER_SESSION`. The launcher
  drops `--ephemeral` for session turns and resumes with `codex exec ... resume <id>`.
- **Known limitations:** a resumed turn with no captured id (the first turn died
  before Codex started its thread) is refused rather than starting fresh. Unconfined
  session rollouts stay in `CODEX_HOME`; BBP does not prune them.
