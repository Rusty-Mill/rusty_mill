---
category: Changed
changelog: rusty_term's terminal mode writes each frame in one write and sends 4–21% fewer bytes, by stating only the part of a style that changed.
---
## 2026-10-26 - Smaller terminal-mode frames

The terminal (TUI) renderer used to write each row separately, start every style change from a reset with the full 24-bit SGR, and end every row with its own reset. It now builds the whole frame in one buffer and writes it once. A style change states only the attributes and colors that changed, and falls back to a reset only when an attribute is turned off. The pen carries from row to row, and one reset closes the frame. Equal styles also compare equal now when they differ only in the style bits of a hidden underline. On full 120x40 repaints after each bench workload, the bytes sent drop by 21% on plain screens and by 4–15% on the color-heavy ones. A property test feeds random chains of style changes through the real parser and checks that each cell comes back with the style it was given, and a tmux capture of a colored screen is identical before and after.
