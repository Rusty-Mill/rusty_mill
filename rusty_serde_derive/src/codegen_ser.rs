use crate::parse::{Data, Fields, Generics, NamedField, Variant};

/// The runtime length expression for a `serialize_struct`/`serialize_map`
/// call: `1` per always-present field, or `if <path>(&value) { 0 } else {
/// 1 }` for a `skip_serializing_if` field, summed. `value_of` maps a
/// field's Rust identifier to the expression that names its value at the
/// call site - `&self.field` for a top-level struct, or the field's own
/// name for an enum arm's `ref field` binding (already a reference either
/// way, so the same expression works both as the serialize-call argument
/// and as the `skip_serializing_if` predicate's argument).
fn count_expr(active: &[&NamedField], value_of: impl Fn(&str) -> String) -> String {
    let terms: Vec<String> = active
        .iter()
        .map(|f| match &f.attrs.skip_serializing_if {
            Some(path) => {
                let value = value_of(&f.name);
                format!("if {path}({value}) {{ 0usize }} else {{ 1usize }}")
            }
            None => "1usize".to_string(),
        })
        .collect();
    if terms.is_empty() {
        "0usize".to_string()
    } else {
        terms.join(" + ")
    }
}

/// Emits one `call_fn(&mut __state, "<wire key>", <value>)?;` per active
/// field, wrapped in `if !<path>(<value>) { ... }` for a
/// `skip_serializing_if` field.
fn field_serialize_calls(
    active: &[&NamedField],
    call_fn: &str,
    value_of: impl Fn(&str) -> String,
) -> String {
    let mut out = String::new();
    for f in active {
        let wire = f.wire_name();
        let value = value_of(&f.name);
        let call = format!("{call_fn}(&mut __state, {wire:?}, {value})?;\n");
        out += &match &f.attrs.skip_serializing_if {
            Some(path) => format!("if !{path}({value}) {{\n    {call}}}\n"),
            None => call,
        };
    }
    out
}

pub fn generate(data: &Data) -> String {
    match data {
        Data::Struct {
            name,
            generics,
            fields,
            deny_unknown_fields: _,
            transparent,
            // `from`/`try_from` only replace the Deserialize impl.
            from: _,
            into,
            remote,
        } => match into {
            Some(into) => into_impl(name, generics, into),
            None => struct_impl(name, generics, fields, *transparent, remote.as_deref()),
        },
        Data::Enum {
            name,
            generics,
            variants,
            tag,
            content,
            untagged,
            deny_unknown_fields: _,
            from: _,
            into,
        } => match into {
            Some(into) => into_impl(name, generics, into),
            None => enum_impl(
                name,
                generics,
                variants,
                tag.as_deref(),
                content.as_deref(),
                *untagged,
            ),
        },
    }
}

/// `#[rusty_serde(into = "T")]`: the entire `Serialize` impl is just
/// "clone into `T`, then serialize that" - none of the container's own
/// fields/variants matter to *this* impl (they still matter to `T`'s own
/// `Serialize`, wherever that comes from). The `Into<T>` conversion takes
/// `self` by value, hence cloning first (`serialize` only gets `&self`).
/// Applies identically to a struct or an enum container.
fn into_impl(name: &str, generics: &Generics, into: &str) -> String {
    let ty = generics.ty(name);
    let impl_decl = generics.impl_decl(None);
    let where_clause = generics.where_clause("::rusty_serde::Serialize");
    format!(
        "impl{impl_decl} ::rusty_serde::Serialize for {ty}{where_clause} {{\n\
             fn serialize<__S>(&self, serializer: __S) -> Result<__S::Ok, __S::Error>\n\
             where\n\
                 __S: ::rusty_serde::Serializer,\n\
             {{\n\
                 let __intermediate: {into} = ::std::clone::Clone::clone(self).into();\n\
                 ::rusty_serde::Serialize::serialize(&__intermediate, serializer)\n\
             }}\n\
         }}\n"
    )
}

