---
category: Added
changelog: The Ai Env Manager window can open a skill's files, edit one as text, review the diff and save it after you confirm.
---
## 2026-11-15 - Ai Env Manager: editor screen in the GUI (slice 4)

- **Added:** a "Files…" button per skill opens an editor dialog: a file list for the source folder or any tool, a read-only viewer, a plain textarea, and a diff preview with "Confirm and save". Files the backend marks read-only (foreign targets, refused files) have no Edit button. Backend refusals, including "changed on disk", show as the dialog's error and the text stays in the box until a save succeeds.
- **Changed:** `ui/` only. No Rust, no CSP change, no new JS dependency; file text is set with `textContent`.
- **Known limitations:** closing or switching files with unsaved text is blocked silently, and the shared-file warning, revert step and "edited" badge wiring come in slice 5. Checked by screenshot against a mocked backend in light and dark; the real commands are covered by the slice 3 tests.
