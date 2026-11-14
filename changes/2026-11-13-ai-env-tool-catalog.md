---
category: Added
changelog: Ai Env Manager tool catalog (TOML data, restricted zero-dependency reader) and hardening of the filesystem linker.
---
## 2026-11-13 - Ai Env Manager tool catalog and linker hardening (M0 slice 3)

- **Added:** `ai_env_core` reads its tool catalog from `data/tools.toml` through a restricted, zero-dependency TOML reader (`[[tool]]` tables and string values only; any other syntax is an error with its line). The catalog is strict about unknown keys, missing keys, bad or duplicate ids and relative paths. Seeded with Claude Code and Codex.
- **Fixed:** `ai_env_fs` (from the review of #1672): symlink and junction targets are compared after canonicalising; `mklink` is never given a path with `cmd.exe` metacharacters; the copy marker hashes what was written; an unrecordable source path is refused before any change; a failed cleanup is reported; removing a marked folder re-checks its hash.
- **Known limitations:** Cursor and Gemini CLI are not in the catalog until their skills folders are verified; the Windows junction path still is not run in CI; no CLI yet.