fn struct_impl(
    name: &str,
    generics: &Generics,
    fields: &Fields,
    transparent: bool,
    remote: Option<&str>,
) -> String {
    let body = match fields {
        Fields::Unit => {
            format!("::rusty_serde::Serializer::serialize_unit_struct(serializer, {name:?})")
        }
        Fields::Unnamed(0) => {
            format!("::rusty_serde::Serializer::serialize_unit_struct(serializer, {name:?})")
        }
        Fields::Unnamed(1) => format!(
            "::rusty_serde::Serializer::serialize_newtype_struct(serializer, {name:?}, &self.0)"
        ),
        Fields::Unnamed(n) => {
            let mut out = format!(
                "let mut __state = ::rusty_serde::Serializer::serialize_tuple_struct(serializer, {name:?}, {n})?;\n"
            );
            for i in 0..*n {
                out += &format!(
                    "::rusty_serde::ser::SerializeTupleStruct::serialize_field(&mut __state, &self.{i})?;\n"
                );
            }
            out += "::rusty_serde::ser::SerializeTupleStruct::end(__state)";
            out
        }
        // Parse-time validation guarantees `transparent` only reaches here
        // with exactly one field - serialize exactly as that field would
        // on its own, the same way a tuple-struct-of-one already does.
        Fields::Named(fields) if transparent => format!(
            "::rusty_serde::Serializer::serialize_newtype_struct(serializer, {name:?}, &self.{field})",
            field = fields[0].name,
        ),
        Fields::Named(fields) => named_struct_serialize_body(name, fields),
    };

    let impl_decl = generics.impl_decl(None);
    let where_clause = generics.where_clause("::rusty_serde::Serialize");
    // `remote` targets the impl at a different (foreign, or just
    // undecorated) type instead of this one - `self` is still built from
    // the annotated struct's own field list, just read as `path::Type`
    // instead of `name` (field access is structural either way, so nothing
    // else here needs to change).
    let ty = generics.ty(remote.unwrap_or(name));
    format!(
        "impl{impl_decl} ::rusty_serde::Serialize for {ty}{where_clause} {{\n\
             fn serialize<__S>(&self, serializer: __S) -> Result<__S::Ok, __S::Error>\n\
             where\n\
                 __S: ::rusty_serde::Serializer,\n\
             {{\n\
                 {body}\n\
             }}\n\
         }}\n"
    )
}

fn named_struct_serialize_body(name: &str, fields: &[NamedField]) -> String {
    let active: Vec<&NamedField> = fields
        .iter()
        .filter(|f| !f.attrs.skips_serializing())
        .collect();
    // `getter = "path"` (only meaningful alongside a container-level
    // `remote`) reads a field that isn't visible as `self.field` from this
    // impl's own module - `path(self)` (expected to return an owned value)
    // stands in for the ordinary field access instead.
    let value_of = |field_name: &str| {
        let getter = active
            .iter()
            .find(|f| f.name == field_name)
            .and_then(|f| f.attrs.getter.as_deref());
        match getter {
            Some(getter) => format!("&({getter}(self))"),
            None => format!("&self.{field_name}"),
        }
    };

    // A flattened field's fields merge into the parent object, so the
    // parent can no longer use serialize_struct (a fixed key set) - it
    // falls back to serialize_map (an open one) for the whole struct, the
    // same way real serde does. Parse-time validation already guarantees
    // at most one flatten field.
    match active.iter().find(|f| f.attrs.flatten) {
        None => {
            let count = count_expr(&active, value_of);
            let calls = field_serialize_calls(
                &active,
                "::rusty_serde::ser::SerializeStruct::serialize_field",
                value_of,
            );
            format!(
                "let mut __state = ::rusty_serde::Serializer::serialize_struct(serializer, {name:?}, {count})?;\n\
                 {calls}\
                 ::rusty_serde::ser::SerializeStruct::end(__state)"
            )
        }
        Some(flat) => {
            let normal: Vec<&NamedField> = active
                .iter()
                .filter(|f| !f.attrs.flatten)
                .copied()
                .collect();
            let calls = field_serialize_calls(
                &normal,
                "::rusty_serde::ser::SerializeMap::serialize_entry",
                value_of,
            );
            let flat_ident = &flat.name;
            format!(
                "let mut __state = ::rusty_serde::Serializer::serialize_map(serializer, None)?;\n\
                 {calls}\
                 ::rusty_serde::Serialize::serialize(&self.{flat_ident}, ::rusty_serde::flatten::FlattenSerializer::new(&mut __state))?;\n\
                 ::rusty_serde::ser::SerializeMap::end(__state)"
            )
        }
    }
}

