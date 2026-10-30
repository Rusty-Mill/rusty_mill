---
category: Added
changelog: rusty-mux predicts typed characters at a shell prompt over a slow path and draws them underlined ahead of the server's echo in a split window; `RUSTY_MUX_PREDICT=off` turns it off.
---
## 2026-10-30 - Predictive echo in the client

- **Added:** `rusty_mux_tui::predict`, the client half of `docs/projects/mux/PREDICTIVE-ECHO.md`. A `Predictor` groups typed keys into epochs: printable one-cell characters typed on the blank tail of the cursor's row, and Backspace over its own unchecked guesses. Any other key, a resize, a lost lease, a new screen, a screen switch, an echo mode change and output that leaves the row start a tentative epoch, drawn only once one of its guesses is found on the screen at an `EchoAck`. A failed check drops the whole epoch, and two within ten keys turn predictions off until the next Enter. Nothing is predicted in `Hidden` or `Unknown` mode. A `Gate` times a `Sync` on the block's connection at most once a second and draws above a 30 ms smoothed round trip, stopping below 20 ms.
- **Added:** block connections ask for `ECHO_ACK` and `SYNC`. In a split window the focused block's predictions are drawn underlined, with the cursor after them; a prediction never covers a wide glyph or leaves its block.
- **Added:** `RUSTY_MUX_PREDICT` (`on`, the default, or `off`) is the owner-required off switch; any other value is refused before the terminal enters raw mode.
- **Changed:** `rusty_term_core` exports `char_width` without the `gui` feature, as it already exports `Cell`.
- **Known limitations:** a block shown alone (passing its output through) shows no predictions yet; that and the end-to-end tests over a delayed path are build PR 3.
