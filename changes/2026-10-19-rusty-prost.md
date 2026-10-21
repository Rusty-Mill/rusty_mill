---
category: Added
changelog: "`rusty_prost`: a dependency-free Protobuf message layer on `rusty_protobuf` (phase 2 of a first-party prost/tonic): a `Message` trait for hand-written schemas plus `Timestamp`, `Any`, `Struct`, `Value`, `Empty` and `google.rpc` `Status`/`ErrorInfo`. No derive, no code generation."
---
## 2026-10-19 - rusty_prost: Protobuf message layer

- **Why:** phase 2 of the first-party `prost`/`tonic` plan (board cards t43-t46). `rusty_protobuf` is the wire; this is the typed layer the gateway guardrails and `rusty_a2a` use `prost` for today. Design approved before any code (decisions 1-6 on card t43).
- **Added:** `crates/libs/protocol/rusty_prost`, Tier S (the only dependency is `rusty_protobuf`; `prost` and `prost-types` are dev-dependency oracles).
  - `Message`: `encode` and `merge_field` are the two methods an author writes; `merge`, `decode`, `decode_with_depth` and `encode_to_vec` are provided. Last scalar wins, sub-messages merge, repeated fields append, a oneof keeps its last member, unknown fields are skipped. `Name` carries the type URL for `Any`.
  - `field::{message, push_message, merge_message, map_entry}`: the helpers sub-messages and maps need. `map_entry` merges each occurrence into the entry's key or value (a message value split across occurrences becomes one), and a repeated entry replaces.
  - `wkt`: `Empty`, `Timestamp`, `Any` (`pack`/`unpack`/`is`, matching the type URL by its last path segment), `Struct`, and `Value` as an enum (a buffer that sets no kind decodes to `Null`), each with a `Name`. `rpc`: `Status` and `ErrorInfo`. Maps are `BTreeMap`.
- **Tests:** 46, against two independent references.
  - `prost` 0.14 as an oracle: identical bytes on 3,000 random messages of a 25-field schema, each side reads the other's output, and 24,000 corrupted inputs agree on accept/reject and on the message read (5,396 both accept, 18,598 both reject, 6 where we are deliberately stricter, each explained).
  - Buffers recorded from Google's `protobuf` for Python for every well-known type and `google.rpc`, byte-identical on re-encode (`tests/fixtures/gen.py` reproduces them).
  - The gateway's eight `ext_mcp` messages and a slice of the A2A schema (a oneof holding a `Value`, a map of messages, `optional int32`, an enum, a `Timestamp`) re-written on the trait and compared with the `prost`-derived originals. This is the proof the trait is enough for both consumers.
  - Review-driven additions: repeated map values merge, a oneof message member merges when its variant repeats, `Any` identity (custom prefixes, wrong names, malformed URLs).
  - Merge rules, depth limit, and a 5,000-level `Value` that must not overflow the stack. Ten mutations (merge, oneof, zero-valued kinds, URL check, append, map-key rules, depth, details) each fail a test.
- **Found while testing:** `prost` does not check the wire type of a map field (a varint-typed one is read as an entry length); upb orders a map key after keys it is a prefix of. Neither affects meaning; both are in the README.
- **Known limitations:** no derive, macro or generator, so a consumer's schema is hand-written (the gateway's eight messages are the measure of that cost; `rusty_a2a`'s 44 come with the migration). No packed/fixed-width/`sint` helpers (neither consumer has a repeated numeric field; the reader has them), no `Duration` or `FieldMask`, no JSON conversion. A buffer of many tiny repeated sub-messages decodes to far more memory than its size, as with `prost`: bound the input size at the transport (`rusty_tonic`). The library builds on its declared `rust-version` 1.75 (checked with rustc 1.75.0, together with `rusty_protobuf`'s 51 tests); `rusty_prost`'s own tests need 1.85, the floor of the `prost` oracle.
