---
category: Added
changelog: The Ai Env Manager window backend can list, read, preview and save skill file edits (no editor screen yet).
---
## 2026-11-15 - Ai Env Manager: editor commands in the GUI shell (slice 3)

- **Added:** four Tauri commands, `files`, `file`, `preview_edit` and `apply_edit`, forwarding to `ai_env_app`. They take an entry id, an optional tool id and a path relative to the entry; the page never sends or receives an absolute path. `apply_edit` re-checks every rule from the same inputs as `preview_edit`.
- **Changed:** no new plugin, no capability permission beyond `core:default` (app commands need none here), no CSP change. The capability description and the README list all seven commands.
- **Known limitations:** no editor screen yet (slice 4). The tests run the command logic on temp folders, not through a window, as for the first three commands. A marked-copy edit is covered in `ai_env_app`, not here, since the GUI only makes links on Unix.
