---
category: Added
changelog: Ai Env Manager filesystem adapter (ai_env_fs): observes a tool folder and applies link plans with symlink, junction and marked-copy fallback.
---
## 2026-11-13 - Ai Env Manager filesystem linker (M0 slice 2)

- **Added:** `crates/apps/ai_env_manager/crates/ai_env_fs`. `observe` reads a target path into the core's `Observed` (a marked copy records its source and a SHA-256 of the tree, so an edited copy is told from a stale one); `apply` runs a `Plan`, rejects any plan holding a refusal before touching the disk, and never removes a path that is not a link or a marked folder. Depends only on `ai_env_core` and `rusty_sha2`.
- **Known limitations:** the Windows junction (`mklink /J`) is untested in CI; symlinks inside an entry's source are refused; no tool catalog (waits on the TOML-reader decision) or CLI yet.
