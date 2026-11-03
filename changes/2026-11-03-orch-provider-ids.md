---
category: Changed
changelog: rusty_orch names providers by a validated `AgentId` token instead of a closed enum, so a new model backend is configuration; old snapshots still load.
---
## 2026-11-03 - rusty_orch: providers are ids, not an enum

- **Changed:** `orch_core::task::Agent` (`Claude | Codex | Gemini | Local`) is now `orch_core::AgentId`, a `Copy` token of 1 to 32 bytes from `[a-z0-9_-]`. Each adapter exports the id it serves (`orch_claude::ID`, `orch_codex::ID`, `orch_ollama::ID`); the binary's routable set is `rusty_orch::agents::KNOWN`. First step of the Let Em Loose lens runner (#594). ADR-0019.
- **Changed:** `orch-store` writes agents as their id string (a new, appended `AgentRow::Id` variant, schema tag still v1) and reads the four old variants as `claude`, `codex`, `gemini` and `local`. Nothing stored is rewritten until a goal is saved again.
- **Changed:** errors and progress lines print the id: `stopped: codex failed T-1`, `cannot serve Research through local`.
- **Known limitations:** a snapshot saved by this version is reported corrupt by an older `rusty_orch`, so a rollback cannot resume goals saved after it: don't roll rusty_orch back past this once goals are saved. The config-driven provider registry comes with the `rusty_orch lens` subcommand in a later PR.
