---
category: Added
changelog: rusty-mux now draws predicted typing when one block fills the terminal too, where the program's text style is plain, and has end-to-end tests over a slow path.
---
## 2026-10-30 - Predictive echo for a single block

- **Added:** when one block's raw output passes through, the client draws its predictions with absolute cursor moves, underlined, then `SGR 0` and the cursor back home; never `DECSC`. The cells they cover are put back before the block's next output is passed on, and the predictions drawn again after it if they still apply. They are drawn only while the block's rendition is plain and its cursor move absolute and inside the screen, so that is restored exactly.
- **Added:** `rusty_term_core` reports `AnsiParser::plain` (between sequences, the `SGR 0` pen, US-ASCII in GL, unprotected) and `Grid::plain_cursor` (no origin mode, no insert mode, no open OSC 8 link).
- **Added:** end-to-end tests through a stand-in `ssh` that holds every message 100 ms each way: typing at a shell is drawn ahead underlined once the line's first key is on the screen; nothing typed at a prompt with echo off is ever written; a fast path or `RUSTY_MUX_PREDICT=off` draws nothing.
- **Known limitations:** with any other rendition at the cursor (a coloured prompt that leaves colour set), no predictions are drawn. Resize, reconnect and focus moves reset predictions (unit-tested in build PR 2) but have no end-to-end test.
