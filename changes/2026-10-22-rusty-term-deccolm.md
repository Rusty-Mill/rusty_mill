---
category: Fixed
changelog: rusty_term honours DEC mode 40; with it set, DECCOLM clears the screen, resets margins and homes the cursor without resizing.
---
## 2026-10-22 - rusty_term: DECCOLM side effects behind mode 40, never a resize

- **Fixed:** DEC private mode 3 (DECCOLM) was ignored entirely. Mode 40 (allow 80/132 column switching) is now honoured and off by default. With mode 40 set, `CSI ? 3 h/l` clears the screen, resets the top/bottom and left/right margins and homes the cursor. With mode 40 reset, DECCOLM is ignored. DECRQM reports both modes, and RIS resets them. Closes Rusty-Mill/rusty_mill#856.
- **Deliberate:** the grid is never resized to 80 or 132 columns. In a mux the server owns the PTY size and each client its own viewport. Ghostty resizes its grid here; xterm only does so with `allowColumnMode`.
- **Changed:** the `vttest_1_3` fidelity golden now shows the screen DECCOLM clears. `vttest_1_2`, `vttest_1_3` and `vttest_2_3` stay different from libghostty-vt by design; `FIDELITY.md` records them as deliberate divergences.
