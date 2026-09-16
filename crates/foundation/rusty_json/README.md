# rusty_json

A from-scratch JSON library for Rust: parser, serializer, and a `Value`
tree, plus full `serde::Serialize`/`Deserialize` integration so it works as
a drop-in JSON *format* for any serde-derived type, not just its own
`Value`. `no_std` + `alloc` by default, with a default-on `std` feature for
ergonomics (`Display`/`Error` impls, `to_writer`/`from_reader`).

```rust
let value: rusty_json::Value = rusty_json::from_str(r#"{"a": [1, 2, true]}"#).unwrap();
assert_eq!(value["a"][1].as_i64(), Some(2));
assert_eq!(rusty_json::to_string(&value).unwrap(), r#"{"a":[1,2,true]}"#);

// `to_string`/`to_string_pretty`/`from_str`/`from_slice` work over any
// `serde::Serialize`/`Deserialize` type, not just `Value` -- including
// `#[derive(Serialize, Deserialize)]` structs/enums.
#[derive(serde::Serialize, serde::Deserialize, PartialEq, Debug)]
struct Point { x: i32, y: i32 }

let json = rusty_json::to_string(&Point { x: 1, y: 2 }).unwrap();
assert_eq!(json, r#"{"x":1,"y":2}"#);
assert_eq!(rusty_json::from_str::<Point>(&json).unwrap(), Point { x: 1, y: 2 });

// `to_value`/`from_value` convert directly to/from `Value`, without a JSON
// text intermediate.
let value = rusty_json::to_value(&Point { x: 1, y: 2 }).unwrap();
assert_eq!(value["x"], 1);
assert_eq!(rusty_json::from_value::<Point>(value).unwrap(), Point { x: 1, y: 2 });

// `json!` builds a `Value` tree from literal syntax.
let built = rusty_json::json!({ "ok": true, "items": [1, 2, 3] });
assert_eq!(built["items"][1], 2);
```

## What's here

- `Value`: `Null`/`Bool`/`Number`/`String`/`Array`/`Object`, with `get`/
  indexing/`as_*`/`is_*` accessors, mutable counterparts, [RFC 6901](https://www.rfc-editor.org/rfc/rfc6901)
  JSON Pointer lookup (`pointer`/`pointer_mut`), and `From`/`FromIterator`
  conversions.
- `Map`: a real newtype over a sorted map (`.entry()` API, `Keys`/`Values`/
  `Iter`/`IntoIter`/etc.), iterating in sorted key order -- matching
  `serde_json::Map`'s default (non-`preserve_order`) behavior. There's no
  insertion-order-preserving mode.
- `Number`: preserves `u64`/`i64`/float representation distinctly, so
  integers round-trip exactly instead of going lossily through `f64`;
  128-bit conversions included.
- A hand-rolled parser/serializer (`from_str`/`from_slice`/`to_string`/
  `to_string_pretty`), position-aware syntax errors (`Error::line`/`column`/
  `classify`), and a pluggable `Formatter` trait (`CompactFormatter`/
  `PrettyFormatter`) for custom output shapes.
- Full `serde::Serialize`/`Deserialize` support: `Value`/`Map` implement both
  traits against *real* `serde` (verified against `serde_json`'s own
  serializer/deserializer in tests), and every top-level function
  (`to_string`/`from_str`/`to_vec`/`from_slice`/`to_value`/`from_value`/
  `to_writer`/`from_reader`) is generic over any `Serialize`/`Deserialize`
  type, not just `Value`.
- `StreamDeserializer` for reading multiple whitespace-separated JSON values
  from one source.
- The `json!` declarative macro for building `Value` trees from Rust literal
  syntax.

## Known limitations

- No `RawValue` (deferred, unstarted).
- No arbitrary-precision number support -- `Number` is a fixed
  `u64`/`i64`/`f64` representation.
- `Map` has no insertion-order-preserving mode -- always sorted by key.
- The optional `rusty_json-derive` crate in this workspace
  (`#[derive(RustyJson)]`) is an early stub: it doesn't serialize a struct's
  actual fields and its deserialize path always errors. It isn't wired into
  this crate's public API (`rusty_json` doesn't re-export it) -- don't
  depend on it yet.

See [`ROADMAP.md`](ROADMAP.md) for the phased parity plan and
[`gap-analysis.md`](gap-analysis.md) for the `serde_json`-parity audit this
phase's additions were built from (fully closed as of this writing).
