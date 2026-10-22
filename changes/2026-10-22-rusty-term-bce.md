---
category: Fixed
changelog: rusty_term erases (EL, ED, ECH, ICH, DCH, IL, DL, SU, SD and scrolling) now fill with the current SGR background (BCE), as the terminfo it inherits from xterm-256color advertises.
---
## 2026-10-22 - rusty_term: background colour erase (BCE)

- **Fixed:** erasing with a background colour set (`ESC[42m` then `ESC[K`, `ESC[2J`, `ESC[5X`, and similar) filled with the default background. Programs that rely on the advertised `bce` capability (htop, tmux, ncurses colour schemes) showed black gaps after coloured text. Erased and scrolled-in cells now take the current SGR background, with the default foreground and no attributes, as in xterm, Ghostty and Alacritty. Resize and theme fills stay default; RIS and DECSTR reset the erase colour with the pen. Closes Rusty-Mill/rusty_mill#854.
- **Changed:** the `htop_self` fidelity golden gained three background runs, and that case now matches libghostty-vt.
