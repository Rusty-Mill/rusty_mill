# rusty_prost

The message layer on [`rusty_protobuf`](../rusty_protobuf): no dependencies, no derive, no
code generation. Schemas are hand-written.

- **`Message`:** `encode` (fields in number order, proto3 defaults omitted) and `merge_field`
  (a `match` on the field number). `merge`, `decode`, `decode_with_depth` and `encode_to_vec` are
  provided. Last scalar wins, sub-messages merge, repeated fields append, a oneof's last member
  wins, unknown fields are skipped.
- **`field`:** `message`, `push_message`, `merge_message` and `map_entry`, the helpers sub-messages and
  maps need. `map_entry` takes two closures that each merge **one occurrence** into the entry's key or
  value, so a message value split across occurrences merges into one; a repeated *entry* replaces.
- **`wkt`:** `Empty`, `Timestamp`, `Any` (`pack`/`unpack`/`is`), `Struct` and `Value`, each with a `Name`
  so it can be packed. `Any` matches by the last path segment of the type URL, so another prefix
  still matches and a URL with no `/` names no type. `Value` is an enum;
  a buffer that sets no kind decodes to `Null`. Maps are `BTreeMap`, so output is deterministic.
- **`rpc`:** `Status` and `ErrorInfo`, which `rusty_tonic` carries as error details.
- Enum fields are `i32` on the struct (unknown values round-trip); `Option` is for message fields
  and proto3 `optional`.

Not here: packed/fixed-width helpers (neither consumer has a repeated numeric field; the reader
has `packed_varints` and friends), `Duration`, `FieldMask`, JSON conversion.

**Decoding untrusted input:** the reader cannot panic, over-read or recurse past 100 levels, but
a buffer of many tiny repeated sub-messages decodes to far more memory than its size (as with
`prost`). Bound the input size at the transport.

**Where it differs from `prost` and the reference**
- A known field with the wrong wire type is an error (as in `prost`; the reference treats it as an
  unknown field). `prost` skips that check for map fields; we do not.
- Map entries are always written with both key and value, as the reference does; `prost` drops a
  default key or value. They decode the same. Keys are written in `BTreeMap` order, which is the
  reference's order except that upb puts a key after the keys it is a prefix of.
- Groups and 64-bit-overflowing varints are rejected (see `rusty_protobuf`).

**Rust version:** the library builds on 1.75 (the `rust-version`); its tests need 1.85, the floor of
the `prost` oracle.

Tested against `prost` (a dev-dependency oracle: byte-identical encodings, cross-decoding, and
agreement under random corruption) and buffers recorded from Google's `protobuf` for Python.
