use crate::parse::{Data, Fields, Variant};

pub fn generate(data: &Data) -> String {
    match data {
        Data::Struct { name, fields } => struct_impl(name, fields),
        Data::Enum { name, variants } => enum_impl(name, variants),
    }
}

fn struct_impl(name: &str, fields: &Fields) -> String {
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
        Fields::Named(field_names) => {
            let n = field_names.len();
            let mut out = format!(
                "let mut __state = ::rusty_serde::Serializer::serialize_struct(serializer, {name:?}, {n})?;\n"
            );
            for field in field_names {
                out += &format!(
                    "::rusty_serde::ser::SerializeStruct::serialize_field(&mut __state, {field:?}, &self.{field})?;\n"
                );
            }
            out += "::rusty_serde::ser::SerializeStruct::end(__state)";
            out
        }
    };

    format!(
        "impl ::rusty_serde::Serialize for {name} {{\n\
             fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>\n\
             where\n\
                 S: ::rusty_serde::Serializer,\n\
             {{\n\
                 {body}\n\
             }}\n\
         }}\n"
    )
}

fn enum_impl(name: &str, variants: &[Variant]) -> String {
    let mut arms = String::new();
    for (index, variant) in variants.iter().enumerate() {
        let vname = &variant.name;
        arms += &variant_arm(name, index as u32, vname, &variant.fields);
    }

    format!(
        "impl ::rusty_serde::Serialize for {name} {{\n\
             fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>\n\
             where\n\
                 S: ::rusty_serde::Serializer,\n\
             {{\n\
                 match *self {{\n{arms}}}\n\
             }}\n\
         }}\n"
    )
}

fn variant_arm(enum_name: &str, index: u32, vname: &str, fields: &Fields) -> String {
    match fields {
        Fields::Unit => format!(
            "    {enum_name}::{vname} => ::rusty_serde::Serializer::serialize_unit_variant(serializer, {enum_name:?}, {index}, {vname:?}),\n"
        ),
        Fields::Unnamed(0) => format!(
            "    {enum_name}::{vname}() => ::rusty_serde::Serializer::serialize_unit_variant(serializer, {enum_name:?}, {index}, {vname:?}),\n"
        ),
        Fields::Unnamed(1) => format!(
            "    {enum_name}::{vname}(ref __f0) => ::rusty_serde::Serializer::serialize_newtype_variant(serializer, {enum_name:?}, {index}, {vname:?}, __f0),\n"
        ),
        Fields::Unnamed(n) => {
            let binders = (0..*n).map(|i| format!("ref __f{i}")).collect::<Vec<_>>().join(", ");
            let mut body = format!(
                "let mut __state = ::rusty_serde::Serializer::serialize_tuple_variant(serializer, {enum_name:?}, {index}, {vname:?}, {n})?;\n"
            );
            for i in 0..*n {
                body += &format!(
                    "        ::rusty_serde::ser::SerializeTupleVariant::serialize_field(&mut __state, __f{i})?;\n"
                );
            }
            body += "        ::rusty_serde::ser::SerializeTupleVariant::end(__state)";
            format!("    {enum_name}::{vname}({binders}) => {{\n        {body}\n    }}\n")
        }
        Fields::Named(field_names) => {
            let binders = field_names
                .iter()
                .map(|f| format!("ref {f}"))
                .collect::<Vec<_>>()
                .join(", ");
            let n = field_names.len();
            let mut body = format!(
                "let mut __state = ::rusty_serde::Serializer::serialize_struct_variant(serializer, {enum_name:?}, {index}, {vname:?}, {n})?;\n"
            );
            for f in field_names {
                body += &format!(
                    "        ::rusty_serde::ser::SerializeStructVariant::serialize_field(&mut __state, {f:?}, {f})?;\n"
                );
            }
            body += "        ::rusty_serde::ser::SerializeStructVariant::end(__state)";
            format!("    {enum_name}::{vname} {{ {binders} }} => {{\n        {body}\n    }}\n")
        }
    }
}
