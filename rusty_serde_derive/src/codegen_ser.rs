use crate::parse::{Data, Fields, Generics, NamedField, Variant};

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

fn struct_impl(name: &str, generics: &Generics, fields: &Fields) -> String {
    let body = match fields {
        Fields::Unit => format!("::rusty_serde::Serializer::serialize_unit_struct(serializer, {name:?})"),
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
        Fields::Named(fields) => {
            let active: Vec<&NamedField> = fields.iter().filter(|f| !f.attrs.skip).collect();
            let n = active.len();
            let mut out = format!(
                "let mut __state = ::rusty_serde::Serializer::serialize_struct(serializer, {name:?}, {n})?;\n"
            );
            for field in &active {
                let wire = field.wire_name();
                let ident = &field.name;
                out += &format!(
                    "::rusty_serde::ser::SerializeStruct::serialize_field(&mut __state, {wire:?}, &self.{ident})?;\n"
                );
            }
            out += "::rusty_serde::ser::SerializeStruct::end(__state)";
            out
        }
    };

    let impl_decl = generics.impl_decl(None, "::rusty_serde::Serialize");
    let ty = generics.ty(name);
    format!(
        "impl{impl_decl} ::rusty_serde::Serialize for {ty} {{\n\
             fn serialize<__S>(&self, serializer: __S) -> Result<__S::Ok, __S::Error>\n\
             where\n\
                 __S: ::rusty_serde::Serializer,\n\
             {{\n\
                 {body}\n\
             }}\n\
         }}\n"
    )
}

fn enum_impl(name: &str, generics: &Generics, variants: &[Variant]) -> String {
    let mut arms = String::new();
    for (index, variant) in variants.iter().enumerate() {
        arms += &variant_arm(name, index as u32, variant);
    }

    let impl_decl = generics.impl_decl(None, "::rusty_serde::Serialize");
    let ty = generics.ty(name);
    format!(
        "impl{impl_decl} ::rusty_serde::Serialize for {ty} {{\n\
             fn serialize<__S>(&self, serializer: __S) -> Result<__S::Ok, __S::Error>\n\
             where\n\
                 __S: ::rusty_serde::Serializer,\n\
             {{\n\
                 match *self {{\n{arms}}}\n\
             }}\n\
         }}\n"
    )
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
            let binders = fields
                .iter()
                .map(|f| {
                    if f.attrs.skip {
                        format!("{}: _", f.name)
                    } else {
                        format!("ref {}", f.name)
                    }
                })
                .collect::<Vec<_>>()
                .join(", ");
            let active: Vec<&NamedField> = fields.iter().filter(|f| !f.attrs.skip).collect();
            let n = active.len();
            let mut body = format!(
                "let mut __state = ::rusty_serde::Serializer::serialize_struct_variant(serializer, {enum_name:?}, {index}, {wire_vname:?}, {n})?;\n"
            );
            for f in &active {
                let wire = f.wire_name();
                let ident = &f.name;
                body += &format!(
                    "        ::rusty_serde::ser::SerializeStructVariant::serialize_field(&mut __state, {wire:?}, {ident})?;\n"
                );
            }
            body += "        ::rusty_serde::ser::SerializeStructVariant::end(__state)";
            format!("    {enum_name}::{vname} {{ {binders} }} => {{\n        {body}\n    }}\n")
        }
    }
}
