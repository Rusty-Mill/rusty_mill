---
category: Fixed
changelog: rusty_conhost asks the terminal for its cursor after a resize before it answers GetConsoleScreenBufferInfo, as ConPTY does, so PSReadLine's echo and command output no longer drift in rusty_term's window.
---
## 2026-11-02 - rusty_conhost resyncs the cursor with the terminal after a resize

- **Fixed:** after a resize, rusty_term reflows with its scrollback and our console host has none, so the host's cursor row could differ from the terminal's (divergence V3). PSReadLine reads the cursor from GetConsoleScreenBufferInfo and positions its line absolutely, so in `rusty_term --gui` on the own host its echo landed mid-screen, output joined the prompt line when the window widened, and output was overwritten. A resize now marks the cursor as possibly wrong; the next GetConsoleScreenBufferInfo sends DSR 6 to the terminal and is held, with every packet after it, until the terminal's CPR moves the host's cursor or 500 ms pass. This is what ConPTY does (microsoft/terminal GH#18725) (#1375).
- **Known limitations:** only the cursor is resynced. The screen contents the host reports (ReadConsoleOutput) can still differ from the terminal's after a shrink then a grow, as under ConPTY. A terminal that never answers DSR 6 costs the first GetConsoleScreenBufferInfo after each resize 500 ms.
