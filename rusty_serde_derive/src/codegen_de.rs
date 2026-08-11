use crate::parse::{Data, Fields, Generics, Variant};

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
        } => enum_impl(name, generics, variants),
    }
}

/// `enum {ty} { name0, name1, ..., [__ignore] }` plus a `Deserialize` impl
/// that maps a JSON-style string identifier to one of those variants. Used
/// both for a struct's field names and an enum's variant names. This
/// generated enum only ever holds identifier tags, so - unlike the "real"
/// visitor types below - it never needs the outer type's own generics.
fn ident_enum(ty: &str, names: &[String], expecting: &str, allow_unknown: bool) -> String {
    let mut decls: Vec<String> = names.to_vec();
    if allow_unknown {
        decls.push("__ignore".to_string());
    }
    let decls = decls.join(", ");

    let mut arms = String::new();
    for n in names {
        arms += &format!("                            {n:?} => Ok({ty}::{n}),\n");
    }
    let fallback = if allow_unknown {
        format!("                            _ => Ok({ty}::__ignore),\n")
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
        impl_decl: generics.impl_decl(Some("'de"), "::rusty_serde::Deserialize<'de>"),
        ty: format!("{struct_name}{use_site}"),
        construct: format!("{struct_name} {{ __marker: ::std::marker::PhantomData }}"),
    }
}

/// Body of a `visit_map` that fills in `field_names` from a `MapAccess`,
/// erroring on duplicates/missing fields, then builds `constructor { .. }`.
/// `constructor` is a bare (non-generic) path like `Foo` or `Foo::Variant`:
/// Rust infers any generic arguments from the enclosing function's return
/// type, so it never needs to be spelled out here.
fn visit_map_body(field_enum: &str, field_names: &[String], constructor: &str) -> String {
    let mut out = String::new();
    for f in field_names {
        out += &format!("let mut __{f}: Option<_> = None;\n");
    }
    out += &format!(
        "while let Some(__key) = ::rusty_serde::de::MapAccess::next_key::<{field_enum}>(&mut map)? {{\n\
             match __key {{\n"
    );
    for f in field_names {
        out += &format!(
            "                {field_enum}::{f} => {{\n\
                     if __{f}.is_some() {{\n\
                         return Err(::rusty_serde::Error::custom({dup:?}));\n\
                     }}\n\
                     __{f} = Some(::rusty_serde::de::MapAccess::next_value(&mut map)?);\n\
                 }}\n",
            dup = format!("duplicate field `{f}`")
        );
    }
    out += &format!(
        "                {field_enum}::__ignore => {{\n\
                     let _ = ::rusty_serde::de::MapAccess::next_value::<::rusty_serde::de::IgnoredAny>(&mut map)?;\n\
                 }}\n\
             }}\n\
         }}\n"
    );
    for f in field_names {
        out += &format!(
            "let {f} = __{f}.ok_or_else(|| ::rusty_serde::Error::custom({missing:?}))?;\n",
            missing = format!("missing field `{f}`")
        );
    }
    let field_list = field_names.join(", ");
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
                 impl{impl_decl} ::rusty_serde::de::Visitor<'de> for {vty} {{\n\
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
                vty = v.ty,
                construct = v.construct,
            )
        }
        Fields::Unnamed(0) => {
            let v = visitor("__Visitor", generics);
            format!(
                "{def}\n\
                 impl{impl_decl} ::rusty_serde::de::Visitor<'de> for {vty} {{\n\
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
                vty = v.ty,
                construct = v.construct,
            )
        }
        Fields::Unnamed(1) => {
            let v = visitor("__Visitor", generics);
            format!(
                "{def}\n\
                 impl{impl_decl} ::rusty_serde::de::Visitor<'de> for {vty} {{\n\
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
                 impl{impl_decl} ::rusty_serde::de::Visitor<'de> for {vty} {{\n\
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
                vty = v.ty,
                construct = v.construct,
            )
        }
        Fields::Named(field_names) => {
            let field_list_dbg: Vec<String> = field_names.iter().map(|f| format!("{f:?}")).collect();
            let fields_array = field_list_dbg.join(", ");
            let map_body = visit_map_body("__Field", field_names, name);
            let v = visitor("__Visitor", generics);
            format!(
                "{ident_enum}\n\
                 {def}\n\
                 impl{impl_decl} ::rusty_serde::de::Visitor<'de> for {vty} {{\n\
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
                ident_enum = ident_enum("__Field", field_names, "field identifier", true),
                def = v.def,
                impl_decl = v.impl_decl,
                vty = v.ty,
                construct = v.construct,
            )
        }
    };

    let impl_decl = generics.impl_decl(Some("'de"), "::rusty_serde::Deserialize<'de>");
    format!(
        "impl{impl_decl} ::rusty_serde::Deserialize<'de> for {ty} {{\n\
             fn deserialize<__D>(deserializer: __D) -> Result<Self, __D::Error>\n\
             where\n\
                 __D: ::rusty_serde::Deserializer<'de>,\n\
             {{\n\
                 {body}\n\
             }}\n\
         }}\n"
    )
}

fn enum_impl(name: &str, generics: &Generics, variants: &[Variant]) -> String {
    let ty = generics.ty(name);
    let variant_names: Vec<String> = variants.iter().map(|v| v.name.clone()).collect();
    let variant_list_dbg: Vec<String> = variant_names.iter().map(|v| format!("{v:?}")).collect();
    let variants_array = variant_list_dbg.join(", ");

    let mut arms = String::new();
    for variant in variants {
        arms += &variant_arm(name, &ty, generics, &variant.name, &variant.fields);
    }

    let v = visitor("__Visitor", generics);
    let impl_decl = generics.impl_decl(Some("'de"), "::rusty_serde::Deserialize<'de>");
    format!(
        "impl{impl_decl} ::rusty_serde::Deserialize<'de> for {ty} {{\n\
             fn deserialize<__D>(deserializer: __D) -> Result<Self, __D::Error>\n\
             where\n\
                 __D: ::rusty_serde::Deserializer<'de>,\n\
             {{\n\
                 {ident_enum}\n\
                 {def}\n\
                 impl{v_impl_decl} ::rusty_serde::de::Visitor<'de> for {vty} {{\n\
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
                 ::rusty_serde::Deserializer::deserialize_enum(deserializer, {name:?}, __VARIANTS, {construct})\n\
             }}\n\
         }}\n",
        ident_enum = ident_enum("__Field", &variant_names, "variant identifier", false),
        def = v.def,
        v_impl_decl = v.impl_decl,
        vty = v.ty,
        construct = v.construct,
    )
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
                     impl{impl_decl} ::rusty_serde::de::Visitor<'de> for {vty} {{\n\
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
                vty = v.ty,
                construct = v.construct,
            )
        }
        Fields::Named(field_names) => {
            let field_list_dbg: Vec<String> = field_names.iter().map(|f| format!("{f:?}")).collect();
            let fields_array = field_list_dbg.join(", ");
            let map_body = visit_map_body("__SField", field_names, &constructor);
            let v = visitor("__StructVisitor", generics);
            format!(
                "(__Field::{vname}, __variant) => {{\n\
                     {ident_enum}\n\
                     {def}\n\
                     impl{impl_decl} ::rusty_serde::de::Visitor<'de> for {vty} {{\n\
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
                ident_enum = ident_enum("__SField", field_names, "field identifier", true),
                def = v.def,
                impl_decl = v.impl_decl,
                vty = v.ty,
                construct = v.construct,
            )
        }
    }
}