fn enum_impl(
    name: &str,
    generics: &Generics,
    variants: &[Variant],
    tag: Option<&str>,
    content: Option<&str>,
    untagged: bool,
) -> String {
    let mut arms = String::new();
    for (index, variant) in variants.iter().enumerate() {
        arms += &match (tag, content, untagged) {
            (Some(t), Some(c), _) => variant_arm_adjacent(name, variant, t, c),
            (Some(t), None, _) => variant_arm_tagged(name, variant, t),
            (None, _, true) => variant_arm_untagged(name, variant),
            (None, _, false) => variant_arm(name, index as u32, variant),
        };
    }

    let impl_decl = generics.impl_decl(None);
    let where_clause = generics.where_clause("::rusty_serde::Serialize");
    let ty = generics.ty(name);
    format!(
        "impl{impl_decl} ::rusty_serde::Serialize for {ty}{where_clause} {{\n\
             fn serialize<__S>(&self, serializer: __S) -> Result<__S::Ok, __S::Error>\n\
             where\n\
                 __S: ::rusty_serde::Serializer,\n\
             {{\n\
                 match *self {{\n{arms}}}\n\
             }}\n\
         }}\n"
    )
}

/// Serializes a variant of an internally-tagged enum (`#[rusty_serde(tag =
/// "...")]`) as one flat JSON object: `{"<tag>": "<variant>", ...fields}`.
/// The parser already rejected tuple variants for this case (there's no
/// sound way to splice an arbitrary inner value's serialization into an
/// outer object without knowing its shape), so only unit and named-field
/// variants reach here.
/// Serializes a variant of an untagged enum (`#[rusty_serde(untagged)]`) as
/// its payload alone - no tag, no wrapper object. A unit variant becomes
/// JSON `null`, a newtype variant serializes its inner value directly, a
/// tuple variant becomes an array, and a named-field variant becomes a
/// plain object of just its own fields. Since nothing on the wire
/// distinguishes one variant from another, `Deserialize` recovers the
/// variant by trying each one in turn (see `codegen_de`).
fn variant_arm_untagged(enum_name: &str, variant: &Variant) -> String {
    let vname = &variant.name;
    match &variant.fields {
        Fields::Unit => {
            format!("    {enum_name}::{vname} => ::rusty_serde::Serializer::serialize_unit(serializer),\n")
        }
        Fields::Unnamed(0) => {
            format!("    {enum_name}::{vname}() => ::rusty_serde::Serializer::serialize_unit(serializer),\n")
        }
        Fields::Unnamed(1) => format!(
            "    {enum_name}::{vname}(ref __f0) => ::rusty_serde::Serialize::serialize(__f0, serializer),\n"
        ),
        Fields::Unnamed(n) => {
            let binders = (0..*n).map(|i| format!("ref __f{i}")).collect::<Vec<_>>().join(", ");
            let mut body = format!(
                "let mut __state = ::rusty_serde::Serializer::serialize_tuple(serializer, {n})?;\n"
            );
            for i in 0..*n {
                body += &format!(
                    "        ::rusty_serde::ser::SerializeTuple::serialize_element(&mut __state, __f{i})?;\n"
                );
            }
            body += "        ::rusty_serde::ser::SerializeTuple::end(__state)";
            format!("    {enum_name}::{vname}({binders}) => {{\n        {body}\n    }}\n")
        }
        Fields::Named(fields) => {
            let binders = binder_list(fields);
            let active: Vec<&NamedField> = fields.iter().filter(|f| !f.attrs.skips_serializing()).collect();
            let value_of = |field_name: &str| field_name.to_string();
            let count = count_expr(&active, value_of);
            let calls = field_serialize_calls(
                &active,
                "::rusty_serde::ser::SerializeStruct::serialize_field",
                value_of,
            );
            let body = format!(
                "let mut __state = ::rusty_serde::Serializer::serialize_struct(serializer, {vname:?}, {count})?;\n\
                 {calls}\
                 ::rusty_serde::ser::SerializeStruct::end(__state)"
            );
            format!("    {enum_name}::{vname} {{ {binders} }} => {{\n        {body}\n    }}\n")
        }
    }
}

