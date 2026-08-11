use crate::parse::{Data, DefaultAttr, Fields, FromAttr, Generics, NamedField, Variant};

pub fn generate(data: &Data) -> String {
    match data {
        Data::Struct {
            name,
            generics,
            fields,
            deny_unknown_fields,
            transparent,
            from,
            // `into` only replaces the Serialize impl.
            into: _,
            remote,
        } => match from {
            Some(from) => from_impl(name, generics, from),
            None => struct_impl(
                name,
                generics,
                fields,
                *deny_unknown_fields,
                *transparent,
                remote.as_deref(),
            ),
        },
        Data::Enum {
            name,
            generics,
            variants,
            tag,
            content,
            untagged,
            deny_unknown_fields,
            from,
            into: _,
        } => match from {
            Some(from) => from_impl(name, generics, from),
            None => enum_impl(
                name,
                generics,
                variants,
                tag.as_deref(),
                content.as_deref(),
                *untagged,
                *deny_unknown_fields,
            ),
        },
    }
}

/// `#[rusty_serde(from = "T")]`/`#[rusty_serde(try_from = "T")]`: the
/// entire `Deserialize` impl is just "deserialize a `T`, then convert" -
/// none of the container's own fields/variants matter to *this* impl at
/// all (they still matter to `T`'s own `Deserialize`, wherever that comes
/// from). Applies identically to a struct or an enum container.
fn from_impl(name: &str, generics: &Generics, from: &FromAttr) -> String {
    let ty = generics.ty(name);
    let impl_decl = generics.impl_decl(Some("'de"));
    let outer_where = generics.where_clause("::rusty_serde::Deserialize<'de>");
    let body = match from {
        FromAttr::From(intermediate) => format!(
            "let __intermediate: {intermediate} = ::rusty_serde::Deserialize::deserialize(deserializer)?;\n\
             Ok(::std::convert::From::from(__intermediate))"
        ),
        FromAttr::TryFrom(intermediate) => format!(
            "let __intermediate: {intermediate} = ::rusty_serde::Deserialize::deserialize(deserializer)?;\n\
             <{ty} as ::std::convert::TryFrom<{intermediate}>>::try_from(__intermediate)\n\
                 .map_err(::rusty_serde::Error::custom)"
        ),
    };
    format!(
        "impl{impl_decl} ::rusty_serde::Deserialize<'de> for {ty}{outer_where} {{\n\
             fn deserialize<__D>(deserializer: __D) -> Result<Self, __D::Error>\n\
             where\n\
                 __D: ::rusty_serde::Deserializer<'de>,\n\
             {{\n\
                 {body}\n\
             }}\n\
         }}\n"
    )
}

/// `enum {ty} { name0, name1, ..., [__ignore] }` plus a `Deserialize` impl
/// that maps a JSON-style string identifier to one of those variants. Used
/// both for a struct's field names and an enum's variant names. This
/// generated enum only ever holds identifier tags, so - unlike the "real"
/// visitor types below - it never needs the outer type's own generics.
///
/// What an `ident_enum`'s generated identifier `Visitor` does with a wire
/// value that doesn't match any known name.
enum IdentFallback<'a> {
    /// Struct/variant fields: an error naming the field is a programmer
    /// bug, not a data error - unrecognized fields are handled by the
    /// caller (ignored, or - with `deny_unknown_fields` - an error raised
    /// closer to the actual map-walking code, with more context).
    Error,
    /// A struct's own fields: carries the raw key text along, not just an
    /// "unknown" tag - a flatten field needs it to rebuild the leftover
    /// entries; callers that don't need it just match `__ignore(_)`.
    IgnoreUnknown,
    /// An enum's variant tags, when one variant carries
    /// `#[rusty_serde(other)]`: route anything unrecognized to that
    /// variant instead of erroring.
    MapTo(&'a str),
}

/// `entries` pairs each Rust identifier (the enum variant name, and the
/// field/variant name used everywhere else in the generated code) with the
/// wire name to match against (its own name, unless renamed).
fn ident_enum(
    ty: &str,
    entries: &[(String, String)],
    expecting: &str,
    fallback_kind: IdentFallback,
) -> String {
    // A field's aliases add extra rows to `entries` that share its ident
    // (multiple wire names -> one variant), so the declaration list has to
    // be deduplicated even though the match arms below use every row.
    let mut decls: Vec<String> = Vec::new();
    for (ident, _) in entries {
        if !decls.contains(ident) {
            decls.push(ident.clone());
        }
    }
    if matches!(fallback_kind, IdentFallback::IgnoreUnknown) {
        decls.push("__ignore(::std::string::String)".to_string());
    }
    let decls = decls.join(", ");

    let mut arms = String::new();
    for (ident, wire) in entries {
        arms += &format!("                            {wire:?} => Ok({ty}::{ident}),\n");
    }
    let fallback = match fallback_kind {
        IdentFallback::IgnoreUnknown => {
            format!("                            _ => Ok({ty}::__ignore(value.to_string())),\n")
        }
        IdentFallback::MapTo(other_ident) => {
            format!("                            _ => Ok({ty}::{other_ident}),\n")
        }
        IdentFallback::Error => "                            _ => Err(::rusty_serde::Error::custom(::std::format!(\"unknown variant `{}`\", value))),\n".to_string(),
    };

    format!(
        "#[allow(non_camel_case_types)]\n\
         enum {ty} {{ {decls} }}\n\
         impl<'de> ::rusty_serde::Deserialize<'de> for {ty} {{\n\
             fn deserialize<__D>(deserializer: __D) -> Result<Self, __D::Error>\n\
             where\n\
                 __D: ::rusty_serde::Deserializer<'de>,\n\
             {{\n\
                 struct __IdentVisitor;\n\
                 impl<'de> ::rusty_serde::de::Visitor<'de> for __IdentVisitor {{\n\
                     type Value = {ty};\n\
                     fn expecting(&self, f: &mut ::std::fmt::Formatter) -> ::std::fmt::Result {{\n\
                         f.write_str({expecting:?})\n\
                     }}\n\
                     fn visit_str<__E>(self, value: &str) -> Result<{ty}, __E>\n\
                     where\n\
                         __E: ::rusty_serde::Error,\n\
                     {{\n\
                         match value {{\n{arms}{fallback}                        }}\n\
                     }}\n\
                 }}\n\
                 deserializer.deserialize_identifier(__IdentVisitor)\n\
             }}\n\
         }}\n"
    )
}

