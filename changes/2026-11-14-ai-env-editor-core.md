---
category: Added
changelog: Ai Env Manager has the pure rules for in-app file editing (not yet wired to disk or the GUI).
---
## 2026-11-14 - Ai Env Manager: editor core (slice 1)

- **Added:** `ai_env_core::edit` with `RelPath` (entry-relative path, no escapes), `Ownership`, `check_editable`, `check_text`, `EditPlan` and `plan_edit`. Pure, no disk access. Rules follow D38 and `editor-plan.md`: marker file never editable, unmanaged target files read-only, 1 MiB UTF-8 text only, no-change edits refused.
- **Changed:** `ai_env_fs` takes the marker name from `ai_env_core::MARKER_FILE`, so it has one source.
- **Known limitations:** nothing reads or writes files yet; list/read/preview/apply (slice 2) and the GUI (slices 3 and 4) follow.
