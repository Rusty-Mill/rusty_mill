# Architecture

## Overview

A hand-rolled, dependency-free reimplementation of the ideas behind
[serde](https://serde.rs): a `Serialize`/`Deserialize` data model that's
independent of any wire format, a `#[derive(Serialize, Deserialize)]`
proc-macro built directly against `proc_macro` (no `syn`/`quote`), and two
formats (JSON, a RON-like format) built on top. Not a serde clone at the
implementation level - the derive macro deliberately never parses field
*types*, only field/variant *names*, which is why several serde attributes
(`with`, `remote`) needed a different mechanism here (see the main
[README](./README.md)) than serde's own.

## Boundaries

Domain logic (the data model) is fully independent of I/O - a format only
ever talks to the model through the four core traits, never the reverse.

| Port | Adapter(s) | Notes |
| ---- | ---------- | ----- |
| `Serialize` / `Serializer` | `json::Serializer`, `ron::Serializer` | a `Serialize` impl never assumes which format is driving it - `Serializer::is_human_readable()` is the one hook for format-specific behavior |
| `Deserialize` / `Deserializer` | `json::Deserializer`, `ron::Deserializer`, `value::ValueDeserializer` | `ValueDeserializer` re-drives `Deserialize` against an already-buffered `Value` tree - what `flatten`, untagged/adjacently-tagged enums, and `deserialize_with` all use to avoid needing a second parser |
| `ErasedSerializer` / `ErasedDeserializer` (object-safe) | `rusty_serde::erased`'s adapters, backed by `rusty_serde_erased::Out` | lets `serialize_with`/`deserialize_with` call a function generically without the derive macro ever naming the field's type; the one unsafe primitive this needs is isolated in its own crate so `rusty_serde`/`rusty_serde_derive` stay 100% safe Rust |

## Structure

Three crates, one workspace: `rusty_serde` (the data model + formats),
`rusty_serde_derive` (the proc-macro, a separate crate because
`proc-macro = true` crates can't export anything else), and
`rusty_serde_erased` (the isolated unsafe primitive above). `cargo tree`
shows nothing else - no crates.io dependencies at all, by design (see
README). A fourth format would be a new module implementing the existing
`Serializer`/`Deserializer` traits, not a change to the model itself.

## Data flow

Serialize: `T::serialize(&self, serializer)` walks `T`'s shape, calling
`Serializer` methods (`serialize_str`, `serialize_struct`, ...) that the
concrete format turns into bytes/text as it goes - no intermediate tree
for the common case.

Deserialize: a `Deserializer` inspects the input and calls the matching
`Visitor` method, which builds `T` directly (push-based, not pull-based).
The one exception is anywhere the wire shape can't be known until the
whole value's been read (an internally-tagged enum's tag can appear
anywhere in the object) - those buffer into `Value` first via
`ValueDeserializer`.

## Key decisions
See [docs/adr/](./docs/adr/) for the record of individual decisions and their tradeoffs.

## Non-goals

- **Parsing field types.** The derive macro only ever sees field/variant
  *names* - this is what keeps it small enough to hand-write against
  `proc_macro` directly, but it's also why serde's `with`/`serialize_with`/
  `deserialize_with` needed a purpose-built object-safe erasure layer
  instead of serde's own generic-wrapper trick, and why `remote`+`getter`
  is scoped to structs only.
- **`no_std`.** Would mean swapping every `std::collections::HashMap`/
  `String`/`Vec` for `core`/`alloc` equivalents across the whole crate - a
  different scale of change than an additive attribute.
- **`field_identifier`/`variant_identifier`.** Serde-internal plumbing for
  hand-written `Deserialize` impls that want to reuse serde's own
  identifier-matching codegen. This crate already has the equivalent
  internally (the private `ident_enum` codegen helper) but doesn't expose
  it as a standalone feature - there's no "hand-written impl wants to
  borrow the derive's internals" use case here the way there is upstream.
- **Matching serde's exact error message wording.** A quality-of-
  implementation concern, not a capability gap.
