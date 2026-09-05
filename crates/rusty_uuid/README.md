# rusty_uuid

Minimal, dependency-free UUID v4 (random) generation for Rust, per RFC 4122.

```rust
let id = rusty_uuid::Uuid::new_v4();
println!("{id}"); // e.g. "550e8400-e29b-41d4-a716-446655440000"
```

## Why

Some call sites only need a random 128-bit id with the standard hyphenated
hex `to_string()` form — for example, assigning an id to a document before
indexing it:

```rust
let id = document.id.clone().unwrap_or_else(|| rusty_uuid::Uuid::new_v4().to_string());
```

`rusty_uuid` covers that case with zero external dependencies. Randomness
comes straight from the OS: `/dev/urandom` on Unix, `BCryptGenRandom` on
Windows — both non-blocking on modern platforms — via `rusty_rand`, the
sibling workspace crate (same zero-external-dependency posture) that was
extracted from this crate's own former copy of that plumbing.

## Features

- `Uuid::new_v4()` — random UUID with the version/variant bits set correctly.
- `Display` — standard `8-4-4-4-12` hyphenated lowercase hex representation.
- `FromStr` — parses that same representation back into a `Uuid`.
- `Uuid::nil()` / `Uuid::is_nil()`, `Uuid::as_bytes()` / `Uuid::from_bytes()`.

Not in scope: v1/v3/v5 (namespace/time-based) UUIDs.

## License

MIT
