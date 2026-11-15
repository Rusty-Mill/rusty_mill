---
category: Added
changelog: Ai Env Manager can list, read, preview and save edits to skill files in its backend (no GUI yet).
---
## 2026-11-14 - Ai Env Manager: editor backend (slice 2)

- **Added:** `ai_env_fs` `list_files`, `read_file`, `write_file` and `locate`: reads a file's text and hash, and saves, on Unix, with a staged write (a private sibling file given the original's group and mode, then renamed; refused if owner or group cannot be kept) and, on Windows, by rewriting the file in place so its owner and ACL stay. Either refuses if the file changed since it was read. A link inside an entry is never followed.
- **Added:** `ai_env_app` `list_files`, `read_file`, `edit_preview` and `edit_apply` over a `Place` (the source folder, or what one tool has: the source if linked, else its marked copy). The preview carries a line diff and nothing is written until `edit_apply`. A foreign file is read-only; the marker file is never readable or editable. Saving a marked copy moves it to the existing "edited" state.
- **Changed:** `RelPath` also refuses Windows device names (`CON`, `NUL`, `COM1`...) and parts ending in a dot or space. `EditPlan` fields are private, so a plan can only come from `plan_edit`.
- **Known limitations:** no GUI or CLI use yet (slices 3 and 4). `edit_apply` re-checks every rule from the same inputs as the preview instead of taking a plan back from the front end, so nothing from the webview is trusted. A write can still race another writer between the hash check and the rename; the rename itself is atomic.
- **Known limitations:** on Windows a save is not crash-atomic (a crash mid-write can leave a partly written file), though an error such as disk full writes the old text back first; on Unix POSIX extended ACLs are not carried to the new file. The Windows path is compiled by CI only.
