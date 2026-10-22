---
category: Added
changelog: rusty_term_core fidelity gate: 18 recorded vttest/vim/less/htop/tmux/synthetic sessions with golden grids, checked in CI, plus a libghostty-vt oracle comparison.
---
## 2026-10-21 - rusty_term fidelity corpus

- **Added:** `crates/libs/ui/rusty_term/core/tests/fidelity.rs`, which replays 18 raw 80x24 PTY recordings (`tests/fidelity/corpus/*.vt`, 108 KB) and fails on any change to the final grid: text, grapheme clusters, colours, attributes, underline style and colour, cursor, alt screen. It also requires byte-at-a-time replay to match 4 KiB chunks. This gates the planned scrolling and printing rewrites. Regenerate with `UPDATE_GOLDEN=1`.
- **Added:** `tests/fidelity/record.py` (stdlib only) records vttest menus 1/2/8, vim, less, htop (own PID only), tmux and printf edge cases.
- **Added:** `docs/research/rusty-term-perf/FIDELITY.md` and `oracle_diff.sh`. 12 of 18 cases match libghostty-vt cell for cell. The differences are rusty_term bugs: missing background colour erase (BCE), and emoji-modifier and flag widths that disagree with `wcwidth`, which breaks vim. DECCOLM is unhandled, side effects included.
- **Known limitations:** 80x24 final screens only; no resize, scrollback, hyperlink or image coverage; Linux recordings only. The goldens record today's behaviour, bugs included.
