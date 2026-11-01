---
category: Added
changelog: rusty_conhost serves FillConsoleOutput, ScrollConsoleScreenBuffer, the Read/WriteConsoleOutput family and the buffer and window size calls, so cls, Clear-Host and programs that save and restore the screen work on our console host.
---
## 2026-11-01 - Own console host cell and size APIs

- **Added:** our console host (rusty_conhost) serves FillConsoleOutputCharacter and FillConsoleOutputAttribute, ScrollConsoleScreenBuffer (with clip and fill), ReadConsoleOutput, WriteConsoleOutput, Read/WriteConsoleOutputCharacter and Read/WriteConsoleOutputAttribute (#1330, design host PR 6), and SetConsoleScreenBufferSize, SetConsoleWindowInfo, GetLargestConsoleWindowSize and SetConsoleScreenBufferInfoEx (#1333, host PR 7). Cell writes paint the terminal the way conhost under ConPTY does, runs of CUP, SGR and text between DECSC and DECRC, so `cls`, PowerShell's Clear-Host and screen-saving programs work.
- **Changed:** the size calls follow divergence V1, now spelled out: only the window's own size succeeds, and GetLargestConsoleWindowSize reports the window. SetConsoleScreenBufferInfoEx is accepted and changes nothing (V2).
- **Known limitations:** `cls` and Clear-Host clear the screen but not the terminal's scrollback (conhost sends `CSI 3J` for them). A cell write's DECSC/DECRC drops a pending wrap and replaces the client's saved cursor, as it does under ConPTY. Gridline bits other than the top one, and C0 characters, don't read back from cells as written: the host's model is what the terminal was sent. A ReadConsoleOutput larger than 1 MiB fails.