fn variant_arm_tagged(enum_name: &str, variant: &Variant, tag: &str) -> String {
    let vname = &variant.name;
    let wire_vname = variant.wire_name();
    match &variant.fields {
        Fields::Unit | Fields::Unnamed(0) => {
            let pattern = match &variant.fields {
                Fields::Unit => format!("{enum_name}::{vname}"),
                _ => format!("{enum_name}::{vname}()"),
            };
            format!(
                "    {pattern} => {{\n\
                     let mut __state = ::rusty_serde::Serializer::serialize_map(serializer, Some(1))?;\n\
                     ::rusty_serde::ser::SerializeMap::serialize_entry(&mut __state, {tag:?}, {wire_vname:?})?;\n\
                     ::rusty_serde::ser::SerializeMap::end(__state)\n\
                 }}\n"
            )
        }
        Fields::Unnamed(_) => {
            unreachable!("tuple variants are rejected at parse time when `tag` is set")
        }
        Fields::Named(fields) => {
            let binders = binder_list(fields);
            let active: Vec<&NamedField> = fields
                .iter()
                .filter(|f| !f.attrs.skips_serializing())
                .collect();
            let value_of = |field_name: &str| field_name.to_string();
            let count = count_expr(&active, value_of);
            let calls = field_serialize_calls(
                &active,
                "::rusty_serde::ser::SerializeMap::serialize_entry",
                value_of,
            );
            let body = format!(
                "let mut __state = ::rusty_serde::Serializer::serialize_map(serializer, Some(1usize + {count}))?;\n\
                 ::rusty_serde::ser::SerializeMap::serialize_entry(&mut __state, {tag:?}, {wire_vname:?})?;\n\
                 {calls}\
                 ::rusty_serde::ser::SerializeMap::end(__state)"
            );
            format!("    {enum_name}::{vname} {{ {binders} }} => {{\n        {body}\n    }}\n")
        }
    }
}