/// Struct-definition, impl-header, and construction snippets for a local
/// visitor type. When `generics` is empty this is just a plain unit struct
/// (`struct __Visitor;`); when the outer type is generic over `T`, the
/// visitor needs to redeclare `T` itself and carry a `PhantomData<T>` -
/// Rust doesn't let an item nested inside a generic function's body use
/// that function's type parameters without declaring its own copies, which
/// is why `type Value = Foo<T>` wouldn't otherwise type-check here.
struct Visitor {
    /// The item definition to splice in before the `impl`.
    def: String,
    /// The `impl<...>` header for `impl <impl_decl> Visitor<'de> for <ty>`.
    impl_decl: String,
    /// The trailing `where ...` clause (empty string if there's nothing to
    /// say), spliced in right after `<ty>`.
    where_clause: String,
    /// The visitor's own type, e.g. `__Visitor` or `__Visitor<T>`.
    ty: String,
    /// The expression that builds one, e.g. `__Visitor` or
    /// `__Visitor { __marker: ::std::marker::PhantomData }`.
    construct: String,
}

fn visitor(struct_name: &str, generics: &Generics) -> Visitor {
    if generics.lifetimes.is_empty() && generics.type_params.is_empty() {
        return Visitor {
            def: format!("struct {struct_name};"),
            impl_decl: "<'de>".to_string(),
            where_clause: String::new(),
            ty: struct_name.to_string(),
            construct: struct_name.to_string(),
        };
    }
    let use_site = generics.use_site();
    // `PhantomData` needs to "use" every declared lifetime/type parameter
    // (or Rust rejects them as unused), but it must not name the outer
    // type itself (`PhantomData<Foo<T>>`) - that would re-trigger any
    // bounds `Foo`'s own definition declared on `T`, which this freshly
    // declared `struct __Visitor<T>` doesn't (and shouldn't) redeclare.
    // A tuple of the bare parameters sidesteps that entirely.
    let mut phantom_parts: Vec<String> = generics
        .lifetimes
        .iter()
        .map(|l| {
            let name = l.split(':').next().unwrap().trim();
            format!("&{name} ()")
        })
        .collect();
    phantom_parts.extend(generics.type_params.iter().map(|t| t.name.clone()));
    let trailing_comma = if phantom_parts.len() == 1 { "," } else { "" };
    let phantom_target = format!("({}{trailing_comma})", phantom_parts.join(", "));
    Visitor {
        def: format!(
            "struct {struct_name}{use_site} {{ __marker: ::std::marker::PhantomData<{phantom_target}> }}"
        ),
        impl_decl: generics.impl_decl(Some("'de")),
        where_clause: generics.where_clause("::rusty_serde::Deserialize<'de>"),
        ty: format!("{struct_name}{use_site}"),
        construct: format!("{struct_name} {{ __marker: ::std::marker::PhantomData }}"),
    }
}

