---
category: Fixed
changelog: On Windows the rusty-mux client's Ctrl-] commands work again; the client no longer lets a block's pseudo console switch the local terminal to win32-input-mode.
---
## 2026-10-28 - rusty_mux client keeps the local terminal out of win32-input-mode

- **Fixed:** on Windows, Ctrl-] reached the block's shell as `^]` instead of starting a client command, so detach and every layout command were unusable (manual check, #1212). A Windows server's pseudo console asks for win32-input-mode (`?9001h`) when it starts, and the client relayed the request. Windows Terminal then sent Ctrl-] as an encoded key event the client did not recognise. The client now turns the mode off after passing a block's output through, never sets it when switching blocks, and turns it off on exit.
- **Added:** `RUSTY_MUX_KEYLOG=<path>` appends each keyboard read to that file as a line of hex, to see what a terminal really sends.
- **Known limitations:** keys reach Windows programs as plain VT, so a console program loses key-up events and some modifier detail that win32-input-mode would have carried.
