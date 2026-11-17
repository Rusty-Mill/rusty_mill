---
category: Added
changelog: rusty-mux (TUI) draws floating blocks over the layout and has keys to float, move, resize, raise and lower them (C42, slice 2).
---
## 2026-11-17 - rusty_mux floating layers in the TUI (C42, slice 2)

- **Added:** the TUI asks for `Features::FLOATS` and draws each float over the tiles in a rounded border with a "float" header. After Ctrl-]: `f` floats the focused block, `u` raises and `U` lowers it, `m` then `h j k l` moves it and `H J K L` resize it, 5 points a step. A tile zoom hides the floats.
- **Known limitations:** the GUI does not draw floats yet (it cannot be built in the cloud session; a floated block still shows as detached there). The TUI has no mouse, so no drag. Floats are still not saved across a restart.