/// Body of a `visit_map` that fills in `fields` from a `MapAccess`,
/// erroring on duplicates/missing fields (or defaulting per `#[rusty_serde]`
/// attributes), then builds `constructor { .. }`. `constructor` is a bare
/// (non-generic) path like `Foo` or `Foo::Variant`: Rust infers any generic
/// arguments from the enclosing function's return type, so it never needs
/// to be spelled out here.
///
/// `skip`-ped fields never appear on the wire at all (not read, not
/// matched against `field_enum`) and are unconditionally defaulted;
/// `default`-ed fields are still read if present, but fall back to
/// `Default::default()` instead of erroring when absent.
/// `map_error_ty` is the enclosing `visit_map`'s `MapAccess::Error`
/// associated type name (e.g. `__A::Error`), needed to name the type of
/// the buffer a flattened field's leftover entries get collected into.
fn visit_map_body(
    field_enum: &str,
    fields: &[NamedField],
    constructor: &str,
    map_error_ty: &str,
    deny_unknown_fields: bool,
) -> String {
    let active: Vec<&NamedField> = fields
        .iter()
        .filter(|f| !f.attrs.skips_deserializing())
        .collect();
    let flatten_field = active.iter().find(|f| f.attrs.flatten).copied();
    let normal: Vec<&NamedField> = active
        .iter()
        .filter(|f| !f.attrs.flatten)
        .copied()
        .collect();

    let mut out = String::new();
    for f in &normal {
        let ident = &f.name;
        out += &format!("let mut __{ident}: Option<_> = None;\n");
    }
    if flatten_field.is_some() {
        out += "let mut __flatten_entries: ::std::vec::Vec<(::std::string::String, ::rusty_serde::Value)> = ::std::vec::Vec::new();\n";
    }
    out += &format!(
        "while let Some(__key) = ::rusty_serde::de::MapAccess::next_key::<{field_enum}>(&mut map)? {{\n\
             match __key {{\n"
    );
    for f in &normal {
        let ident = &f.name;
        // `deserialize_with = "path"` routes the field through `path`
        // instead of its own `Deserialize` impl. `path` needs a concrete
        // `D: Deserializer<'de>` to be handed a monomorphized type it can
        // type-check against (see `rusty_serde::erased`'s module docs) -
        // buffering through `Value`/`ValueDeserializer` first (the same
        // machinery `flatten`/untagged enums already use) sidesteps ever
        // needing to name the field's own type here, the same way a plain
        // `next_value()` call already does via inference.
        let read = match &f.attrs.deserialize_with {
            Some(with_fn) => format!(
                "{{\n\
                     let __raw: ::rusty_serde::Value = ::rusty_serde::de::MapAccess::next_value(&mut map)?;\n\
                     ::rusty_serde::erased::call_with_deserialize(\n\
                         ::rusty_serde::value::ValueDeserializer::<{map_error_ty}>::new(__raw),\n\
                         |__d| {with_fn}(__d),\n\
                     )?\n\
                 }}"
            ),
            None => "::rusty_serde::de::MapAccess::next_value(&mut map)?".to_string(),
        };
        out += &format!(
            "                {field_enum}::{ident} => {{\n\
                     if __{ident}.is_some() {{\n\
                         return Err(::rusty_serde::Error::custom({dup:?}));\n\
                     }}\n\
                     __{ident} = Some({read});\n\
                 }}\n",
            dup = format!("duplicate field `{}`", f.de_wire_name())
        );
    }
    out += &if flatten_field.is_some() {
        format!(
            "                {field_enum}::__ignore(__raw_key) => {{\n\
                     let __raw_value = ::rusty_serde::de::MapAccess::next_value::<::rusty_serde::Value>(&mut map)?;\n\
                     __flatten_entries.push((__raw_key, __raw_value));\n\
                 }}\n\
             }}\n\
         }}\n"
        )
    } else if deny_unknown_fields {
        format!(
            "                {field_enum}::__ignore(__raw_key) => {{\n\
                     return Err(::rusty_serde::Error::custom(::std::format!(\"unknown field `{{}}`\", __raw_key)));\n\
                 }}\n\
             }}\n\
         }}\n"
        )
    } else {
        format!(
            "                {field_enum}::__ignore(_) => {{\n\
                     let _ = ::rusty_serde::de::MapAccess::next_value::<::rusty_serde::de::IgnoredAny>(&mut map)?;\n\
                 }}\n\
             }}\n\
         }}\n"
        )
    };
    for f in fields {
        let ident = &f.name;
        if f.attrs.flatten {
            continue;
        }
        if f.attrs.skips_deserializing() {
            let fallback = match &f.attrs.default {
                Some(DefaultAttr::Path(path)) => format!("{path}()"),
                Some(DefaultAttr::Trait) | None => "::std::default::Default::default()".to_string(),
            };
            out += &format!("let {ident} = {fallback};\n");
        } else if let Some(default) = &f.attrs.default {
            let fallback = match default {
                DefaultAttr::Trait => "::std::default::Default::default()".to_string(),
                DefaultAttr::Path(path) => format!("{path}()"),
            };
            out += &format!("let {ident} = __{ident}.unwrap_or_else(|| {fallback});\n");
        } else {
            out += &format!(
                "let {ident} = __{ident}.ok_or_else(|| ::rusty_serde::Error::custom({missing:?}))?;\n",
                missing = format!("missing field `{}`", f.de_wire_name())
            );
        }
    }
    if let Some(flat) = flatten_field {
        let ident = &flat.name;
        out += &format!(
            "let {ident} = ::rusty_serde::Deserialize::deserialize(\
                 ::rusty_serde::value::ValueDeserializer::<{map_error_ty}>::new(\
                     ::rusty_serde::Value::Map(__flatten_entries)\
                 )\
             )?;\n"
        );
    }
    let field_list = fields
        .iter()
        .map(|f| f.name.clone())
        .collect::<Vec<_>>()
        .join(", ");
    out += &format!("Ok({constructor} {{ {field_list} }})\n");
    out
}

