# rusty_flatbuffers

A small FlatBuffers runtime with no dependencies and no code generation.

- **Read:** `Table::root(buf)`, then `scalar`, `string`, `table`, `vector`, `struct_bytes` by slot
  (the field's declaration index). Every access is bounds-checked and returns `Error`; hostile
  input cannot panic or read outside the buffer.
- **Build:** `Builder` writes back to front. Create strings, vectors and sub-tables first, then
  `start_table()`, `add_*` and `finish()`; `Builder::finish(root, id)` closes the buffer. A union is a
  `u8` type slot plus an offset slot.
- **Schemas** are hand-written on top (a decoder reads slots, an encoder adds them). A generator is
  planned once a schema outgrows that; see `crates/apps/rocket_league/rlbot/PLAN.md`.

Tested against buffers made by `rlbot_flat` (planus), recorded as hex in `tests/fixtures/`.
