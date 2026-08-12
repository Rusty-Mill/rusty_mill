# Roadmap

`rusty_json` is a from-scratch JSON library for Rust. This document is the
scope-of-record for what "parity" means at each stage — the `parity-loop`
skill audits against this file before generating any new gap list.

## Phase 1 — RFC 8259-compliant core (done)

A minimal, correct JSON implementation, independent of `serde_json`'s exact
API shape:

- `Value` enum (`Null`, `Bool`, `Number`, `String`, `Array`, `Object`)
- Parser: `&str`/bytes → `Value`, spec-correct per [RFC 8259](https://www.rfc-editor.org/rfc/rfc8259)
  (literals, numbers, string escapes incl. `\uXXXX` and surrogate pairs,
  arrays, objects, whitespace), with position-aware errors
- Serializer: `Value` → compact string, plus a pretty-printed form
- `Value` accessors (`get`, indexing, `as_*`/`is_*`, `From` conversions)
- Design target: `no_std` + `alloc` where feasible, with a default-on `std`
  feature for ergonomics (`Display`/`Error` impls, convenience I/O) — mirrors
  `serde_json`'s own `alloc` feature split.

## Phase 2 — `serde_json` API parity (in progress, most items shipped)

Re-assessed by diffing `rusty_json`'s public API against a pinned
`serde_json` version (`cargo public-api`), symbol by symbol — see
[`gap-analysis.md`](gap-analysis.md) for the full audit record.

**Shipped:**

- `Serialize`/`Deserialize` traits, integrated against real `serde`
  (`Value`/`Map` implement both; every top-level function is generic over
  any `Serialize`/`Deserialize` type)
- Streaming `Serializer`/`Deserializer` (`to_writer`/`to_writer_pretty`,
  `from_reader`), plus `to_vec`/`to_vec_pretty`/`from_slice`
- `to_value`/`from_value` — convert directly to/from `Value` without a JSON
  text intermediate (closed the last gap flagged by `rusty_search`'s
  dependency audit, [`rusty_json#45`](https://github.com/baileyrd/rusty_json/issues/45))
- `StreamDeserializer` for reading multiple whitespace-separated values
- A pluggable `Formatter` trait (`CompactFormatter`/`PrettyFormatter`)
- `Map` as a real newtype with `.entry()` API and iterator views (`Iter`,
  `IterMut`, `Keys`, `Values`, `ValuesMut`, `IntoIter`, `IntoValues`),
  matching `serde_json::Map`'s default (sorted, non-`preserve_order`)
  ordering — resolved as sorted-only, no insertion-order-preserving mode
- `json!` macro for building `Value` trees from Rust literal syntax
- The full "Round 1: pure additions" list from `gap-analysis.md` (accessor/
  conversion/error-classification parity — see that file for the itemized
  list, all closed)

**Remaining:**

- `RawValue` (behind `serde_json`'s non-default `raw_value` feature) —
  unstarted
- Arbitrary-precision numbers (`serde_json`'s `arbitrary_precision`
  feature) — unstarted; would change `Number`'s internal representation
- `Value::sort_all_objects` — `serde_json`'s `preserve_order` interop
  no-op/normalizer. Not meaningful here: `Map`'s ordering was resolved as
  sorted-only (no `preserve_order` equivalent exists to normalize against),
  so this stays a deferred, low-value addition rather than a blocked one.

## Out of scope (this round)

- Anything not reachable from `no_std` + `alloc` unless explicitly gated
  behind the `std` feature.
- An insertion-order-preserving `Map` mode (`serde_json`'s `preserve_order`
  feature, `IndexMap`-backed) — not planned; `Map`'s sorted-only shape is a
  deliberate choice, not a placeholder.
