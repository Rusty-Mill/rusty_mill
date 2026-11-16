---
category: Added
changelog: aienv push and a Push to source button copy the edits made in a tool's copy of a skill back to the skill's source, after a diff and a confirmation.
---
## 2026-11-15 - Push a copy's edit back to the source skill

- **Added:** `aienv push ID [FILE] --tool ID --skills DIR... [--yes]` and a "Push to source…" button in the GUI editor. When a tool's marked copy of a skill was edited, it shows a diff of the source against the copy and, on confirm, writes the changed files to the source with the editor's save. If the copy then equals the source, its marker is refreshed and the skill reads enabled again; otherwise it stays edited and the paths that still differ are listed. Both sides are re-hashed when applying, so a file changed since the preview is refused.
- **Known limitations:** the marker holds no source-side hash, so a source file changed since the copy was taken is overwritten (the preview says so); files that exist on one side only are listed, never created or deleted; a write error part-way leaves the files before it pushed.
