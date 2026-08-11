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
- **`rusty_serde_erased`** - a single unsafe primitive (`Out`, for handing
  a typed output slot across an object-safe/`dyn`-compatible boundary),
  isolated in its own crate so `rusty_serde` and `rusty_serde_derive` can
  stay entirely safe Rust. Not part of the public API surface either of
  those crates promise - an internal building block.

Run `cargo tree` and there's nothing there but the three crates in this
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
  always uses the primary name. `serialize_with = "path"`/
  `deserialize_with = "path"` (named struct fields only) route the field
  through `path` instead of its own `Serialize`/`Deserialize` impl - each
  `path` matches the ordinary
  `fn<S: Serializer>(value: &T, serializer: S) -> Result<S::Ok, S::Error>`/
  `fn<'de, D: Deserializer<'de>>(deserializer: D) -> Result<T, D::Error>`
  convention, and works even for a field type with no `Serialize`/
  `Deserialize` impl of its own (or one you don't want used here), e.g.
  reformatting a `std::time::Duration` as a plain integer:
  ```rust
  mod as_seconds {
      use rusty_serde::{Deserialize, Deserializer, Serializer};
      use std::time::Duration;

      pub fn serialize<S: Serializer>(value: &Duration, serializer: S) -> Result<S::Ok, S::Error> {
          serializer.serialize_u64(value.as_secs())
      }

      pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Duration, D::Error> {
          let secs = u64::deserialize(deserializer)?;
          Ok(Duration::from_secs(secs))
      }
  }

  #[derive(Serialize, Deserialize)]
  struct Event {
      #[rusty_serde(
          serialize_with = "as_seconds::serialize",
          deserialize_with = "as_seconds::deserialize"
      )]
      elapsed: std::time::Duration,
  }
  ```
  Only scalars, `Option`, unit, and unit/newtype enum variants are
  supported inside either `path` - one that tries a sequence/tuple/map/
  struct shape gets a runtime error, not a `compile_error!`, since the
  derive macro has no way to check `path`'s body ahead of time. A
  `deserialize_with` path always runs against a value already buffered
  into this crate's format-agnostic `Value` (the same machinery `flatten`/
  untagged enums use) rather than live input - which means, per `Value`'s
  own number handling, a small non-negative JSON integer arrives as
  `Value::Int` (`visit_i64`), not `Value::UInt`/`visit_u64`; delegating to
  an existing `Deserialize` impl (`u64::deserialize`, above) rather than
  writing a bespoke `Visitor` sidesteps needing to know that.
  `with = "module"` is shorthand for setting both at once -
  `serialize_with = "module::serialize"` and
  `deserialize_with = "module::deserialize"` together (so the example
  above could instead write `#[rusty_serde(with = "as_seconds")]`) -
  mutually exclusive with setting either individually on the same field.
  Neither `with` nor `serialize_with`/`deserialize_with` on their own
  combines with `getter`, and `serialize_with` doesn't combine with
  `skip_serializing_if`; `deserialize_with` doesn't combine with
  `flatten`. See "Generics work without..." below for why this needs an
  object-safe erasure layer (`rusty_serde::erased`) instead of the
  straightforward generic wrapper every other attribute in this list gets
  away with.
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
  shape), so `tag` on an enum with one is a `compile_error!`. Pairing `tag`
  with `content` switches to adjacent tagging instead
  (`#[rusty_serde(tag = "kind", content = "data")]` ->
  `{"kind":"Variant","data":...fields}`, or just `{"kind":"Variant"}` for a
  unit variant) - since the payload gets its own key instead of sharing the
  tag's object, every variant shape is representable, including tuple
  variants; `content` without `tag` is a `compile_error!`. `deny_unknown_fields`
  errors on deserialize instead of silently ignoring a field/key that
  doesn't match any of the type's own (applies to struct/enum-struct-variant
  fields alike); mutually exclusive with a `flatten` field, which needs
  somewhere to put keys that don't match. `transparent` (structs only, and
  only with exactly one named field) serializes/deserializes exactly as
  that one field would on its own, no wrapping - like a tuple-struct-of-one's
  existing behavior, opted into for a named struct. `expecting = "..."`
  overrides the auto-generated `"struct Foo"`/`"enum Foo"`-style text the
  generated `Visitor::expecting()` writes (surfaces in "invalid type"
  deserialize errors) with a custom message - rejected on `untagged`/
  adjacently tagged (`tag` + `content`) enums, since neither drives a
  `Visitor` with an `expecting()` to override. Note that with this crate's
  own JSON/RON formats, a *struct's* `expecting()` text is only actually
  reached on a type mismatch once a `Visitor` is already involved - e.g.
  deserializing an already-buffered `Value` (what `flatten`/untagged
  enums/`deserialize_with` all do internally) - since both formats' own
  top-level `deserialize_struct`/`deserialize_enum` fail with their own
  generic message (`"expected ..."`) on a wrong top-level *shape* before
  ever consulting the `Visitor`; an *enum's* `expecting()` text currently
  has no reachable path at all, since every deserializer this crate ships
  (`json`, `ron`, and the `Value` buffer itself) hard-checks an enum's
  shape the same way. Still implemented for both (matching serde's
  attribute one-for-one, and correct for any future deserializer that
  dispatches more generically), just worth knowing before relying on it.

  An unsupported combination (`skip`/`default` on a variant, any
  `#[rusty_serde(...)]` on a tuple field, `rename_all`/`tag` outside the
  container, `tag` on a struct, an unrecognized `rename_all` style) is
  always a clear `compile_error!` rather than a silent no-op.
