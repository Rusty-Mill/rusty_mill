use crate::parse::{Data, DefaultAttr, Fields, Generics, NamedField, Variant};

pub fn generate(data: &Data) -> String {
    match data {
        Data::Struct {
            name,
            generics,
            fields,
        } => struct_impl(name, generics, fields),
        Data::Enum {
            name,
            generics,
            variants,
            tag,
            untagged,
        } => enum_impl(name, generics, variants, tag.as_deref(), *untagged),
    }
}

/// `enum {ty} { name0, name1, ..., [__ignore] }` plus a `Deserialize` impl
/// that maps a JSON-style string identifier to one of those variants. Used
/// both for a struct's field names and an enum's variant names. This
/// generated enum only ever holds identifier tags, so - unlike the "real"
/// visitor types below - it never needs the outer type's own generics.
///
/// `entries` pairs each Rust identifier (the enum variant name, and the
/// field/variant name used everywhere else in the generated code) with the
/// wire name to match against (its own name, unless renamed).
fn ident_enum(
    ty: &str,
    entries: &[(String, String)],
    expecting: &str,
    allow_unknown: bool,
) -> String {
    let mut decls: Vec<String> = entries.iter().map(|(ident, _)| ident.clone()).collect();
    if allow_unknown {
        // Carries the raw key text along, not just an "unknown" tag - a
        // flatten field needs it to rebuild the leftover entries; callers
        // that don't need it just match `__ignore(_)`.
        decls.push("__ignore(::std::string::String)".to_string());
    }
    let decls = decls.join(", ");

    let mut arms = String::new();
    for (ident, wire) in entries {
        arms += &format!("                            {wire:?} => Ok({ty}::{ident}),\n");
    }
    let fallback = if allow_unknown {
        format!("                            _ => Ok({ty}::__ignore(value.to_string())),\n")
    } else {
        "                            _ => Err(::rusty_serde::Error::custom(::std::format!(\"unknown variant `{}`\", value))),\n".to_string()
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
) -> String {
    let active: Vec<&NamedField> = fields.iter().filter(|f| !f.attrs.skip).collect();
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
        out += &format!(
            "                {field_enum}::{ident} => {{\n\
                     if __{ident}.is_some() {{\n\
                         return Err(::rusty_serde::Error::custom({dup:?}));\n\
                     }}\n\
                     __{ident} = Some(::rusty_serde::de::MapAccess::next_value(&mut map)?);\n\
                 }}\n",
            dup = format!("duplicate field `{}`", f.wire_name())
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
        if f.attrs.skip {
            out += &format!("let {ident} = ::std::default::Default::default();\n");
        } else if let Some(default) = &f.attrs.default {
            let fallback = match default {
                DefaultAttr::Trait => "::std::default::Default::default()".to_string(),
                DefaultAttr::Path(path) => format!("{path}()"),
            };
            out += &format!("let {ident} = __{ident}.unwrap_or_else(|| {fallback});\n");
        } else {
            out += &format!(
                "let {ident} = __{ident}.ok_or_else(|| ::rusty_serde::Error::custom({missing:?}))?;\n",
                missing = format!("missing field `{}`", f.wire_name())
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

fn struct_impl(name: &str, generics: &Generics, fields: &Fields) -> String {
    let ty = generics.ty(name);
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
                         Ok({name})\n\
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
                         Ok({name}())\n\
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
                         ::rusty_serde::Deserialize::deserialize(deserializer).map({name})\n\
                     }}\n\
                     fn visit_seq<__A>(self, mut seq: __A) -> Result<{ty}, __A::Error>\n\
                     where __A: ::rusty_serde::de::SeqAccess<'de> {{\n\
                         let __v0 = ::rusty_serde::de::SeqAccess::next_element(&mut seq)?\n\
                             .ok_or_else(|| ::rusty_serde::Error::custom(\"missing tuple element 0\"))?;\n\
                         Ok({name}(__v0))\n\
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
                         Ok({name}({binders}))\n\
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
        Fields::Named(fields) => {
            let active: Vec<&NamedField> = fields.iter().filter(|f| !f.attrs.skip).collect();
            let entries: Vec<(String, String)> = active
                .iter()
                .filter(|f| !f.attrs.flatten)
                .map(|f| (f.name.clone(), f.wire_name().to_string()))
                .collect();
            let fields_array = active
                .iter()
                .filter(|f| !f.attrs.flatten)
                .map(|f| format!("{:?}", f.wire_name()))
                .collect::<Vec<_>>()
                .join(", ");
            let map_body = visit_map_body("__Field", fields, name, "__A::Error");
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
                ident_enum = ident_enum("__Field", &entries, "field identifier", true),
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
    untagged: bool,
) -> String {
    if untagged {
        return enum_impl_untagged(name, generics, variants);
    }
    let ty = generics.ty(name);
    let variant_entries: Vec<(String, String)> = variants
        .iter()
        .map(|v| (v.name.clone(), v.wire_name().to_string()))
        .collect();
    let variants_array = variant_entries
        .iter()
        .map(|(_, wire)| format!("{wire:?}"))
        .collect::<Vec<_>>()
        .join(", ");

    let mut arms = String::new();
    for variant in variants {
        arms += &variant_arm(name, &ty, generics, &variant.name, &variant.fields);
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
        ident_enum = ident_enum("__Field", &variant_entries, "variant identifier", false),
        def = v.def,
        v_impl_decl = v.impl_decl,
        v_where_clause = v.where_clause,
        vty = v.ty,
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
fn enum_impl_untagged(name: &str, generics: &Generics, variants: &[Variant]) -> String {
    let ty = generics.ty(name);
    let mut attempts = String::new();
    for variant in variants {
        let body = untagged_variant_body(name, &ty, generics, variant);
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
            let active: Vec<&NamedField> = fields.iter().filter(|f| !f.attrs.skip).collect();
            let entries: Vec<(String, String)> = active
                .iter()
                .filter(|f| !f.attrs.flatten)
                .map(|f| (f.name.clone(), f.wire_name().to_string()))
                .collect();
            let fields_array = active
                .iter()
                .filter(|f| !f.attrs.flatten)
                .map(|f| format!("{:?}", f.wire_name()))
                .collect::<Vec<_>>()
                .join(", ");
            let map_body = visit_map_body("__Field", fields, &constructor, "__A::Error");
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
                ident_enum = ident_enum("__Field", &entries, "field identifier", true),
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
            let active: Vec<&NamedField> = fields.iter().filter(|f| !f.attrs.skip).collect();
            let entries: Vec<(String, String)> = active
                .iter()
                .filter(|f| !f.attrs.flatten)
                .map(|f| (f.name.clone(), f.wire_name().to_string()))
                .collect();
            let fields_array = active
                .iter()
                .filter(|f| !f.attrs.flatten)
                .map(|f| format!("{:?}", f.wire_name()))
                .collect::<Vec<_>>()
                .join(", ");
            let map_body = visit_map_body("__SField", fields, &constructor, "__A::Error");
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
                ident_enum = ident_enum("__SField", &entries, "field identifier", true),
                def = v.def,
                impl_decl = v.impl_decl,
                where_clause = v.where_clause,
                vty = v.ty,
                construct = v.construct,
            )
        }
    }
}