fn struct_impl(
    name: &str,
    generics: &Generics,
    fields: &Fields,
    deny_unknown_fields: bool,
    transparent: bool,
    remote: Option<&str>,
) -> String {
    // `remote` targets the impl (and every constructor expression below,
    // which all build a `Self`) at a different type than the one this
    // derive was written on - see `codegen_ser`'s `struct_impl` for the
    // serialize-side half of the same idea. Cosmetic text (`"struct
    // {name}"`, the wire type-name hint passed to `deserialize_struct` and
    // friends) keeps using the original `name` either way.
    let target = remote.unwrap_or(name);
    let ty = generics.ty(target);
    let body = match fields {
        Fields::Unit => {
            let v = visitor("__Visitor", generics);
            format!(
                "{def}\n\
                 impl{impl_decl} ::rusty_serde::de::Visitor<'de> for {vty}{where_clause} {{\n\
                     type Value = {ty};\n\
                     fn expecting(&self, f: &mut ::std::fmt::Formatter) -> ::std::fmt::Result {{\n\
                         f.write_str(\"unit struct {name}\")\n\
                     }}\n\
                     fn visit_unit<__E>(self) -> Result<{ty}, __E>\n\
                     where __E: ::rusty_serde::Error {{\n\
                         Ok({target})\n\
                     }}\n\
                 }}\n\
                 ::rusty_serde::Deserializer::deserialize_unit_struct(deserializer, {name:?}, {construct})",
                def = v.def,
                impl_decl = v.impl_decl,
                where_clause = v.where_clause,
                vty = v.ty,
                construct = v.construct,
            )
        }
        Fields::Unnamed(0) => {
            let v = visitor("__Visitor", generics);
            format!(
                "{def}\n\
                 impl{impl_decl} ::rusty_serde::de::Visitor<'de> for {vty}{where_clause} {{\n\
                     type Value = {ty};\n\
                     fn expecting(&self, f: &mut ::std::fmt::Formatter) -> ::std::fmt::Result {{\n\
                         f.write_str(\"unit struct {name}\")\n\
                     }}\n\
                     fn visit_unit<__E>(self) -> Result<{ty}, __E>\n\
                     where __E: ::rusty_serde::Error {{\n\
                         Ok({target}())\n\
                     }}\n\
                 }}\n\
                 ::rusty_serde::Deserializer::deserialize_unit_struct(deserializer, {name:?}, {construct})",
                def = v.def,
                impl_decl = v.impl_decl,
                where_clause = v.where_clause,
                vty = v.ty,
                construct = v.construct,
            )
        }
        Fields::Unnamed(1) => {
            let v = visitor("__Visitor", generics);
            format!(
                "{def}\n\
                 impl{impl_decl} ::rusty_serde::de::Visitor<'de> for {vty}{where_clause} {{\n\
                     type Value = {ty};\n\
                     fn expecting(&self, f: &mut ::std::fmt::Formatter) -> ::std::fmt::Result {{\n\
                         f.write_str(\"tuple struct {name}\")\n\
                     }}\n\
                     fn visit_newtype_struct<__D>(self, deserializer: __D) -> Result<{ty}, __D::Error>\n\
                     where __D: ::rusty_serde::Deserializer<'de> {{\n\
                         ::rusty_serde::Deserialize::deserialize(deserializer).map({target})\n\
                     }}\n\
                     fn visit_seq<__A>(self, mut seq: __A) -> Result<{ty}, __A::Error>\n\
                     where __A: ::rusty_serde::de::SeqAccess<'de> {{\n\
                         let __v0 = ::rusty_serde::de::SeqAccess::next_element(&mut seq)?\n\
                             .ok_or_else(|| ::rusty_serde::Error::custom(\"missing tuple element 0\"))?;\n\
                         Ok({target}(__v0))\n\
                     }}\n\
                 }}\n\
                 ::rusty_serde::Deserializer::deserialize_newtype_struct(deserializer, {name:?}, {construct})",
                def = v.def,
                impl_decl = v.impl_decl,
                where_clause = v.where_clause,
                vty = v.ty,
                construct = v.construct,
            )
        }
        Fields::Unnamed(n) => {
            let mut elems = String::new();
            let mut binders = String::new();
            for i in 0..*n {
                elems += &format!(
                    "let __v{i} = ::rusty_serde::de::SeqAccess::next_element(&mut seq)?\n\
                         .ok_or_else(|| ::rusty_serde::Error::custom({msg:?}))?;\n",
                    msg = format!("missing tuple element {i}")
                );
                binders += &format!("__v{i}, ");
            }
            let v = visitor("__Visitor", generics);
            format!(
                "{def}\n\
                 impl{impl_decl} ::rusty_serde::de::Visitor<'de> for {vty}{where_clause} {{\n\
                     type Value = {ty};\n\
                     fn expecting(&self, f: &mut ::std::fmt::Formatter) -> ::std::fmt::Result {{\n\
                         f.write_str(\"tuple struct {name}\")\n\
                     }}\n\
                     fn visit_seq<__A>(self, mut seq: __A) -> Result<{ty}, __A::Error>\n\
                     where __A: ::rusty_serde::de::SeqAccess<'de> {{\n\
                         {elems}\n\
                         Ok({target}({binders}))\n\
                     }}\n\
                 }}\n\
                 ::rusty_serde::Deserializer::deserialize_tuple_struct(deserializer, {name:?}, {n}, {construct})",
                def = v.def,
                impl_decl = v.impl_decl,
                where_clause = v.where_clause,
                vty = v.ty,
                construct = v.construct,
            )
        }
        // Parse-time validation guarantees `transparent` only reaches here
        // with exactly one field - delegate straight to that field's own
        // `Deserialize` impl, the same way a tuple-struct-of-one already
        // does (just building `{target} { field: v }` instead of `{target}(v)`).
        Fields::Named(fields) if transparent => {
            let field = &fields[0].name;
            let v = visitor("__Visitor", generics);
            format!(
                "{def}\n\
                 impl{impl_decl} ::rusty_serde::de::Visitor<'de> for {vty}{where_clause} {{\n\
                     type Value = {ty};\n\
                     fn expecting(&self, f: &mut ::std::fmt::Formatter) -> ::std::fmt::Result {{\n\
                         f.write_str(\"struct {name}\")\n\
                     }}\n\
                     fn visit_newtype_struct<__D>(self, deserializer: __D) -> Result<{ty}, __D::Error>\n\
                     where __D: ::rusty_serde::Deserializer<'de> {{\n\
                         ::rusty_serde::Deserialize::deserialize(deserializer).map(|__v0| {target} {{ {field}: __v0 }})\n\
                     }}\n\
                 }}\n\
                 ::rusty_serde::Deserializer::deserialize_newtype_struct(deserializer, {name:?}, {construct})",
                def = v.def,
                impl_decl = v.impl_decl,
                where_clause = v.where_clause,
                vty = v.ty,
                construct = v.construct,
            )
        }
        Fields::Named(fields) => {
            let active: Vec<&NamedField> = fields
                .iter()
                .filter(|f| !f.attrs.skips_deserializing())
                .collect();
            let entries: Vec<(String, String)> = active
                .iter()
                .filter(|f| !f.attrs.flatten)
                .flat_map(|f| {
                    std::iter::once((f.name.clone(), f.de_wire_name().to_string())).chain(
                        f.attrs
                            .aliases
                            .iter()
                            .map(|alias| (f.name.clone(), alias.clone())),
                    )
                })
                .collect();
            let fields_array = active
                .iter()
                .filter(|f| !f.attrs.flatten)
                .map(|f| format!("{:?}", f.de_wire_name()))
                .collect::<Vec<_>>()
                .join(", ");
            let map_body =
                visit_map_body("__Field", fields, target, "__A::Error", deny_unknown_fields);
            let v = visitor("__Visitor", generics);
            format!(
                "{ident_enum}\n\
                 {def}\n\
                 impl{impl_decl} ::rusty_serde::de::Visitor<'de> for {vty}{where_clause} {{\n\
                     type Value = {ty};\n\
                     fn expecting(&self, f: &mut ::std::fmt::Formatter) -> ::std::fmt::Result {{\n\
                         f.write_str(\"struct {name}\")\n\
                     }}\n\
                     fn visit_map<__A>(self, mut map: __A) -> Result<{ty}, __A::Error>\n\
                     where __A: ::rusty_serde::de::MapAccess<'de> {{\n\
                         {map_body}\n\
                     }}\n\
                 }}\n\
                 const __FIELDS: &[&str] = &[{fields_array}];\n\
                 ::rusty_serde::Deserializer::deserialize_struct(deserializer, {name:?}, __FIELDS, {construct})",
                ident_enum = ident_enum("__Field", &entries, "field identifier", IdentFallback::IgnoreUnknown),
                def = v.def,
                impl_decl = v.impl_decl,
                where_clause = v.where_clause,
                vty = v.ty,
                construct = v.construct,
            )
        }
    };

    let impl_decl = generics.impl_decl(Some("'de"));
    let outer_where = generics.where_clause("::rusty_serde::Deserialize<'de>");
    format!(
        "impl{impl_decl} ::rusty_serde::Deserialize<'de> for {ty}{outer_where} {{\n\
             fn deserialize<__D>(deserializer: __D) -> Result<Self, __D::Error>\n\
             where\n\
                 __D: ::rusty_serde::Deserializer<'de>,\n\
             {{\n\
                 {body}\n\
             }}\n\
         }}\n"
    )
}

