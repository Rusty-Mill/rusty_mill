---
category: Added
changelog: rusty-mux-gui frames each block with the pack's header: its folder and git branch, or its program and what it reports doing, and who else has input.
---
## 2026-11-06 - rusty-mux-gui: block frames and headers

- **Added:** arc 2 slice S3b (`docs/projects/term/ARC2-MUX-CLIENT.md`, gui-design D47, D48, handoff D105). An attached window lays the server window's blocks out in pixels, 8 px apart on the frame colour, each in a rounded frame (the focused one in the accent) under a 38 px header. A shell's header shows its folder and git branch; a block running another program shows that program with its C21 status (OSC 7501: working, blocked and why, done, idle, error, with the program's own message); another client holding the block's input shows as "{name} has input". The terminal sits inside the frame on its own background, padded 16 and 12 px.
- **Added:** the GUI's control connection reads each shown block's folder, status report and process chain with `BlockCall`, again when `PwdChanged`, `BlockStatus`, `StatusRemoved` or `ProcessChanged` says so, and follows lease holders from List entries and `LeaseChanged`. `rusty_mux_client`'s `Welcomed` and `Opened` now carry the connection's client id.
- **Changed:** rusty_term's renderers place a pane by pixels as well as cells, and draw an attached window's body pixmap under the grids. Painting a large rounded box only measures distance near its edges, and a pixmap's fingerprint is taken once per change.
- **Known limitations:** the spinner is drawn still (it animates in a later slice). A folder change is seen when the shell reports it (OSC 7) or the foreground program changes. The zoom button, Take over and the disconnected dim come with S3c, S5 and later.
