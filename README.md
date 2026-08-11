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
  `{"Variant": ...}` otherwise) - generic the same way structs are.
- `bool`, all integer widths, `f32`/`f64`, `char`, `String`, `Option<T>`,
  `Vec<T>`, tuples up to arity 8, `HashMap`/`BTreeMap`, `Box<T>`.
- Unknown JSON object fields are ignored during deserialization; missing
  required fields and type mismatches produce descriptive errors with a
  line/column.

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

## What's not (yet)

- Const generics (`struct Foo<const N: usize>`) and `where` clauses -
  rejected with a clear `compile_error!` rather than silently mishandled.
- Any format besides JSON. The data model (`ser`/`de` modules) is
  format-agnostic, so a second format is just a new `Serializer`/
  `Deserializer` impl away.
- Zero-copy deserialization (`&'de str` borrows) - the JSON parser always
  allocates `String`s for simplicity.
