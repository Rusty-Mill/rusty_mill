---
category: Fixed
changelog: rusty_term lays out text per code point by default (as vim and other wcwidth-based apps do), with grapheme clustering opt-in via DEC mode 2027; regional indicators are two columns.
---
## 2026-10-22 - rusty_term: wcwidth widths by default, grapheme clustering via mode 2027

- **Fixed:** rusty_term always merged a whole grapheme into one cell, so `👍🏽` (base and skin tone) took 2 columns and `🇯🇵` 1. Apps that count one code point at a time, vim among them, count 4 and 4, so their cursor positioning landed on the wrong cells and left stray characters on screen. Now, by default:
  - each non-zero-width code point takes its own columns;
  - zero-width ones (combining marks, ZWJ, variation selectors) attach to the glyph on their left;
  - regional indicators are 2 columns, as in vim and Ghostty. glibc's `wcwidth` says 1.
  Closes Rusty-Mill/rusty_mill#855.
- **Added:** DEC private mode 2027 (`CSI ? 2027 h/l`, reported by DECRQM) switches on the previous one-cell-per-grapheme behaviour, as in Ghostty, WezTerm, foot and contour. RIS resets it; DECSTR keeps it, as Ghostty does. Under the mode, a flag is one 2-column glyph.
- **Changed:** `unicode_heavy` throughput roughly doubled (about 13 to 24 MB/s locally), because the default path no longer runs UAX #29 segmentation per character. The `vim_edit` and `wide_combining` fidelity goldens changed in their emoji rows and now match libghostty-vt (15 of 18 cases).
- **Known limitations:** ZWJ sequences such as `👨‍💻` render as their parts by default, as in xterm. Apps that want joined emoji enable mode 2027. There is no config key to make clustering the default yet.
