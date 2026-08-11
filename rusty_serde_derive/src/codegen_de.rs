use crate::parse::{Data, Fields, Variant};

pub fn generate(data: &Data) -> String {
    match data {
        Data::Struct { name, fields } => struct_impl(name, fields),
        Data::Enum { name, variants } => enum_impl(name, variants),
    }
}

/// `enum {ty} { name0, name1, ..., [__ignore] }` plus a `Deserialize` impl
/// that maps a JSON-style string identifier to one of those variants. Used
/// both for a struct's field names and an enum's variant names.
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
             fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>\n\
             where\n\
                 D: ::rusty_serde::Deserializer<'de>,\n\
             {{\n\
                 struct __IdentVisitor;\n\
                 impl<'de> ::rusty_serde::de::Visitor<'de> for __IdentVisitor {{\n\
                     type Value = {ty};\n\
                     fn expecting(&self, f: &mut ::std::fmt::Formatter) -> ::std::fmt::Result {{\n\
                         f.write_str({expecting:?})\n\
                     }}\n\
                     fn visit_str<E>(self, value: &str) -> Result<{ty}, E>\n\
                     where\n\
                         E: ::rusty_serde::Error,\n\
                     {{\n\
                         match value {{\n{arms}{fallback}                        }}\n\
                     }}\n\
                 }}\n\
                 deserializer.deserialize_identifier(__IdentVisitor)\n\
             }}\n\
         }}\n"
    )
}

/// Body of a `visit_map` that fills in `field_names` from a `MapAccess`,
/// erroring on duplicates/missing fields, then builds `constructor { .. }`.
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

fn struct_impl(name: &str, fields: &Fields) -> String {
    let body = match fields {
        Fields::Unit => format!(
            "struct __Visitor;\n\
             impl<'de> ::rusty_serde::de::Visitor<'de> for __Visitor {{\n\
                 type Value = {name};\n\
                 fn expecting(&self, f: &mut ::std::fmt::Formatter) -> ::std::fmt::Result {{\n\
                     f.write_str(\"unit struct {name}\")\n\
                 }}\n\
                 fn visit_unit<E>(self) -> Result<{name}, E>\n\
                 where E: ::rusty_serde::Error {{\n\
                     Ok({name})\n\
                 }}\n\
             }}\n\
             ::rusty_serde::Deserializer::deserialize_unit_struct(deserializer, {name:?}, __Visitor)"
        ),
        Fields::Unnamed(0) => format!(
            "struct __Visitor;\n\
             impl<'de> ::rusty_serde::de::Visitor<'de> for __Visitor {{\n\
                 type Value = {name};\n\
                 fn expecting(&self, f: &mut ::std::fmt::Formatter) -> ::std::fmt::Result {{\n\
                     f.write_str(\"unit struct {name}\")\n\
                 }}\n\
                 fn visit_unit<E>(self) -> Result<{name}, E>\n\
                 where E: ::rusty_serde::Error {{\n\
                     Ok({name}())\n\
                 }}\n\
             }}\n\
             ::rusty_serde::Deserializer::deserialize_unit_struct(deserializer, {name:?}, __Visitor)"
        ),
        Fields::Unnamed(1) => format!(
            "struct __Visitor;\n\
             impl<'de> ::rusty_serde::de::Visitor<'de> for __Visitor {{\n\
                 type Value = {name};\n\
                 fn expecting(&self, f: &mut ::std::fmt::Formatter) -> ::std::fmt::Result {{\n\
                     f.write_str(\"tuple struct {name}\")\n\
                 }}\n\
                 fn visit_newtype_struct<D>(self, deserializer: D) -> Result<{name}, D::Error>\n\
                 where D: ::rusty_serde::Deserializer<'de> {{\n\
                     ::rusty_serde::Deserialize::deserialize(deserializer).map({name})\n\
                 }}\n\
                 fn visit_seq<A>(self, mut seq: A) -> Result<{name}, A::Error>\n\
                 where A: ::rusty_serde::de::SeqAccess<'de> {{\n\
                     let __v0 = ::rusty_serde::de::SeqAccess::next_element(&mut seq)?\n\
                         .ok_or_else(|| ::rusty_serde::Error::custom(\"missing tuple element 0\"))?;\n\
                     Ok({name}(__v0))\n\
                 }}\n\
             }}\n\
             ::rusty_serde::Deserializer::deserialize_newtype_struct(deserializer, {name:?}, __Visitor)"
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
            format!(
                "struct __Visitor;\n\
                 impl<'de> ::rusty_serde::de::Visitor<'de> for __Visitor {{\n\
                     type Value = {name};\n\
                     fn expecting(&self, f: &mut ::std::fmt::Formatter) -> ::std::fmt::Result {{\n\
                         f.write_str(\"tuple struct {name}\")\n\
                     }}\n\
                     fn visit_seq<A>(self, mut seq: A) -> Result<{name}, A::Error>\n\
                     where A: ::rusty_serde::de::SeqAccess<'de> {{\n\
                         {elems}\n\
                         Ok({name}({binders}))\n\
                     }}\n\
                 }}\n\
                 ::rusty_serde::Deserializer::deserialize_tuple_struct(deserializer, {name:?}, {n}, __Visitor)"
            )
        }
        Fields::Named(field_names) => {
            let field_list_dbg: Vec<String> = field_names.iter().map(|f| format!("{f:?}")).collect();
            let fields_array = field_list_dbg.join(", ");
            let map_body = visit_map_body("__Field", field_names, name);
            format!(
                "{ident_enum}\n\
                 struct __Visitor;\n\
                 impl<'de> ::rusty_serde::de::Visitor<'de> for __Visitor {{\n\
                     type Value = {name};\n\
                     fn expecting(&self, f: &mut ::std::fmt::Formatter) -> ::std::fmt::Result {{\n\
                         f.write_str(\"struct {name}\")\n\
                     }}\n\
                     fn visit_map<A>(self, mut map: A) -> Result<{name}, A::Error>\n\
                     where A: ::rusty_serde::de::MapAccess<'de> {{\n\
                         {map_body}\n\
                     }}\n\
                 }}\n\
                 const __FIELDS: &[&str] = &[{fields_array}];\n\
                 ::rusty_serde::Deserializer::deserialize_struct(deserializer, {name:?}, __FIELDS, __Visitor)",
                ident_enum = ident_enum("__Field", field_names, "field identifier", true),
            )
        }
    };

    format!(
        "impl<'de> ::rusty_serde::Deserialize<'de> for {name} {{\n\
             fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>\n\
             where\n\
                 D: ::rusty_serde::Deserializer<'de>,\n\
             {{\n\
                 {body}\n\
             }}\n\
         }}\n"
    )
}

