---
category: Changed
changelog: rusty_term's `Grid::row` returns cells by value and `Grid::row_mut` is removed, so cell storage can be compacted behind the accessors; RTCP and RTSN output for every fidelity recording is now pinned by hash.
---
## 2026-10-25 - rusty_term: by-value cell accessors and pinned wire bytes

- **Changed:** `Grid::row(y)` returns `Vec<Cell>` instead of `&[Cell]`; empty past the last row, as before. This is PR 1 of the compact cell plan (backlog row 5, owner sign-off 2026-10-25), which stores cells as 8 bytes plus a style table and needs readers to take cells by value.
- **Removed:** `Grid::row_mut`. The GUI overlay fills rows with `window.rs`'s existing `fill_row` helper, and the accessibility test writes through `Grid::set_cell`.
- **Changed:** viewport cell reads (`cell_of`, for history and live rows) go through one crate-private `Grid::line_cell`. A column past a live row's width now reads as a blank instead of panicking, as history rows already did.
- **Tested:** `wire_bytes_match_pinned_hashes` hashes (FNV-1a 64) the RTCP v3 checkpoint and RTSN v1 snapshot of every fidelity recording, plus four synthetic inputs for OSC 8 links, DECSCA with DECSED, scrollback and a grapheme cluster in two pens. The hashes live in `tests/fidelity/corpus/wire.hashes` and regenerate with `UPDATE_GOLDEN=1`. Each input is encoded twice 2 ms apart and must match, so no timer age can enter a pinned hash. `dump_pins_wide_trailer_columns` builds its moved trailer with `set_cell`. `row_mut_write_is_cleared` is deleted along with `row_mut`.
- **Known limitations:** `Grid::row` now allocates one `Vec` per call. Its callers are tests, the accessibility tree and a benchmark assertion; no frame path uses it.
