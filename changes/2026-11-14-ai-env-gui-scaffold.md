---
category: Added
changelog: Ai Env Manager gets a Tauri desktop shell (ai-env-gui) that lists skills per tool and enables or disables them with a plan preview.
---
## 2026-11-14 - Ai Env Manager: desktop GUI scaffold (D31, D34)

- **Added:** `crates/apps/ai_env_manager/gui` (binary `ai-env-gui`, Tauri 2). One window: a table of skills against tools with their state (not enabled, enabled, stale, edited, foreign), and an Enable or Disable button that opens a plan preview. Nothing is written until Apply; a refused tool (for example a foreign folder) is shown with its reason and left untouched. The backend is three typed commands (`list`, `plan_change`, `apply_change`) over `ai_env_app`; `apply_change` plans again and never trusts a plan from the page. Skill folders and tools are fixed at launch with `--skills DIR` and `--tool ID`, as in `aienv`. The page has no file, shell or HTTP access and runs under a strict CSP; it is plain HTML and JS with no npm toolchain.
- **Changed:** `ai_env_app` takes an `Op` (Enable or Disable) instead of a bool, documents `Error`, uses plain `From` impls, and exposes `home_dir()` (shared by the CLI and GUI).
- **Known limitations:** standalone workspace like `rusty_key`'s desktop shell (not built by root CI yet); skills only, no sources, scanner or profiles (M1 to M3); a folder picker is not offered, by design.