fn enum_impl(
    name: &str,
    generics: &Generics,
    variants: &[Variant],
    tag: Option<&str>,
    content: Option<&str>,
    untagged: bool,
    deny_unknown_fields: bool,
) -> String {
    if untagged {
        return enum_impl_untagged(name, generics, variants, deny_unknown_fields);
    }
    if let (Some(t), Some(c)) = (tag, content) {
        return enum_impl_adjacent(name, generics, variants, t, c, deny_unknown_fields);
    }
    let ty = generics.ty(name);
    let variant_entries: Vec<(String, String)> = variants
        .iter()
        .map(|v| (v.name.clone(), v.de_wire_name().to_string()))
        .collect();
    let variants_array = variant_entries
        .iter()
        .map(|(_, wire)| format!("{wire:?}"))
        .collect::<Vec<_>>()
        .join(", ");
    let other_variant = variants.iter().find(|v| v.attrs.other).map(|v| &v.name);

    let mut arms = String::new();
    for variant in variants {
        arms += &variant_arm(
            name,
            &ty,
            generics,
            &variant.name,
            &variant.fields,
            deny_unknown_fields,
        );
    }

    let v = visitor("__Visitor", generics);

    // Both external and internal tagging drive the exact same
    // EnumAccess/VariantAccess-based visitor - the only difference is which
    // Deserializer method hands it the input. Internal tagging's added
    // complexity (buffering a JSON object to find the tag key regardless of
    // its position, then re-deserializing the rest) all lives in the JSON
    // format implementation, not here.
    let deserialize_call = match tag {
        Some(t) => format!(
            "::rusty_serde::Deserializer::deserialize_internally_tagged_enum(deserializer, {name:?}, {t:?}, __VARIANTS, {construct})",
            construct = v.construct,
        ),
        None => format!(
            "::rusty_serde::Deserializer::deserialize_enum(deserializer, {name:?}, __VARIANTS, {construct})",
            construct = v.construct,
        ),
    };

    let impl_decl = generics.impl_decl(Some("'de"));
    let outer_where = generics.where_clause("::rusty_serde::Deserialize<'de>");
    format!(
        "impl{impl_decl} ::rusty_serde::Deserialize<'de> for {ty}{outer_where} {{\n\
             fn deserialize<__D>(deserializer: __D) -> Result<Self, __D::Error>\n\
             where\n\
                 __D: ::rusty_serde::Deserializer<'de>,\n\
             {{\n\
                 {ident_enum}\n\
                 {def}\n\
                 impl{v_impl_decl} ::rusty_serde::de::Visitor<'de> for {vty}{v_where_clause} {{\n\
                     type Value = {ty};\n\
                     fn expecting(&self, f: &mut ::std::fmt::Formatter) -> ::std::fmt::Result {{\n\
                         f.write_str(\"enum {name}\")\n\
                     }}\n\
                     fn visit_enum<__A>(self, data: __A) -> Result<{ty}, __A::Error>\n\
                     where __A: ::rusty_serde::de::EnumAccess<'de> {{\n\
                         match ::rusty_serde::de::EnumAccess::variant(data)? {{\n\
                             {arms}\n\
                         }}\n\
                     }}\n\
                 }}\n\
                 const __VARIANTS: &[&str] = &[{variants_array}];\n\
                 {deserialize_call}\n\
             }}\n\
         }}\n",
        ident_enum = ident_enum(
            "__Field",
            &variant_entries,
            "variant identifier",
            match other_variant {
                Some(ident) => IdentFallback::MapTo(ident),
                None => IdentFallback::Error,
            },
        ),
        def = v.def,
        v_impl_decl = v.impl_decl,
        v_where_clause = v.where_clause,
        vty = v.ty,
    )
}

