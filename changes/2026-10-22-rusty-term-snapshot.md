---
category: Added
changelog: rusty_term_core can encode the whole drawable terminal state, scrollback included, into a compact versioned snapshot and rebuild a grid from it, the first piece of a mux client attach.
---
## 2026-10-22 - rusty_term: full grid snapshot encoder and decoder

- **Added:** `Grid::encode_snapshot` and `Grid::decode_snapshot`. A snapshot carries what a client needs to draw the terminal: the size, the cursor, the scroll region, autowrap, the title, the grapheme-cluster and hyperlink tables, scrollback, the live screen and, while the alternate screen is active, the stashed primary screen. It does not carry parser state, input modes, the palette or images. The format is versioned (`RTSN` magic plus a version byte), so a later cell-layout change can bump it.
- **Added:** each line is stored as runs of one style. A style is defined the first time it is used and referenced by id after that, and repeated identical cells (blank tails) collapse to a single repeat run. Plain text costs one byte per ASCII cell.
- **Added:** the decoder treats its input as untrusted. It reads through `rusty_wire`'s bounds-checked reader, checks every count, index, character and dimension, rejects overflowing varints, and caps the decoded size at 2^25 cells and 2^20 lines. It builds the grid only after the whole input has been validated, so a truncated header allocates no screen. Tests feed it every truncated prefix and every single-byte corruption of a snapshot.
- **Changed:** for 200x50 plus 10,000 history lines, a snapshot is 2.09 MB (1.04 B/cell) against libghostty-vt's 7.81 MB. Encoding took about 9.3 ms against libghostty-vt's 9.6 ms in one container, and 11.5-14.3 ms against 9.3-11.0 ms on a later host, so it is not reliably faster. Decoding takes about 26-34 ms. Every fidelity-corpus screen survives a round trip unchanged.
- **Known limitations:** full snapshots only. Incremental updates (a per-row change counter and stable row numbers) come with the mux protocol that consumes them.