- `where` clauses on generic structs/enums, forwarded into the generated
  `impl` alongside its own `Serialize`/`Deserialize` bounds.
- `#[rusty_serde(bound = "T: Trait")]` (container-only) replaces the
  derive's auto-generated `T: Serialize`/`T: Deserialize` where-clause
  entirely (both directions), for when the inferred bound is wrong - e.g.
  `bound = ""` for a generic struct whose only use of `T` is a
  `PhantomData<T>` field, which doesn't actually need `T` to be
  (de)serializable even though the macro (unable to see field types)
  would otherwise assume it does.
- `#[rusty_serde(from = "T")]`/`#[rusty_serde(try_from = "T")]`
  (container-only, deserialize side only - `#[derive(Deserialize)]` is
  still required for the impl to exist at all): deserializes an
  intermediate type `T` and converts via `From<T>`/`TryFrom<T>` instead of
  reading the container's own fields directly. A `try_from` conversion
  error is reported through the target format's error type via
  `Error::custom`, so it needs to implement `Display`.
- `#[rusty_serde(into = "T")]` (container-only, serialize side only): the
  `Serialize` counterpart to `from`/`try_from` - clones into `T` (via
  `Into<T>`, so the container itself needs `Clone`) then serializes that,
  instead of serializing the container's own fields directly.
- `#[rusty_serde(remote = "path::Type")]` (container-only, structs only)
  targets the generated impls at `path::Type` instead of the annotated
  struct itself, using the annotated struct's own field list as
  `path::Type`'s shape - for writing a `Serialize`/`Deserialize` impl for a
  type whose definition you can't add `#[derive(...)]` to directly. A
  field's `#[rusty_serde(getter = "path::to::fn")]` calls `path::to::fn(self)`
  (expected to return an owned value) instead of `&self.field` during
  serialization, for a field that isn't visible from wherever the impl
  ends up (`getter` is meaningless, and rejected, without `remote` also
  set). Deserializing still builds `path::Type { field1, field2, .. }`
  directly, so its fields need to already be nameable from that point -
  either public, or (as with a private-field type you don't own) by giving
  the annotated struct the same module as `path::Type` itself, same as
  real serde's own remote-derive examples. `path::Type` also needs to
  already be local enough for Rust's orphan rule to allow the impl in the
  first place - a foreign crate's type typically isn't, unless it's used
  alongside a `with`-equivalent indirection this crate doesn't support (see
  "What's not (yet)" below); `remote` on an enum is a `compile_error!`
  rather than an untested, partially-working shape.
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