/// Serializes a variant of an adjacently-tagged enum (`#[rusty_serde(tag =
/// "...", content = "...")]`) as `{"<tag>": "<variant>", "<content>":
/// <data>}` - a unit variant omits the content key entirely, everything
/// else nests its own shape (bare value for a newtype, array for a tuple,
/// object for named fields) under it. Unlike internal tagging (`tag`
/// alone), every variant shape is representable here, since the payload
/// gets its own key instead of needing to be spliced into the tag's own
/// object.
fn variant_arm_adjacent(enum_name: &str, variant: &Variant, tag: &str, content: &str) -> String {
    let vname = &variant.name;
    let wire_vname = variant.wire_name();
    match &variant.fields {
        Fields::Unit | Fields::Unnamed(0) => {
            let pattern = match &variant.fields {
                Fields::Unit => format!("{enum_name}::{vname}"),
                _ => format!("{enum_name}::{vname}()"),
            };
            format!(
                "    {pattern} => {{\n\
                     let mut __state = ::rusty_serde::Serializer::serialize_map(serializer, Some(1))?;\n\
                     ::rusty_serde::ser::SerializeMap::serialize_entry(&mut __state, {tag:?}, {wire_vname:?})?;\n\
                     ::rusty_serde::ser::SerializeMap::end(__state)\n\
                 }}\n"
            )
        }
        Fields::Unnamed(1) => format!(
            "    {enum_name}::{vname}(ref __f0) => {{\n\
                     let mut __state = ::rusty_serde::Serializer::serialize_map(serializer, Some(2))?;\n\
                     ::rusty_serde::ser::SerializeMap::serialize_entry(&mut __state, {tag:?}, {wire_vname:?})?;\n\
                     ::rusty_serde::ser::SerializeMap::serialize_entry(&mut __state, {content:?}, __f0)?;\n\
                     ::rusty_serde::ser::SerializeMap::end(__state)\n\
                 }}\n"
        ),
        Fields::Unnamed(n) => {
            let binders = (0..*n)
                .map(|i| format!("ref __f{i}"))
                .collect::<Vec<_>>()
                .join(", ");
            let tuple_lit = format!(
                "({})",
                (0..*n).map(|i| format!("__f{i}")).collect::<Vec<_>>().join(", ")
            );
            format!(
                "    {enum_name}::{vname}({binders}) => {{\n\
                     let mut __state = ::rusty_serde::Serializer::serialize_map(serializer, Some(2))?;\n\
                     ::rusty_serde::ser::SerializeMap::serialize_entry(&mut __state, {tag:?}, {wire_vname:?})?;\n\
                     ::rusty_serde::ser::SerializeMap::serialize_entry(&mut __state, {content:?}, &{tuple_lit})?;\n\
                     ::rusty_serde::ser::SerializeMap::end(__state)\n\
                 }}\n"
            )
        }
        Fields::Named(fields) => {
            let binders = binder_list(fields);
            let active: Vec<&NamedField> = fields
                .iter()
                .filter(|f| !f.attrs.skips_serializing())
                .collect();
            // A small local adapter that serializes just this variant's
            // (already-borrowed, via the match arm's `ref` bindings) active
            // fields as a struct - generic over each field's own type, with
            // no bound beyond `Serialize` (inferred from the actual field
            // types at construction), so it works without the derive macro
            // ever having to name those types itself.
            let type_params: Vec<String> =
                (0..active.len()).map(|i| format!("__T{i}")).collect();
            let type_params_decl = type_params.join(", ");
            let type_params_bounded = type_params
                .iter()
                .map(|t| format!("{t}: ::rusty_serde::Serialize"))
                .collect::<Vec<_>>()
                .join(", ");
            let content_fields_decl = active
                .iter()
                .zip(&type_params)
                .map(|(f, t)| format!("{}: &'a {t}", f.name))
                .collect::<Vec<_>>()
                .join(", ");
            let content_value_of = |field_name: &str| format!("self.{field_name}");
            let content_count = count_expr(&active, content_value_of);
            let content_calls = field_serialize_calls(
                &active,
                "::rusty_serde::ser::SerializeStruct::serialize_field",
                content_value_of,
            );
            let content_construct = active
                .iter()
                .map(|f| f.name.clone())
                .collect::<Vec<_>>()
                .join(", ");
            let def = format!(
                "struct __Content<'a, {type_params_decl}> {{ {content_fields_decl} }}\n\
                 impl<'a, {type_params_bounded}> ::rusty_serde::Serialize for __Content<'a, {type_params_decl}> {{\n\
                     fn serialize<__CS>(&self, serializer: __CS) -> Result<__CS::Ok, __CS::Error>\n\
                     where\n\
                         __CS: ::rusty_serde::Serializer,\n\
                     {{\n\
                         let mut __state = ::rusty_serde::Serializer::serialize_struct(serializer, {vname:?}, {content_count})?;\n\
                         {content_calls}\
                         ::rusty_serde::ser::SerializeStruct::end(__state)\n\
                     }}\n\
                 }}\n"
            );
            let body = format!(
                "{def}\
                 let mut __state = ::rusty_serde::Serializer::serialize_map(serializer, Some(2))?;\n\
                 ::rusty_serde::ser::SerializeMap::serialize_entry(&mut __state, {tag:?}, {wire_vname:?})?;\n\
                 ::rusty_serde::ser::SerializeMap::serialize_entry(&mut __state, {content:?}, &__Content {{ {content_construct} }})?;\n\
                 ::rusty_serde::ser::SerializeMap::end(__state)"
            );
            format!("    {enum_name}::{vname} {{ {binders} }} => {{\n        {body}\n    }}\n")
        }
    }
}

