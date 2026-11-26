---
category: Added
changelog: The GUI gained a workspace explorer panel on its right edge, opened from the command palette or the prefix chord `e`.
---
## 2026-11-26 - Workspace explorer panel: drawing, keys and mouse

- **Added:** the explorer panel (gui-design D99, card #1573). It overlays the right edge, 360 px wide between the strip and the ribbon, and never reflows the blocks. Open it with the palette row "Workspace explorer" or the chord `e` after the prefix (`Command::Explorer`, `Cmd::Explorer`); it shows only when the source says the block can list a workspace. Keys: arrows, Enter, PageUp/PageDown, `y` copies the path to the system clipboard, Esc closes. A click selects a row, the close button closes, a click on a block goes on to the block. Light and dark screenshots are in `docs/projects/term/gui-design/assets/workspace-explorer/`.
- **Known limitations:** the wheel does not scroll the list yet (the cursor does), no Home/End, no double-click to toggle, and no "Copied" line in the footer. The footer lays its keys on the left and the size/age on the right, where D99 puts them the other way round. Not checked on Windows or on a GPU from the cloud.