/// An adjacently-tagged enum (`#[rusty_serde(tag = "t", content = "c")]`)
/// buffers the whole input into a `Value` (same as `untagged` has to), reads
/// the tag key to know which variant to expect ahead of time, then reuses
/// `untagged_variant_body`'s per-shape codegen against the `content` key's
/// value (or `Value::Null`, for a unit variant, if `content` is absent -
/// same as external tagging already treats a bare `"Variant"` as having no
/// data). Unlike `enum_impl_untagged`, the tag tells us exactly which
/// variant to try, so this dispatches with a single `match` instead of
/// trying every variant in declaration order.
fn enum_impl_adjacent(
    name: &str,
    generics: &Generics,
    variants: &[Variant],
    tag: &str,
    content: &str,
    deny_unknown_fields: bool,
) -> String {
    let ty = generics.ty(name);
    let other_variant = variants.iter().find(|v| v.attrs.other).map(|v| &v.name);

    let mut arms = String::new();
    for variant in variants {
        let body = untagged_variant_body(name, &ty, generics, variant, deny_unknown_fields);
        let wire = variant.de_wire_name();
        arms += &format!(
            "                    {wire:?} => (|| -> Result<Self, __D::Error> {{\n{body}\n}})(),\n"
        );
    }
    let fallback = match other_variant {
        Some(ident) => format!("                    _ => Ok({name}::{ident}),\n"),
        None => "                    __other => Err(::rusty_serde::Error::custom(::std::format!(\"unknown variant `{}`\", __other))),\n".to_string(),
    };

    let missing_tag_msg = format!("missing field `{tag}`");
    let impl_decl = generics.impl_decl(Some("'de"));
    let outer_where = generics.where_clause("::rusty_serde::Deserialize<'de>");
    format!(
        "impl{impl_decl} ::rusty_serde::Deserialize<'de> for {ty}{outer_where} {{\n\
             fn deserialize<__D>(deserializer: __D) -> Result<Self, __D::Error>\n\
             where\n\
                 __D: ::rusty_serde::Deserializer<'de>,\n\
             {{\n\
                 let __outer: ::rusty_serde::Value = ::rusty_serde::Deserialize::deserialize(deserializer)?;\n\
                 let __tag = __outer.get({tag:?})\n\
                     .ok_or_else(|| ::rusty_serde::Error::custom({missing_tag_msg:?}))?\n\
                     .as_str()\n\
                     .ok_or_else(|| ::rusty_serde::Error::custom(\"tag must be a string\"))?;\n\
                 let __value: ::rusty_serde::Value = __outer.get({content:?}).cloned().unwrap_or(::rusty_serde::Value::Null);\n\
                 match __tag {{\n\
                     {arms}\
                     {fallback}\
                 }}\n\
             }}\n\
         }}\n"
    )
}

/// An untagged enum (`#[rusty_serde(untagged)]`) has nothing on the wire
/// that names the variant, so there's no way to know which one to expect
/// ahead of time - the only option is to buffer the whole input into a
/// `Value` once, then try each variant's own shape against a clone of it
/// in declaration order, keeping the first one that deserializes cleanly.
/// `EnumAccess`/`VariantAccess` (built for the tagged case, where the
/// variant is already known) don't fit this at all, so this generates a
/// completely different `deserialize` body from `enum_impl`'s.
fn enum_impl_untagged(
    name: &str,
    generics: &Generics,
    variants: &[Variant],
    deny_unknown_fields: bool,
) -> String {
    let ty = generics.ty(name);
    let mut attempts = String::new();
    for variant in variants {
        let body = untagged_variant_body(name, &ty, generics, variant, deny_unknown_fields);
        attempts += &format!(
            "if let Ok(__v) = (|| -> Result<Self, __D::Error> {{\n{body}\n}})() {{\n\
                 return Ok(__v);\n\
             }}\n"
        );
    }

    let impl_decl = generics.impl_decl(Some("'de"));
    let outer_where = generics.where_clause("::rusty_serde::Deserialize<'de>");
    let missing_variant_msg = format!("data did not match any variant of {name}");
    format!(
        "impl{impl_decl} ::rusty_serde::Deserialize<'de> for {ty}{outer_where} {{\n\
             fn deserialize<__D>(deserializer: __D) -> Result<Self, __D::Error>\n\
             where\n\
                 __D: ::rusty_serde::Deserializer<'de>,\n\
             {{\n\
                 let __value: ::rusty_serde::Value = ::rusty_serde::Deserialize::deserialize(deserializer)?;\n\
                 {attempts}\
                 Err(::rusty_serde::Error::custom({missing_variant_msg:?}))\n\
             }}\n\
         }}\n"
    )
}

