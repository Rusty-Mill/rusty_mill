# rusty_term fidelity corpus

Date: 2026-10-21 · Backlog item 14 of `../RUSTY-TERM-PERF-BACKLOG.md` (top-5 #1): the gate that every engine change must pass.

## What it is

The gate and the oracle answer different questions:

| | Gate | Oracle |
|---|---|---|
| Question | Did this change alter any screen? | Is rusty_term right? |
| Where | `cargo test -p rusty_term_core --test fidelity`, run in CI | `sh docs/research/rusty-term-perf/oracle_diff.sh [--diff]`, never in CI |
| Compared against | rusty_term's own committed golden | libghostty-vt's grid for the same recording |
| A difference means | the change altered a screen | rusty_term, libghostty or the harness is wrong; recorded below |

The oracle is libghostty-vt from the phall1 fork, prebuilt; see `ghbench/src/oracle.rs`. Palette indices go through the standard xterm palette, so a theme difference never shows up as a fidelity difference.

**Corpus.** `crates/libs/ui/rusty_term/core/tests/fidelity/corpus/` holds 18 raw 80x24 PTY recordings (`.vt`, 108 KB in total), each with its golden final grid (`.grid`). The golden records text, grapheme clusters, the style runs (colours, attributes, underline style and colour), the cursor, cursor visibility and the alternate-screen flag.

**Gate.** The test fails on any cell change. It also replays every recording one byte at a time and requires the same grid as 4 KiB chunks, since escape sequences straddle PTY reads.

**Recording.** `tests/fidelity/record.py` uses only the Python standard library. It drives the programs below in an 80x24 PTY with scripted keys and answers vttest's DA1 query.
- Recordings depend on the host's program versions, so they are committed rather than re-recorded in CI.
- `.vt` files are marked `binary` in a nested `.gitattributes`. The root `text=auto eol=lf` rule would otherwise strip their `\r` bytes.

| Case | Source | Exercises |
|---|---|---|
| `vttest_1_1`..`1_3` | vttest 2.7, menu 1 | cursor movement, border fill, DECCOLM, autowrap |
| `vttest_2_1`..`2_4` | vttest, menu 2 | tab stops, 132-column mode, screen features |
| `vttest_8_1`..`8_2` | vttest, menu 8 | VT102 insert/delete char and line |
| `vim_edit` | vim, no vimrc, `syntax on`, `number` | insert, split, search, wide text and emoji |
| `less_page` | less | paging, search highlight |
| `htop_self` | htop limited to its own PID | colour bars, the function-key bar, BCE |
| `tmux_split` | tmux, status bar stripped | vertical split, scroll regions, status line |
| `wide_combining`, `sgr_matrix`, `scroll_regions`, `wrap_tabs`, `alt_screen` | printf | CJK/emoji/ZWJ/RTL; every SGR form; DECSTBM/RI/IL; wrap, DECAWM off, EL/ICH/DCH/ECH; alt-screen round trip |

No recording contains host process lists, absolute paths or host names: htop runs as `-p $$`, vim gets a relative path, and the tmux status bar is blanked.

## Oracle results (2026-10-21)

12 of 18 cases matched libghostty-vt cell for cell at first, attributes included, and all six differences were explained. After the BCE fix (Rusty-Mill/rusty_mill#854), 13 of 18 matched; after the width fix (Rusty-Mill/rusty_mill#855), 15 of 18 match. The remaining three are the deliberate DECCOLM divergence below. The table keeps the original findings.

| Case(s) | Difference | Cause | Verdict |
|---|---|---|---|
| `htop_self` | rows 9, 10, 23: the cells after the text keep the default background in rusty_term; libghostty fills them with the SGR background (green, cyan) | **BCE missing.** `EL`, `ECH` and `ED` fill with the *default* colours (`Grid::erase_cell`, `core/src/grid.rs:2826`) instead of the current SGR background. rusty_term's terminfo inherits `xterm-256color`, which advertises `bce`, so ncurses apps rely on it. Reproduced in isolation: `ESC[42m ESC[K`, `ESC[42m ESC[5X` and `ESC[42m ESC[2J` all leave black cells. | **Fixed** (Rusty-Mill/rusty_mill#854): erases now fill with the SGR background; `htop_self` matches the oracle |
| `vim_edit` | stray characters after `👍🏽` and inside `🇯🇵` (two rows) | **Width disagreement with `wcwidth`.** rusty_term folded an emoji modifier or a regional-indicator pair into one cluster: `👍🏽` was 2 columns and `🇯🇵` 1. vim counts each code point: 4 and 4. It gives each regional indicator 2 columns, as libghostty does; glibc's `wcwidth` says 1 and `unicode-width` agrees with glibc. vim's cursor positioning then landed on the wrong cells. | **Fixed** (Rusty-Mill/rusty_mill#855): per-code-point widths by default, grapheme clustering opt-in with DEC mode 2027 (as Ghostty, WezTerm, foot and contour do), and regional indicators 2 columns. `vim_edit` and `wide_combining` now match the oracle. |
| `vttest_1_2`, `vttest_1_3`, `vttest_2_3` | different screens from the first 132-column step on | **DECCOLM** (`CSI ? 3 h/l`, enabled by vttest's `CSI ? 40 h`). libghostty resizes its grid to 132 columns; rusty_term never resizes. | **Deliberate divergence (owner decision, Rusty-Mill/rusty_mill#856). Do not "fix".** In a mux the server owns the PTY size and each client its own viewport, so a program must not force 132 columns. With mode 40 set, DECCOLM clears the screen, resets every margin and homes the cursor, at the current size; with mode 40 reset it is ignored. vttest then draws for 132 columns on an 80-column grid and its lines wrap, so these three cases will keep differing from the oracle. |

## Limits

- **80x24 only.** Resize and reflow are not covered. Those need a recording format with resize events.
- **Final screen only.** Intermediate frames are not compared. Each vttest screen is its own recording instead.
- **Not covered:** scrollback contents, hyperlinks (OSC 8), images (sixel, kitty), titles, mouse modes and line attributes (DECDWL/DECDHL).
- **Recorded on Linux only.** A Windows ConPTY corpus is still owed (spike 1 of `MUX-ADOPT-DECISIONS.md`).
- **The oracle is the fork's libghostty-vt.** Upstream was not built here.

## Deliberate divergences from the oracle

These are intended. A change that makes them match libghostty is a regression, not a fix.

| Cases | rusty_term | libghostty | Why |
|---|---|---|---|
| `vttest_1_2`, `vttest_1_3`, `vttest_2_3` | DECCOLM (mode 3) never resizes the grid. With mode 40 it only clears the screen, resets the margins and homes the cursor. | resizes the grid to 80/132 columns | Rusty-Mill/rusty_mill#856: in a mux the server owns the PTY size and each client its own viewport. |
