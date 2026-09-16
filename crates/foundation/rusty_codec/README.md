# rusty_codec

A `#![no_std]` + `alloc` sovereign TOML configuration parser and binary
buffer serialization crate.

## TOML (`rusty_toml`)

A real (subset, honestly-documented) parser — not a placeholder. Verified
against this crate's own real `Cargo.toml` and a real dependency line lifted
from `rusty_tls`'s `Cargo.toml` (inline table, feature array, hyphenated
keys), not just synthetic examples.

**Implemented:** comments, bare/dotted keys, tables (`[section]`, including
nested `[a.b.c]`), basic strings (`"..."` with `\n`/`\t`/`\\`/`\"` escapes)
and literal strings (`'...'`, no escape processing), integers and floats
(with `_` digit separators and exponents), booleans, arrays (including ones
spanning multiple physical lines), and inline tables (`{ a = 1, b = 2 }`).

**Known, deliberate gaps:** no arrays-of-tables (`[[section]]`), no
multi-line basic/literal strings (`"""..."""`/`'''...'''`), no quoted/dotted
table headers with quoted segments, no native date/time values (parsed as
plain strings instead).

```rust
use rusty_codec::TomlValue;

let doc = TomlValue::parse_str(r#"
[package]
name = "example"
version = "0.1.0"
"#).unwrap();

assert_eq!(doc.get("package").unwrap().get("name").unwrap().as_str(), Some("example"));
```

## Binary buffer codec (`rusty_bincode`)

A length-prefixed byte-buffer round-trip (`serialize`/`deserialize`) — real
and correct as far as it goes, but not field-by-field struct serialization
the "bincode" name might imply; it's a generic `Vec<u8>` container format.

## Testing

```
cargo test
```
