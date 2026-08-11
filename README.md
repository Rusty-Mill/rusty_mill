# rusty_serde

A hand-rolled, dependency-free reimplementation of the ideas behind
[serde](https://serde.rs): a `Serialize`/`Deserialize` data model that's
independent of any wire format, a compact JSON format built on top of it,
and a `#[derive(Serialize, Deserialize)]` macro - all with **zero crates.io
dependencies**.

```toml
[dependencies]
rusty_serde = { path = "rusty_serde" }
```

```rust
use rusty_serde::{Deserialize, Serialize};

#[derive(Debug, PartialEq, Serialize, Deserialize)]
struct Point {
    x: i32,
    y: i32,
}

let json = rusty_serde::json::to_string(&Point { x: 1, y: 2 }).unwrap();
assert_eq!(json, r#"{"x":1,"y":2}"#);

let point: Point = rusty_serde::json::from_str(&json).unwrap();
assert_eq!(point, Point { x: 1, y: 2 });
```

## Layout

- **`rusty_serde`** - the data model (`Serialize`, `Deserialize`,
  `Serializer`, `Deserializer`, `Visitor`, ...), built-in impls for
  primitives/`String`/`Option`/`Vec`/tuples/`HashMap`/`BTreeMap`, and the
  `json` module (`to_string` / `from_str`).
- **`rusty_serde_derive`** - the `#[derive(Serialize)]` / `#[derive(Deserialize)]`
  proc-macro, written directly against the compiler-provided `proc_macro`
  crate (no `syn`, no `quote`).

Run `cargo tree` and there's nothing there but the two crates in this
workspace - the derive macro talks to `proc_macro` (part of the Rust
toolchain, not a crate you depend on), and everything else is `std`.

## What's supported

- Structs: named (`struct Foo { a: i32 }`), tuple (`struct Foo(i32, String)`),
  and unit (`struct Foo;`) - generic over any number of lifetime/type
  parameters (`struct Foo<'a, T: Clone> { ... }`).
- Enums: unit, newtype, tuple, and struct variants, serialized the way serde
  calls "externally tagged" (`"Variant"` for a unit variant,
  `{"Variant": ...}` otherwise) - generic the same way structs are. A unit
  variant marked `#[rusty_serde(other)]` is the deserialize catch-all for
  any tag that doesn't match another variant (external or internally
  tagged; not meaningful on `untagged`, which already tries every variant),
  discarding whatever data came with the unrecognized tag.
- `bool`, all integer widths, `f32`/`f64`, `char`, `String`, `Option<T>`,
  `Vec<T>`, tuples up to arity 8, `HashMap`/`BTreeMap`, `Box<T>`.
- Unknown JSON object fields are ignored during deserialization; missing
  required fields and type mismatches produce descriptive errors with a
  line/column.
