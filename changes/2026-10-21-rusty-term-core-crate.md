---
category: Changed
changelog: Split rusty_term's VT engine into the headless rusty_term_core crate (no tokio, PTY or renderer), re-exported as rusty_term::core.
---
## 2026-10-21 - rusty_term_core: headless VT engine crate

- **Changed:** `crates/libs/ui/rusty_term/src/core/` moved, history kept, to a new workspace crate `rusty_term_core` at `crates/libs/ui/rusty_term/core/`. Without features it depends only on `rusty_base64`, `rusty_hex`, `unicode-width` and `unicode-segmentation`. `rusty_term` re-exports it as `rusty_term::core`, so dependents (`mill-term`, `rusty_boot`, the fuzz targets) need no change. Prerequisite 1 of `docs/research/MUX-ADOPT-DECISIONS.md`, approved with the performance backlog.
- **Changed:** `rusty_term`'s `gui`, `l13` and `web-bridge` features forward to the core features of the same names. Items the front-ends used through `pub(crate)` are now `pub`: the `Grid` fields `clusters` and `kitty_images`, `KittyImage`, `KittyFrame`, `GridImage`, `CommandBlock`, `png::Image`/`decode`, `ensure_contrast`, `bidi_mirrored` and a few `Grid` methods. Added `Grid::scrollback_len`.
- **Known limitations:** the core's `l13` feature still pulls tokio through `rusty_lsp`. The parser's XTVERSION reply now reads the core crate's version, kept at 0.1.1 in lockstep with `rusty_term`. No behaviour change: the 730 tests pass unchanged (269 rusty_term + 445 core + 16 binary, the same 730 as before the move).