/// The `{ ref a, ref b, c: _ }` pattern for a named-field variant/struct
/// match arm: skipped fields are bound to `_` (matched but discarded, so
/// the pattern stays exhaustive without pulling in a value that's never
/// used), everything else is bound by reference.
fn binder_list(fields: &[NamedField]) -> String {
    fields
        .iter()
        .map(|f| {
            if f.attrs.skips_serializing() {
                format!("{}: _", f.name)
            } else {
                format!("ref {}", f.name)
            }
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn variant_arm(enum_name: &str, index: u32, variant: &Variant) -> String {
    let vname = &variant.name;
    let wire_vname = variant.wire_name();
    match &variant.fields {
        Fields::Unit => format!(
            "    {enum_name}::{vname} => ::rusty_serde::Serializer::serialize_unit_variant(serializer, {enum_name:?}, {index}, {wire_vname:?}),\n"
        ),
        Fields::Unnamed(0) => format!(
            "    {enum_name}::{vname}() => ::rusty_serde::Serializer::serialize_unit_variant(serializer, {enum_name:?}, {index}, {wire_vname:?}),\n"
        ),
        Fields::Unnamed(1) => format!(
            "    {enum_name}::{vname}(ref __f0) => ::rusty_serde::Serializer::serialize_newtype_variant(serializer, {enum_name:?}, {index}, {wire_vname:?}, __f0),\n"
        ),
        Fields::Unnamed(n) => {
            let binders = (0..*n).map(|i| format!("ref __f{i}")).collect::<Vec<_>>().join(", ");
            let mut body = format!(
                "let mut __state = ::rusty_serde::Serializer::serialize_tuple_variant(serializer, {enum_name:?}, {index}, {wire_vname:?}, {n})?;\n"
            );
            for i in 0..*n {
                body += &format!(
                    "        ::rusty_serde::ser::SerializeTupleVariant::serialize_field(&mut __state, __f{i})?;\n"
                );
            }
            body += "        ::rusty_serde::ser::SerializeTupleVariant::end(__state)";
            format!("    {enum_name}::{vname}({binders}) => {{\n        {body}\n    }}\n")
        }
        Fields::Named(fields) => {
            let binders = binder_list(fields);
            let active: Vec<&NamedField> = fields.iter().filter(|f| !f.attrs.skips_serializing()).collect();
            let value_of = |field_name: &str| field_name.to_string();
            let count = count_expr(&active, value_of);
            let calls = field_serialize_calls(
                &active,
                "::rusty_serde::ser::SerializeStructVariant::serialize_field",
                value_of,
            );
            let body = format!(
                "let mut __state = ::rusty_serde::Serializer::serialize_struct_variant(serializer, {enum_name:?}, {index}, {wire_vname:?}, {count})?;\n\
                 {calls}\
                 ::rusty_serde::ser::SerializeStructVariant::end(__state)"
            );
            format!("    {enum_name}::{vname} {{ {binders} }} => {{\n        {body}\n    }}\n")
        }
    }
}