- Field attributes on named struct/variant fields (`rename`/`default` on
  enum variants too):
  ```rust
  #[derive(Serialize, Deserialize)]
  struct Config {
      #[rusty_serde(rename = "name")]
      display_name: String,
      #[rusty_serde(default)]
      retries: u32,
      #[rusty_serde(skip)]
      cache: Option<String>,
  }
  ```
  `rename` uses a different wire key/variant tag than the Rust name
  (`rename(serialize = "..", deserialize = "..")` sets either direction
  independently, leaving the other at the field's own Rust name).
  `default` falls back to `Default::default()` if the field is missing on
  deserialize instead of erroring (`default = "path"` falls back to calling
  an arbitrary zero-arg `path()` instead). `skip` never serializes the field
  and always defaults it on deserialize, ignoring anything present on the
  wire under its name (`skip_serializing`/`skip_deserializing` are its two
  halves independently - one keeps the field off the wire while still
  reading it if present, the other always defaults it while still writing
  it). `alias = "..."` (repeatable) accepts extra wire names on deserialize
  alongside the field's primary name/`rename`; serialize is unaffected and
  always uses the primary name.
- Container attributes:
  ```rust
  #[derive(Serialize, Deserialize)]
  #[rusty_serde(rename_all = "camelCase")]
  struct Config {
      display_name: String, // -> "displayName" on the wire
  }

  #[derive(Serialize, Deserialize)]
  #[rusty_serde(tag = "kind")]
  enum Shape {
      Circle,                                 // -> {"kind":"Circle"}
      Rectangle { width: f64, height: f64 },   // -> {"kind":"Rectangle","width":..,"height":..}
  }
  ```
  `rename_all` case-converts every field/variant name that didn't set its
  own `rename` (`lowercase`, `UPPERCASE`, `PascalCase`, `camelCase`,
  `snake_case`, `SCREAMING_SNAKE_CASE`, `kebab-case`,
  `SCREAMING-KEBAB-CASE`). `tag` (enums only) switches from external
  tagging (`{"Variant": ...}`) to internal tagging (`{"<tag>": "Variant",
  ...fields}`), for unit and named-field variants - tuple/newtype variants
  aren't representable that way (there's no sound way to splice an
  arbitrary value's serialization into an outer object without knowing its
  shape), so `tag` on an enum with one is a `compile_error!`. `deny_unknown_fields`
  errors on deserialize instead of silently ignoring a field/key that
  doesn't match any of the type's own (applies to struct/enum-struct-variant
  fields alike); mutually exclusive with a `flatten` field, which needs
  somewhere to put keys that don't match.

  An unsupported combination (`skip`/`default` on a variant, any
  `#[rusty_serde(...)]` on a tuple field, `rename_all`/`tag` outside the
  container, `tag` on a struct, an unrecognized `rename_all` style) is
  always a clear `compile_error!` rather than a silent no-op.
- `where` clauses on generic structs/enums, forwarded into the generated
  `impl` alongside its own `Serialize`/`Deserialize` bounds.
- `Serializer`/`Deserializer::is_human_readable()`, so a hand-written
  `Serialize`/`Deserialize` impl can pick a representation per format (an
  ISO-8601 string vs. a raw integer for a timestamp, say). Defaults to
  `true`; both of this crate's own formats (`json`, `ron`) are text-based
  and inherit that default.
- `Serializer::collect_seq`/`collect_map`/`collect_str`, for serializing
  straight from an `Iterator`/`Display` value without collecting into a
  `Vec`/`HashMap`/`String` first. Default impls built on the existing
  `serialize_seq`/`serialize_map`/`serialize_str`, so every `Serializer`
  implementation (including `json`/`ron`) gets them for free.

Generics work without the derive macro's parser ever looking at field
*types* (it only needs field/variant *names*, since `Serialize`/
`Deserialize` are invoked generically and Rust's own type inference fills
in the rest): every declared type parameter just gets a blanket
`Serialize`/`Deserialize` bound tacked onto the generated `impl`, e.g.
`impl<T: Serialize> Serialize for Foo<T>`. That's always sound - any field
type built from `T` already needs that bound to compile - if occasionally
more conservative than a hand-written impl would be (an unused
`PhantomData<T>` field would still force `T: Serialize`, since the macro
can't see that `T` goes unused there).

Internally-tagged enums are the one place a JSON value has to be buffered
into an in-memory tree before it can be deserialized: the tag key can
appear anywhere in the object, so there's no way to know which variant
you're reading until you've already read every entry. That buffering (and
the second `Deserializer` implementation that runs the ordinary
`Deserialize` machinery back against the buffered tree) lives entirely in
the JSON format module, behind a `deserialize_internally_tagged_enum`
method on the core `Deserializer` trait that other formats can just leave
at its default ("not supported") if they don't need it.

## Testing

Besides hand-picked cases in `tests/roundtrip.rs`, `tests/fuzz_roundtrip.rs`
round-trips thousands of arbitrary values (and specifically strings, to
stress `\uXXXX`/surrogate-pair escaping, and numbers, to stress formatting)
through a tiny hand-rolled xorshift PRNG - no `proptest`/`quickcheck`,
consistent with the rest of the project.

## What's not (yet)

- Const generics (`struct Foo<const N: usize>`) - rejected with a clear
  `compile_error!` rather than silently mishandled.
- Any format besides JSON. The data model (`ser`/`de` modules) is
  format-agnostic, so a second format is just a new `Serializer`/
  `Deserializer` impl away.
- Zero-copy deserialization (`&'de str` borrows) - the JSON parser always
  allocates `String`s for simplicity.
