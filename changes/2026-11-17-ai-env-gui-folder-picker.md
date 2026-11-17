---
category: Added
changelog: Ai Env Manager finds skills in nested folders (CLI and GUI) and the GUI can choose a skills folder with a native dialog.
---
## 2026-11-17 - Ai Env Manager: recursive skill search and GUI folder picker

- **Changed:** `ai_env_fs::discover_skills` searches every folder below the one given, so `aienv` and the GUI find skills in nested packs (for example `C:\dev\skill_pack`). A folder holding `SKILL.md` is a skill and is not searched further. Symlinked, hidden, `node_modules`, `target` and `.git` folders are skipped; folders deeper than 8 levels and unreadable ones are reported in `skipped`.
- **Added:** GUI "Choose folder…" button (header and empty state) and a `choose_folder` command that opens the native dialog from the backend and adds the folder for the session. `--skills` is now optional in the GUI (still required in `aienv`). New dependency: `tauri-plugin-dialog` (GUI crate only; approved by Nano on 2026-11-17; the page gets no dialog permission and never sends a path).
- **Known limitations:** chosen folders are not saved between launches (D47). The dialog itself is not exercised in CI; `add_folder` is.
