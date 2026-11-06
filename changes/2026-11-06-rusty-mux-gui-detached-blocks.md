---
category: Added
changelog: rusty-mux-gui shows a window's detached blocks in a chip on the strip and puts one back with Reattach.
---
## 2026-11-06 - rusty-mux-gui: detached blocks

- **Added:** arc 2 slice S4b (`docs/projects/term/ARC2-MUX-CLIENT.md`, gui-design D61, handoff D108). A block another client took out of the shown window's layout appears in a "{n} detached" chip before the clients count. A click opens its popover: each block's lead glyph, name, state and how long ago it left, newest first, with a Reattach button. Up and Down move, Enter reattaches, Esc or a click outside closes. Reattach places the block beside the focused one; the server places it.
- **Added:** the ribbon counts them ("2 blocks, 1 detached"). A window whose blocks are all detached stays open and says "No blocks shown" with how many keep running, and Reattach fills it again.
- **Changed:** rusty_term's session-source port gains `Session::detached` (`Detached`), `Act::Reattach` and `Remote::empty`. The session switcher module is now `gui::popover`, which draws both popovers. An attached window with no block still draws its strip and body.
- **Known limitations:** the chip is hidden on a server without `LAYOUT_EDIT`. A block detached before this client attached shows no age. The popover's ages update when the session or a block's state changes, not on a timer.