/// One variant's attempt at consuming the buffered `__value`, as the body
/// of a `Result<Self, __D::Error>`-returning closure (so failed attempts
/// can use `?` freely and just fall through via the closure's `Err`).
fn untagged_variant_body(
    enum_name: &str,
    enum_ty: &str,
    generics: &Generics,
    variant: &Variant,
    deny_unknown_fields: bool,
) -> String {
    let vname = &variant.name;
    let constructor = format!("{enum_name}::{vname}");
    match &variant.fields {
        Fields::Unit | Fields::Unnamed(0) => {
            let ctor_call = match &variant.fields {
                Fields::Unit => constructor,
                _ => format!("{constructor}()"),
            };
            format!(
                "match __value.clone() {{\n\
                     ::rusty_serde::Value::Null => Ok({ctor_call}),\n\
                     _ => Err(::rusty_serde::Error::custom(\"expected null\")),\n\
                 }}"
            )
        }
        Fields::Unnamed(1) => format!(
            "::rusty_serde::Deserialize::deserialize(\
                 ::rusty_serde::value::ValueDeserializer::<__D::Error>::new(__value.clone())\
             ).map({constructor})"
        ),
        Fields::Unnamed(n) => {
            let mut elems = String::new();
            let mut binders = String::new();
            for i in 0..*n {
                elems += &format!(
                    "let __v{i} = ::rusty_serde::de::SeqAccess::next_element(&mut seq)?\n\
                             .ok_or_else(|| ::rusty_serde::Error::custom({msg:?}))?;\n",
                    msg = format!("missing tuple element {i}")
                );
                binders += &format!("__v{i}, ");
            }
            let v = visitor("__TupleVisitor", generics);
            format!(
                "{def}\n\
                 impl{impl_decl} ::rusty_serde::de::Visitor<'de> for {vty}{where_clause} {{\n\
                     type Value = {enum_ty};\n\
                     fn expecting(&self, f: &mut ::std::fmt::Formatter) -> ::std::fmt::Result {{\n\
                         f.write_str(\"tuple variant {enum_name}::{vname}\")\n\
                     }}\n\
                     fn visit_seq<__A>(self, mut seq: __A) -> Result<{enum_ty}, __A::Error>\n\
                     where __A: ::rusty_serde::de::SeqAccess<'de> {{\n\
                         {elems}\n\
                         Ok({constructor}({binders}))\n\
                     }}\n\
                 }}\n\
                 ::rusty_serde::Deserializer::deserialize_tuple(\
                     ::rusty_serde::value::ValueDeserializer::<__D::Error>::new(__value.clone()), {n}, {construct})",
                def = v.def,
                impl_decl = v.impl_decl,
                vty = v.ty,
                where_clause = v.where_clause,
                construct = v.construct,
            )
        }
        Fields::Named(fields) => {
            let active: Vec<&NamedField> = fields
                .iter()
                .filter(|f| !f.attrs.skips_deserializing())
                .collect();
            let entries: Vec<(String, String)> = active
                .iter()
                .filter(|f| !f.attrs.flatten)
                .flat_map(|f| {
                    std::iter::once((f.name.clone(), f.de_wire_name().to_string())).chain(
                        f.attrs
                            .aliases
                            .iter()
                            .map(|alias| (f.name.clone(), alias.clone())),
                    )
                })
                .collect();
            let fields_array = active
                .iter()
                .filter(|f| !f.attrs.flatten)
                .map(|f| format!("{:?}", f.de_wire_name()))
                .collect::<Vec<_>>()
                .join(", ");
            let map_body = visit_map_body(
                "__Field",
                fields,
                &constructor,
                "__A::Error",
                deny_unknown_fields,
            );
            let v = visitor("__StructVisitor", generics);
            format!(
                "{ident_enum}\n\
                 {def}\n\
                 impl{impl_decl} ::rusty_serde::de::Visitor<'de> for {vty}{where_clause} {{\n\
                     type Value = {enum_ty};\n\
                     fn expecting(&self, f: &mut ::std::fmt::Formatter) -> ::std::fmt::Result {{\n\
                         f.write_str(\"struct variant {enum_name}::{vname}\")\n\
                     }}\n\
                     fn visit_map<__A>(self, mut map: __A) -> Result<{enum_ty}, __A::Error>\n\
                     where __A: ::rusty_serde::de::MapAccess<'de> {{\n\
                         {map_body}\n\
                     }}\n\
                 }}\n\
                 const __FIELDS: &[&str] = &[{fields_array}];\n\
                 ::rusty_serde::Deserializer::deserialize_struct(\
                     ::rusty_serde::value::ValueDeserializer::<__D::Error>::new(__value.clone()), {vname:?}, __FIELDS, {construct})",
                ident_enum = ident_enum("__Field", &entries, "field identifier", IdentFallback::IgnoreUnknown),
                def = v.def,
                impl_decl = v.impl_decl,
                vty = v.ty,
                where_clause = v.where_clause,
                construct = v.construct,
            )
        }
    }
}

