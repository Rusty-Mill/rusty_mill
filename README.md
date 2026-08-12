# rusty_json

A from-scratch JSON library for Rust. `no_std` + `alloc` by default, with a
default-on `std` feature for ergonomics.

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
```

See [`ROADMAP.md`](ROADMAP.md) for the phased parity plan (currently Phase 1:
an RFC 8259-compliant core), and [`gap-analysis.md`](gap-analysis.md) for the
tracked gap list this phase was built from.