# gap-analysis.md — rusty_serde vs. serde-rs/serde

Reference pinned at `serde-rs/serde@747814f` (shallow clone, current `main`
as of this run).

**Assessment path:** `spec` (documentation-driven). `cargo public-api`
symbol-diffing isn't meaningful here: `rusty_serde` deliberately doesn't
mirror serde's module layout or its `serde_core`/`serde_derive`/`with`-module
architecture (e.g. its own `Value` lives at `rusty_serde::value::Value`, not
`serde_json::Value`; there's no `serde_core` split). The comparable surface
is serde's *documented attribute/feature set* — extracted from
`serde_derive/src/internals/symbol.rs` (the canonical list of every
`#[serde(...)]` attribute keyword) plus core trait methods
(`is_human_readable`, `collect_*`) — matched against what `rusty_serde`
currently supports (`rusty_serde_derive/src/parse.rs`'s `Attrs` struct, and
`rusty_serde/src/{ser,de}.rs`'s trait definitions).

No hand-curated roadmap/scope doc exists in this repo (checked for
`ROADMAP.md`, `ARCHITECTURE.md`, `docs/`, an issues-as-roadmap convention -
none found); the README's "What's supported" section is the closest thing to
one, and is what "already have" below is checked against.

Internal-only serde symbols that don't correspond to a user-facing feature
(`field_identifier`, `variant_identifier`, `repr`, and the internal
`serialize`/`deserialize` keys used by `with`-module splitting) are omitted -
matching serde_derive's own macro-internal plumbing isn't a meaningful parity
target when the two derive macros don't share an implementation.

| Symbol | Category | Source | Platforms | Reference | Breaking? | Est. size | Notes |
| --- | --- | --- | --- | --- | --- | --- | --- |
| `default = "path"` | attr (field, existing bare `default`) | spec | n/a | `serde_derive` `DEFAULT` symbol / [field attrs](https://serde.rs/field-attrs.html#default--path) | no | S | Extends the existing bare `#[rusty_serde(default)]` to accept `default = "path"`, calling an arbitrary zero-arg fn/path instead of `Default::default()`. Purely additive parse-level extension. |
| `alias = "name"` | attr (field) | spec | n/a | `ALIAS` symbol / [field attrs](https://serde.rs/field-attrs.html#alias) | no | M | Accept one or more alternate wire names on deserialize (original `wire_name()` still wins for serialize). Repeatable attribute; needs the generated `__Field` ident-matching arm to accept multiple string patterns per variant. |
| `deny_unknown_fields` | attr (container) | spec | n/a | `DENY_UNKNOWN_FIELDS` symbol / [container attrs](https://serde.rs/container-attrs.html#deny_unknown_fields) | no | S | Opt-in: the generated `__ignore` catch-all arm errors instead of skipping. Default behavior (ignore) is unchanged, so existing derives aren't affected. |
| `skip_serializing` / `skip_deserializing` | attr (field) | spec | n/a | `SKIP_SERIALIZING` / `SKIP_DESERIALIZING` symbols / [field attrs](https://serde.rs/field-attrs.html#skip_serializing) | no | M | One-directional versions of the existing `skip` (which does both). New attribute keys; existing `skip` keeps its current meaning (`skip_serializing` + `skip_deserializing` together), so no behavior change for current users. |
| `rename(serialize = "..", deserialize = "..")` | attr (field/variant/container, existing `rename`) | spec | n/a | `RENAME` symbol / [field attrs](https://serde.rs/field-attrs.html#rename) | no | M | Extends existing `rename = "x"` (single name, both directions) to also accept the two-key form for direction-specific wire names. Additive parse extension; bare-string form keeps working unchanged. |
| `tag = "t", content = "c"` (adjacently tagged enums) | attr (container) | spec | n/a | `TAG` + `CONTENT` symbols / [container attrs](https://serde.rs/container-attrs.html#tag--content) | no | L | Third enum representation alongside existing external (default) and internal (`tag` alone) tagging: `{"t":"Variant","c":<data>}`. Reuses the `Value`-buffering machinery already built for `untagged`/internal tagging. Newtype/tuple variants (unrepresentable under internal tagging) *are* representable here, unlike `tag` alone. |
| `other` | attr (variant) | spec | n/a | `OTHER` symbol / [variant attrs](https://serde.rs/variant-attrs.html#other) | no | S | Marks a unit variant as the deserialize catch-all for unrecognized tag values, instead of the current hard error. Only meaningful on external/internal-tagged enums (not `untagged`, which already tries variants until one matches). |
| `with = "module"` | attr (field) | spec | n/a | `WITH` symbol / [field attrs](https://serde.rs/field-attrs.html#with) | no | M | Routes a field's serialize/deserialize through `module::serialize`/`module::deserialize` instead of the field type's own `Serialize`/`Deserialize` impl - the standard escape hatch for third-party types. Purely additive codegen path. |
| `serialize_with = "path"` / `deserialize_with = "path"` | attr (field) | spec | n/a | `SERIALIZE_WITH` / `DESERIALIZE_WITH` symbols / [field attrs](https://serde.rs/field-attrs.html#serialize_with) | no | S | One-directional version of `with` above - natural to land in the same PR/issue as `with` since they share the same codegen shape (a wrapper call instead of a trait-method call). |
| `bound = "T: Trait"` | attr (container/field) | spec | n/a | `BOUND` symbol / [container attrs](https://serde.rs/container-attrs.html#bound) | no | M | Overrides the derive's auto-generated `T: Serialize`/`T: Deserialize` bound on a generic type parameter - needed when the real bound is different (e.g. a field wrapped in a custom `with` module that doesn't require the field type itself to be (De)Serialize). Additive: only changes output for types that opt in. |
| `transparent` | attr (container) | spec | n/a | `TRANSPARENT` symbol / [container attrs](https://serde.rs/container-attrs.html#transparent) | no | S | For a single-field named or tuple struct: serialize/deserialize exactly as the inner field would, with no wrapping - like the existing newtype-struct behavior, but opt-in for a *named* single-field struct too. Purely additive (new container attribute, no change to existing codegen paths). |
| `Serializer::is_human_readable` / `Deserializer::is_human_readable` | core trait method | spec | n/a | serde's `Serializer`/`Deserializer` traits | no | S | New trait methods with a default impl (`true`) - additive to the trait (existing format impls keep compiling unchanged) and lets a derived impl special-case a compact vs. human-readable representation (e.g. `#[serde(with = ...)]` implementations commonly branch on this). |
| `Serializer::collect_seq` / `collect_map` / `collect_str` | core trait method | spec | n/a | serde's `Serializer` trait | no | S | Default-impl convenience methods for serializing from an `Iterator`/`Display` without collecting into a `Vec`/`String` first. Additive; existing `Serializer` impls inherit the default and don't need to change. |
| `from = "T"` / `try_from = "T"` (container) | attr (container) | spec | n/a | `FROM` / `TRY_FROM` symbols / [container attrs](https://serde.rs/container-attrs.html#from) | no | M | Deserialize via an intermediate type `T` then `T::into()`/`T::try_into()`. Additive - only affects containers that opt in. |
| `into = "T"` (container) | attr (container) | spec | n/a | `INTO` symbol / [container attrs](https://serde.rs/container-attrs.html#into) | no | M | Serialize side of the above: clone into `T` then serialize that. Natural to pair with `from`/`try_from` in the same issue given the shared "convert, then delegate" codegen shape. |
| `remote = "path"` (+ field `getter`) | attr (container/field) | spec | n/a | `REMOTE` / `GETTER` symbols / [remote derive](https://serde.rs/remote-derive.html) | no | L | Lets a derive be written for a type this crate doesn't own (e.g. a foreign struct), including private-field access via `getter`. Substantial codegen surface (a full second impl shape) relative to everything else on this list - lowest priority, largest scope; flagged here mainly for completeness rather than a near-term issue. |

**Deliberately out of scope for this run** (not rows above):
- `field_identifier` / `variant_identifier` / `repr` / internal `with`-module
  `serialize`/`deserialize` keys - serde_derive-internal plumbing, not a
  user-facing capability gap.
- `no_std` support, `Deserializer::deserialize_in_place` (a pure allocation
  optimization, not a capability gap), and matching serde's exact error
  message wording - all quality-of-implementation concerns rather than
  missing capabilities.
- A `serde_bytes`-equivalent efficient byte-array format - `rusty_serde`
  already has working (if not maximally efficient) `Vec<u8>`
  serialize/deserialize via the existing `bytes`/`byte_buf` methods; not a
  capability gap.

## Pass 2 (re-scan)

All 15 rows above (plus `with = "module"`, filed separately after this table
was first written) are implemented and merged. Re-checked `serde-rs/serde`
for drift: `master` is still at the exact same commit (`747814f`) pinned
above - nothing shipped upstream since Pass 1. The three rows below are
items Pass 1's read of `symbol.rs` simply missed, not new upstream surface;
found by re-cross-referencing the current `serde_derive/src/internals/symbol.rs`
symbol list and https://serde.rs/container-attrs.html against
`rusty_serde_derive`'s `Attrs` struct.

| Symbol | Category | Source | Platforms | Reference | Breaking? | Est. size | Notes |
| --- | --- | --- | --- | --- | --- | --- | --- |
| `crate = "path"` | attr (container) | spec | n/a | `CRATE` symbol / [container attrs](https://serde.rs/container-attrs.html#crate) | no | L | Overrides the `::rusty_serde::...` path prefix generated code uses, for a crate that re-exports/vendors `rusty_serde` under a different name. Conceptually a simple parameterization, but mechanically large - every one of the hundreds of `::rusty_serde::` path literals across `codegen_ser.rs`/`codegen_de.rs` needs threading through a configurable prefix instead of a hardcoded string. |
| `expecting = "..."` | attr (container) | spec | n/a | `EXPECTING` symbol / [container attrs](https://serde.rs/container-attrs.html#expecting) | no | S | Overrides the auto-generated `"struct {name}"`/`"enum {name}"` text used in the generated `Visitor::expecting()` (shows up in "invalid type" error messages) with a custom string. Purely additive - a handful of codegen sites, one per container's `Visitor` impl. |
| `rename_all_fields = "..."` | attr (container, enums only) | spec | n/a | `RENAME_ALL_FIELDS` symbol / [container attrs](https://serde.rs/container-attrs.html#rename_all_fields) | no | S | Like `rename_all`, but applies the case conversion to the *fields* of every struct variant across an enum, rather than to the variant names themselves (which is what plain `rename_all` on an enum already does). Reuses the existing `apply_rename_all_fields` case-conversion helper, just applied per-variant instead of at the top level. |

**Still deliberately out of scope** (unchanged from Pass 1, re-confirmed
against the current symbol list): `field_identifier` / `variant_identifier`
/ `repr` (serde_derive-internal plumbing - used to hand-write a `Deserialize`
impl for an identifier enum paired with a derive elsewhere; doesn't fit this
crate's private, non-exposed `ident_enum` codegen), the `borrow` *attribute*
itself (zero-copy deserialization for `&'de str`/`Cow<'de, str>` fields
already works without it - `borrow` exists upstream because serde's derive
parses field types and is conservative by default about which type
parameters may borrow, a default this crate's type-blind derive never needs
to override; see README's "What's not (yet)"), `no_std`,
`deserialize_in_place`, exact error-message-wording parity, and a
`serde_bytes`-equivalent format.
