# rusty_protobuf

A small Protobuf runtime with no dependencies and no code generation.

- **Read:** `Reader::new(buf)` yields each `Field { number, value }` in wire order, zero-copy.
  Typed accessors (`int32`, `sint64`, `str`, `message`, `packed_varints`, ...) check the wire type.
  Every access is bounds-checked and returns `Error`; hostile input cannot panic, read outside
  the buffer, or make the reader allocate from a length it claims.
- **Write:** `Writer` appends front to back. `message(number, |w| ...)` writes a sub-message and
  fixes up its length prefix. `finish()` reports an invalid field number or a message over 2 GiB.
- **Schemas** are hand-written on top (a decoder matches on `Field::number`, an encoder calls
  `Writer`). Unknown fields are yielded and skipped, not preserved. Groups are rejected.
- **Limits:** nesting depth 100 by default (`Reader::with_depth_limit`), varints of at most 10
  bytes that fit 64 bits, field numbers 1..=2^29-1.

Tested against buffers recorded from Google's `protobuf` for Python (upb), as hex in
`tests/fixtures/`; `tests/fixtures/gen.py` reproduces them. The reference accepts a few
things this crate rejects on purpose (a varint overflowing 64 bits, well-formed groups); those
rows are marked `strict_` in the fixtures.
