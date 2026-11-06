---
category: Added
changelog: rusty-mux-gui draws the pack's top strip (session, a tab per window with its layout thumbnail, "+", clients attached) in Instrument Sans, and switches windows from it.
---
## 2026-11-06 - rusty-mux-gui: the top strip and the interface font

- **Added:** arc 2 slice S3a (`docs/projects/term/ARC2-MUX-CLIENT.md`, handoff D104-D107, gui-design D45, D46, D59). An attached window draws the F1 top strip in place of the cell-grid tab bar: the session chip ("rusty_mill on {host}", opening the menu without the shell launchers), a tab per server window with its 26×16 layout thumbnail, "+" for a new window, "{n} clients attached" and the caption buttons. Clicking a tab selects that window on the server; the window shown follows the session's active window, whoever picks it.
- **Added:** proportional interface text in rusty_term (`gui/text.rs`): Instrument Sans 400 and 500, static instances of the OFL variable font, embedded under `crates/libs/ui/rusty_term/assets/fonts/` with its licence. A small antialiased painter (`gui/paint.rs`) draws rounded boxes, the pack's stroke icons and text into a pixmap both renderers present: the CPU renderer copies it, the GPU one uploads it as a texture.
- **Changed:** a window whose last block ends hands over to its neighbour that shows blocks instead of closing the GUI; with none left it still closes. `rusty-mux-gui` takes `--gpu`.
- **Known limitations:** local mode keeps today's chrome (D107). No thumbnail marks a running block yet, and the clients count counts people (distinct principals), not apps (handoff Q23). Glyphs are not kerned. While scrollback search is open the cell search bar takes the strip's place.
