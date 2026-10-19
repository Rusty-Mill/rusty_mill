---
category: Added
changelog: "`rusty_protobuf`: a dependency-free Protobuf wire-format runtime (phase 1 of a first-party prost/tonic): a zero-copy, bounds-checked reader and a length-fixing writer, no code generation, schemas hand-written on top."
---
## 2026-10-19 - rusty_protobuf: Protobuf wire-format runtime

- **Why:** `rusty_flatbuffers` covers FlatBuffers; the other half of the board's "home-grown serialization" card (t06) is Protobuf, which `rusty_a2a` and the agent gateway's guardrails use through `prost`. This is the wire layer only; `rusty_prost` (message layer) and `rusty_tonic` (gRPC) are separate follow-ons.
- **Added:** `crates/libs/protocol/rusty_protobuf`, Tier S (no dependencies, `forbid(unsafe_code)`).
  - `Reader`: an iterator of `Field { number, value }` over a borrowed buffer, with typed accessors (`int32`, `sint64`, `str`, `message`, `packed_varints`, ...). Every access is bounds-checked and returns `Error`; it fuses after the first error, never allocates from a claimed length, and refuses nesting past 100 levels (`with_depth_limit` to change it).
  - `Writer`: front to back, `message(n, |w| ..)` fixes up the length prefix (one byte reserved, shifted only for bodies of 128 bytes or more); `finish()` reports an invalid field number or a message over 2 GiB.
  - Zigzag helpers for packed `sint` runs.
- **Tests:** buffers recorded from Google's `protobuf` for Python (upb) as hex, with `tests/fixtures/gen.py` to reproduce them: decode and byte-identical re-encode for every scalar type, nested, repeated, packed, map, oneof and enum cases; 21 malformed buffers each checked against the reference's verdict; every-prefix truncation, seeded random corruption and random bytes; depth, varint and packed-length limits; writer framing. Mutation-checked: removing the varint-overflow, depth, length-fixup, fuse, group and packed-length rules each fails a test.
- **Known limitations:** groups (wire types 3 and 4) are rejected, which is stricter than the reference. A varint that overflows 64 bits is rejected (the reference accepts it, `prost` and Go reject it). Unknown fields are yielded and skipped, not preserved for re-encoding. There is no schema, derive or codegen: those are `rusty_prost` and, if a schema outgrows hand-writing, a later generator.
