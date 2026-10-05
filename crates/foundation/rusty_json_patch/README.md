# rusty_json_patch

JSON Pointer (RFC 6901), JSON Patch (RFC 6902) and JSON Merge Patch
(RFC 7386) over `rusty_json::Value`. `no_std` + `alloc`; one first-party
dependency and no external ones (ADR-0002 Tier S, root ADR-0007).

```rust
use rusty_json::json;
use rusty_json_patch::{diff, merge_patch, Patch, Pointer};

// Pointers, parsed once.
let p = Pointer::parse("/a/b~1c")?;            // tokens ["a", "b/c"]

// Patches: wire form in, atomic apply.
let mut doc = json!({"a": {"b/c": 1}});
let patch = Patch::from_value(&json!([{"op": "replace", "path": "/a/b~1c", "value": 2}]))?;
patch.apply(&mut doc)?;                         // all ops or none

// Diffs: the patch that turns one document into another.
let delta = diff(&doc, &json!({"a": {"b/c": 2}, "list": [1]}));
assert_eq!(delta.to_value().to_json_string(), r#"[{"op":"add","path":"/list","value":[1]}]"#);

// Merge patch: overlay an object, `null` deletes.
merge_patch(&mut doc, &json!({"a": null, "x": 1}));
```

Why it exists: `rusty_agui`'s `STATE_DELTA` and `ACTIVITY_DELTA` events
carry RFC 6902 patches, and the workspace had no implementation. `diff`
is deliberately simple (member-wise, index-wise, tail add/remove): the
state deltas it is built for are small objects, not long arrays with
mid-list insertions.

Tests are the RFCs' own appendix examples plus atomicity, root and index
edge cases, and diff round trips.

```
cargo test -p rusty_json_patch
cargo check -p rusty_json_patch --no-default-features
```