/// `enum_name` is the bare path prefix used to build constructor
/// expressions (`Foo::Variant`, generics omitted and inferred); `enum_ty`
/// is the full `Foo<'a, T>` used anywhere a *type* (not a value) is named.
fn variant_arm(
    enum_name: &str,
    enum_ty: &str,
    generics: &Generics,
    vname: &str,
    fields: &Fields,
    deny_unknown_fields: bool,
) -> String {
    let constructor = format!("{enum_name}::{vname}");
    match fields {
        Fields::Unit => format!(
            "(__Field::{vname}, __variant) => {{\n\
                 ::rusty_serde::de::VariantAccess::unit_variant(__variant)?;\n\
                 Ok({constructor})\n\
             }}\n"
        ),
        Fields::Unnamed(0) => format!(
            "(__Field::{vname}, __variant) => {{\n\
                 ::rusty_serde::de::VariantAccess::unit_variant(__variant)?;\n\
                 Ok({constructor}())\n\
             }}\n"
        ),
        Fields::Unnamed(1) => format!(
            "(__Field::{vname}, __variant) => {{\n\
                 let __value = ::rusty_serde::de::VariantAccess::newtype_variant(__variant)?;\n\
                 Ok({constructor}(__value))\n\
             }}\n"
        ),
        Fields::Unnamed(n) => {
            let mut elems = String::new();
            let mut binders = String::new();
            for i in 0..*n {
                elems += &format!(
                    "let __v{i} = ::rusty_serde::de::SeqAccess::next_element(&mut seq)?\n\
                             .ok_or_else(|| ::rusty_serde::Error::custom({msg:?}))?;\n",
                    msg = format!("missing tuple element {i}")
                );
                binders += &format!("__v{i}, ");
            }
            let v = visitor("__TupleVisitor", generics);
            format!(
                "(__Field::{vname}, __variant) => {{\n\
                     {def}\n\
                     impl{impl_decl} ::rusty_serde::de::Visitor<'de> for {vty}{where_clause} {{\n\
                         type Value = {enum_ty};\n\
                         fn expecting(&self, f: &mut ::std::fmt::Formatter) -> ::std::fmt::Result {{\n\
                             f.write_str(\"tuple variant {enum_name}::{vname}\")\n\
                         }}\n\
                         fn visit_seq<__A>(self, mut seq: __A) -> Result<{enum_ty}, __A::Error>\n\
                         where __A: ::rusty_serde::de::SeqAccess<'de> {{\n\
                             {elems}\n\
                             Ok({constructor}({binders}))\n\
                         }}\n\
                     }}\n\
                     ::rusty_serde::de::VariantAccess::tuple_variant(__variant, {n}, {construct})\n\
                 }}\n",
                def = v.def,
                impl_decl = v.impl_decl,
                where_clause = v.where_clause,
                vty = v.ty,
                construct = v.construct,
            )
        }
        Fields::Named(fields) => {
            let active: Vec<&NamedField> = fields
                .iter()
                .filter(|f| !f.attrs.skips_deserializing())
                .collect();
            let entries: Vec<(String, String)> = active
                .iter()
                .filter(|f| !f.attrs.flatten)
                .flat_map(|f| {
                    std::iter::once((f.name.clone(), f.de_wire_name().to_string())).chain(
                        f.attrs
                            .aliases
                            .iter()
                            .map(|alias| (f.name.clone(), alias.clone())),
                    )
                })
                .collect();
            let fields_array = active
                .iter()
                .filter(|f| !f.attrs.flatten)
                .map(|f| format!("{:?}", f.de_wire_name()))
                .collect::<Vec<_>>()
                .join(", ");
            let map_body = visit_map_body(
                "__SField",
                fields,
                &constructor,
                "__A::Error",
                deny_unknown_fields,
            );
            let v = visitor("__StructVisitor", generics);
            format!(
                "(__Field::{vname}, __variant) => {{\n\
                     {ident_enum}\n\
                     {def}\n\
                     impl{impl_decl} ::rusty_serde::de::Visitor<'de> for {vty}{where_clause} {{\n\
                         type Value = {enum_ty};\n\
                         fn expecting(&self, f: &mut ::std::fmt::Formatter) -> ::std::fmt::Result {{\n\
                             f.write_str(\"struct variant {enum_name}::{vname}\")\n\
                         }}\n\
                         fn visit_map<__A>(self, mut map: __A) -> Result<{enum_ty}, __A::Error>\n\
                         where __A: ::rusty_serde::de::MapAccess<'de> {{\n\
                             {map_body}\n\
                         }}\n\
                     }}\n\
                     const __SFIELDS: &[&str] = &[{fields_array}];\n\
                     ::rusty_serde::de::VariantAccess::struct_variant(__variant, __SFIELDS, {construct})\n\
                 }}\n",
                ident_enum = ident_enum("__SField", &entries, "field identifier", IdentFallback::IgnoreUnknown),
                def = v.def,
                impl_decl = v.impl_decl,
                where_clause = v.where_clause,
                vty = v.ty,
                construct = v.construct,
            )
        }
    }
}
