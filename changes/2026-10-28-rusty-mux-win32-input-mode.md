---
category: Fixed
changelog: On Windows the rusty-mux client's Ctrl-] commands work again; the client no longer lets a block's pseudo console switch the local terminal to win32-input-mode.
---
## 2026-10-28 - rusty_mux client keeps the local terminal out of win32-input-mode

- **Fixed:** on Windows, Ctrl-] reached the block's shell as `^]` instead of starting a client command, so detach and every layout command were unusable (manual check, #1212). The likely cause, traced from source but not yet reproduced on Windows: a Windows server's pseudo console asks for win32-input-mode (`?9001h`) when it starts, the client relayed the request, and Windows Terminal then sent Ctrl-] as an encoded key event the client did not recognise. The client now drops 9001 from the mode sets in a block's passed-through output, holding a set split across reads until it completes so other bytes pass unchanged. It also never sets the mode when switching blocks, and turns it off on exit. Ctrl-] still needs a retest in Windows Terminal.
- **Added:** `RUSTY_MUX_KEYLOG=<path>` writes each keyboard read to a new file as a line of hex, to see what a terminal really sends. It records everything typed, passwords included, and says so on start. The file must not exist yet. On Unix it is created owner-only (0600); on Windows it takes its directory's permissions.
- **Known limitations:** a mode set split across reads after more than 64 bytes keeps its 9001; no real program sends one that long. Keys reach Windows programs as plain VT, so a console program loses key-up events and some modifier detail that win32-input-mode would have carried.
