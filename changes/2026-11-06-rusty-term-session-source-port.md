---
category: Changed
changelog: The GUI reads every pane through a new session-source port (`rusty_term::gui::source`), so a rusty_mux block can later be drawn like a local shell; no behaviour change.
---
## 2026-11-06 - rusty_term: the GUI's session-source port

- **Added:** `rusty_term::gui::source`, arc 2 slice S2a (`docs/projects/term/ARC2-MUX-CLIENT.md`, pack D101, D103). A pane is a `PaneIo`: a writer for input and size requests, a `Feed` of what to show (`Bytes`, a whole `Screen`, or `Resized` by the source), and whether the window answers the program's queries. `PaneIo::local` is today's PTY path.
- **Changed:** the pane reader thread applies a feed instead of reading the PTY itself, and writes the parser's replies only when the pane answers queries (a mux block's server already does). The read-ahead thread moved into `source.rs` unchanged.
- **Known limitations:** no behaviour change. A failure to start a pane's read thread is now reported as a failed spawn instead of an immediately exiting pane. `rusty_mux_gui` (S2b) is the first source other than local shells.
