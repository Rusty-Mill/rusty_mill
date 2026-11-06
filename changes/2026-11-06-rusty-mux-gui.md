---
category: Added
changelog: New `rusty-mux-gui` client shows a rusty_mux window's split blocks in rusty_term's native window, following the server's layout and shared focus.
---
## 2026-11-06 - rusty_mux: `rusty_mux_gui`, the GUI client

- **Added:** `crates/apps/rusty_mux/crates/gui` (`rusty_mux_gui`, binary `rusty-mux-gui <socket> [block]`), arc 2 slice S2b (`docs/projects/term/ARC2-MUX-CLIENT.md`, pack D96, D101–D103). It attaches to one server window and draws its split tree through rusty_term's session-source port: a connection per shown block, its screen from the attach checkpoint, then its output. The window follows `LayoutChanged` and `FocusChanged`; a click tells the server the new focus (`FocusBlock`). Only the focused block's input lease is held, and only a leased block is resized to its place. The server's terminal answers the program's queries, so the client doesn't.
- **Added:** `rusty_term::gui::run_attached` with `source::{Source, Tree, Remote}`: a window that shows a source's panes and layout, opens no local shells and never edits its layout itself (split, close and resize keys do nothing there). `rusty_term_core`'s `FrameView::cropped` shows the top-left of a block larger than its place.
- **Known limitations:** no chrome for mux state yet (block headers, status, switcher: slices S3–S5). Layout edits from the GUI, reconnecting after a lost path and predictive echo are not in this client yet; typing into a block before its lease arrives is dropped.