fn enum_impl(name: &str, variants: &[Variant]) -> String {
    let variant_names: Vec<String> = variants.iter().map(|v| v.name.clone()).collect();
    let variant_list_dbg: Vec<String> = variant_names.iter().map(|v| format!("{v:?}")).collect();
    let variants_array = variant_list_dbg.join(", ");

    let mut arms = String::new();
    for variant in variants {
        arms += &variant_arm(name, &variant.name, &variant.fields);
    }

    format!(
        "impl<'de> ::rusty_serde::Deserialize<'de> for {name} {{\n\
             fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>\n\
             where\n\
                 D: ::rusty_serde::Deserializer<'de>,\n\
             {{\n\
                 {ident_enum}\n\
                 struct __Visitor;\n\
                 impl<'de> ::rusty_serde::de::Visitor<'de> for __Visitor {{\n\
                     type Value = {name};\n\
                     fn expecting(&self, f: &mut ::std::fmt::Formatter) -> ::std::fmt::Result {{\n\
                         f.write_str(\"enum {name}\")\n\
                     }}\n\
                     fn visit_enum<A>(self, data: A) -> Result<{name}, A::Error>\n\
                     where A: ::rusty_serde::de::EnumAccess<'de> {{\n\
                         match ::rusty_serde::de::EnumAccess::variant(data)? {{\n\
                             {arms}\n\
                         }}\n\
                     }}\n\
                 }}\n\
                 const __VARIANTS: &[&str] = &[{variants_array}];\n\
                 ::rusty_serde::Deserializer::deserialize_enum(deserializer, {name:?}, __VARIANTS, __Visitor)\n\
             }}\n\
         }}\n",
        ident_enum = ident_enum("__Field", &variant_names, "variant identifier", false),
    )
}

fn variant_arm(enum_name: &str, vname: &str, fields: &Fields) -> String {
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
            format!(
                "(__Field::{vname}, __variant) => {{\n\
                     struct __TupleVisitor;\n\
                     impl<'de> ::rusty_serde::de::Visitor<'de> for __TupleVisitor {{\n\
                         type Value = {enum_name};\n\
                         fn expecting(&self, f: &mut ::std::fmt::Formatter) -> ::std::fmt::Result {{\n\
                             f.write_str(\"tuple variant {enum_name}::{vname}\")\n\
                         }}\n\
                         fn visit_seq<A>(self, mut seq: A) -> Result<{enum_name}, A::Error>\n\
                         where A: ::rusty_serde::de::SeqAccess<'de> {{\n\
                             {elems}\n\
                             Ok({constructor}({binders}))\n\
                         }}\n\
                     }}\n\
                     ::rusty_serde::de::VariantAccess::tuple_variant(__variant, {n}, __TupleVisitor)\n\
                 }}\n"
            )
        }
        Fields::Named(field_names) => {
            let field_list_dbg: Vec<String> = field_names.iter().map(|f| format!("{f:?}")).collect();
            let fields_array = field_list_dbg.join(", ");
            let map_body = visit_map_body("__SField", field_names, &constructor);
            format!(
                "(__Field::{vname}, __variant) => {{\n\
                     {ident_enum}\n\
                     struct __StructVisitor;\n\
                     impl<'de> ::rusty_serde::de::Visitor<'de> for __StructVisitor {{\n\
                         type Value = {enum_name};\n\
                         fn expecting(&self, f: &mut ::std::fmt::Formatter) -> ::std::fmt::Result {{\n\
                             f.write_str(\"struct variant {enum_name}::{vname}\")\n\
                         }}\n\
                         fn visit_map<A>(self, mut map: A) -> Result<{enum_name}, A::Error>\n\
                         where A: ::rusty_serde::de::MapAccess<'de> {{\n\
                             {map_body}\n\
                         }}\n\
                     }}\n\
                     const __SFIELDS: &[&str] = &[{fields_array}];\n\
                     ::rusty_serde::de::VariantAccess::struct_variant(__variant, __SFIELDS, __StructVisitor)\n\
                 }}\n",
                ident_enum = ident_enum("__SField", field_names, "field identifier", true),
            )
        }
    }
}
