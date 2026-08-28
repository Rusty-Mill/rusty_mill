# gap-analysis.md — Phase 2, round 1 (serde_json API parity)

**Status: fully closed.** Every item below — including both "stop-and-ask"
items — has shipped. This file is kept as the historical audit record (per
`ROADMAP.md`'s note that `parity-loop` reads it before generating a new gap
list); it no longer represents open work. See `ROADMAP.md`'s "Remaining"
section for what's actually still open (`RawValue`, arbitrary-precision
numbers).

Originally assessed via `cargo public-api` (source: `diff`), diffing this
crate's public surface against `serde_json` **1.0.151** (pinned for that
run, default features) by symbol name, per `ROADMAP.md`. Raw diff output
filtered for tooling noise: `PartialEq::eq`/`Iterator::Item`/
`Serializer::Ok`/`FromStr::Err` associated-type/method artifacts collapsed
into the one real row they represent (or dropped where there was no real
gap behind them).

## Round 1: pure additions (no new dependency, no breaking change) — done

| Symbol | Category | Source | Platforms | Reference | Breaking? | Est. size | Status | Notes |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| `Value::is_i64`/`is_u64`/`is_f64` | fn (existing type) | diff | core | `serde_json::Value` | no | S | ✅ shipped | Delegate to existing `Number::is_*` |
| `Value::as_null` | fn (existing type) | diff | core | `serde_json::Value::as_null` | no | S | ✅ shipped | Returns `Option<()>` |
| `Value::get_mut`/`as_array_mut`/`as_object_mut` | fn (existing type) | diff | core | `serde_json::Value` | no | M | ✅ shipped | Mutable counterparts to existing read-only accessors |
| `IndexMut<&str>`/`IndexMut<usize>` for `Value` | fn (existing type) | diff | core | `serde_json::Value` | no | M | ✅ shipped | Matches serde_json's auto-vivify: missing object key inserts `Null`; indexing a `Null` value promotes it to an empty `Object` first. Panics on index-out-of-bounds for arrays and on non-object/array+non-null, same as serde_json |
| `Value::take` | fn (existing type) | diff | core | `serde_json::Value::take` | no | S | ✅ shipped | `mem::take`-style: replace with `Null`, return old value |
| `Number::as_i128`/`as_u128`/`from_i128`/`from_u128` | fn (existing type) | diff | core | `serde_json::Number` | no | S | ✅ shipped | 128-bit variants alongside existing 64-bit ones |
| `PartialEq<T> for Value` (and reverse) | fn (existing type) | diff | core | `serde_json::Value`'s primitive `PartialEq` impls | no | M | ✅ shipped | `bool`/`i64`/`u64`/`f64`/`&str`/`String`, both directions — lets `value == "foo"` work directly |
| `pub type Result<T>` | type | diff | core | `serde_json::Result` | no | S | ✅ shipped | `Result<T> = core::result::Result<T, Error>` convenience alias |
| `Error::Category` + `classify`/`is_syntax`/`is_io`/`is_eof`/`is_data` | type + fn | diff | core | `serde_json::error::Category`, `serde_json::Error` | no | M | ✅ shipped | `Io`/`Syntax`/`Data`/`Eof` classification; our parser only ever produces `Syntax` today, `Eof` distinguishable from "unexpected end of input" cases |
| `Value::pointer`/`pointer_mut` | fn (existing type) | diff | core | `serde_json::Value::pointer`, [RFC 6901](https://www.rfc-editor.org/rfc/rfc6901) | no | M | ✅ shipped | JSON Pointer lookup |
| `FromIterator<(String, Value)>`/`FromIterator<Value>` for `Value` | fn | diff | core | `serde_json::Value`'s `FromIterator` impls | no | S | ✅ shipped | Object- and array-from-iterator construction |
| `impl FromStr for Value` | fn | diff | core | `serde_json::Value` (`str::parse`) | no | S | ✅ shipped | Thin wrapper over existing `from_str` free function |
| `json!` macro | macro | diff | core | `serde_json::json!` | no | M | ✅ shipped | Declarative macro building `Value` trees from Rust literal syntax; doesn't need serde, builds on existing `From` impls |

## Beyond round 1: also shipped

| Symbol | Status | Notes |
| --- | --- | --- |
| `to_value`/`from_value` | ✅ shipped | Convert directly to/from `Value` without a JSON text intermediate. Not in the original round-1 diff — added in response to [`rusty_json#45`](https://github.com/baileyrd/rusty_json/issues/45), `rusty_search`'s dependency-audit ask, once `Value`'s serde integration (below) made it straightforward |

## Deferred to a later round

| Symbol | Category | Source | Platforms | Reference | Breaking? | Est. size | Status | Notes |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| `RawValue` | type | roadmap | std | `serde_json::value::RawValue` | no | L | ⏳ not started | Behind serde_json's non-default `raw_value` feature; not in the original round's diff (default-features only). Still a roadmap item |
| Arbitrary-precision numbers | fn (existing type) | roadmap | std | serde_json's `arbitrary_precision` feature | maybe | L | ⏳ not started | Would change `Number`'s internal representation |
| `Value::sort_all_objects` | fn (existing type) | diff | core | `serde_json::Value::sort_all_objects` | no | — | ⏳ not started, unblocked | serde_json's `preserve_order` interop no-op/normalizer. The original blocker ("not meaningful until `Map` has an ordering choice") is now resolved — `Map` is sorted-only, no `preserve_order` equivalent — so this is a legitimate no-op to add if ever wanted, just low value given there's nothing to normalize against |

## Stop-and-ask items — both resolved, both shipped

These showed up in the original diff but failed the loop's "pure addition
only" bar at the time — flagged instead of auto-implemented, per
parity-loop's rules. Both have since been explicitly approved and shipped.

1. **serde `Serialize`/`Deserialize` integration** (`Serializer`,
   `Deserializer`, `to_writer`/`to_vec`/`from_reader`/`from_slice` generic
   over `T: Serialize`/`Deserialize`, `Formatter`/`CompactFormatter`/
   `PrettyFormatter`, `StreamDeserializer`, `Error` implementing
   `serde::de::Error`/`serde::ser::Error`) — ✅ shipped. Required adding
   `serde` as a new third-party dependency (`default-features = false,
   features = ["alloc"]`); approved and landed as the headline Phase 2 item.
2. **`Map` becoming a real newtype with `.entry()`/iterator views**
   (`Entry`, `OccupiedEntry`, `VacantEntry`, `Keys`, `Values`, `ValuesMut`,
   `Iter`, `IterMut`, `IntoIter`, `IntoValues`) instead of a bare
   `BTreeMap<String, Value>` alias — ✅ shipped, breaking change approved.
   The bundled ordering question was resolved as sorted-only (matching
   `serde_json::Map`'s default, non-`preserve_order` behavior) — no
   insertion-order-preserving mode was added.