`serialize_with`/`deserialize_with` are the one place a field's generated
code needs to call a function generically (over any `S: Serializer`/
`D: Deserializer<'de>`) without the derive macro having parsed - or being
able to name - that field's own type. A normal generic wrapper can't do
this (its own `Serialize`/`Deserialize` impl would need to satisfy every
possible field type at once, which doesn't type-check for a function
specific to one type); instead, `rusty_serde::erased` gives the with-
function a *concrete* stand-in type (`ErasedAsSerializer`/
`ErasedAsDeserializer`) that implements the real trait, satisfying the
with-function's own genericity through ordinary monomorphization. That
stand-in forwards each call through a small object-safe (`dyn`-compatible)
trait - and getting a real value back out through that boundary needs one
small, deliberately isolated piece of `unsafe` code (`rusty_serde_erased`'s
`Out`), so `rusty_serde` and `rusty_serde_derive` themselves stay 100%
safe Rust. Which side of the call needs `Out` flips between the two
directions: `Serializer::Ok` is opaque per-format, so a *real* serializer
wrapped as erased needs it to hand its result back to the original
caller; `Deserializer` has no such opaque type of its own (every method's
result comes from whichever `Visitor` the caller supplies), so it's the
caller's *own* `Visitor`, once wrapped as erased, that needs it instead -
`deserialize_with`'s `T` (the with-function's own, already-concrete return
type) needs no such hand-off at all, unlike `serialize_with`'s `S::Ok`,
unknown until called.

`deserialize_with` specifically also routes through this crate's
`Value`/`ValueDeserializer` buffering (the same machinery `flatten`/
untagged enums use) before ever reaching the with-function, rather than
handing it live input directly - `Deserialize::deserialize` has no `&self`
(a value doesn't exist to read a per-instance function pointer from yet,
unlike `Serialize::serialize`), so there's no way to carry a specific
with-function through a single reusable generic wrapper type the way
`serialize_with`'s `With<T>` does; buffering into the already-nameable
`Value` type first sidesteps ever needing a per-field type name on this
side too.

Internally-tagged enums are the one place a JSON value has to be buffered
into an in-memory tree before it can be deserialized: the tag key can
appear anywhere in the object, so there's no way to know which variant
you're reading until you've already read every entry. That buffering (and
the second `Deserializer` implementation that runs the ordinary
`Deserialize` machinery back against the buffered tree) lives entirely in
the JSON format module, behind a `deserialize_internally_tagged_enum`
method on the core `Deserializer` trait that other formats can just leave
at its default ("not supported") if they don't need it. Adjacently- and
untagged enums buffer the same way, but through the format-agnostic
`Value`/`ValueDeserializer` machinery instead - no core trait method needed,
since a tag (if any) and a self-contained sub-value are all that's needed
to pick and drive a variant, and any format's `Deserialize::deserialize`
call already produces a `Value` for free.

## Testing

Besides hand-picked cases in `tests/roundtrip.rs`, `tests/fuzz_roundtrip.rs`
round-trips thousands of arbitrary values (and specifically strings, to
stress `\uXXXX`/surrogate-pair escaping, and numbers, to stress formatting)
through a tiny hand-rolled xorshift PRNG - no `proptest`/`quickcheck`,
consistent with the rest of the project.

## What's not (yet)

- Const generics (`struct Foo<const N: usize>`) - rejected with a clear
  `compile_error!` rather than silently mishandled.
- Zero-copy deserialization (`&'de str` borrows) - the JSON parser always
  allocates `String`s for simplicity.
