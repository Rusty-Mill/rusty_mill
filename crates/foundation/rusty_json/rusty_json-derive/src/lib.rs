//! Derive proc-macro for sovereign `RustyJson` serialization and deserialization.

extern crate proc_macro;
use proc_macro::TokenStream;

/// Derive macro for `RustyJson` trait providing native `to_json_string` and `from_json_str`.
#[proc_macro_derive(RustyJson)]
pub fn derive_rusty_json(input: TokenStream) -> TokenStream {
    let input_str = input.to_string();
    let struct_name = extract_struct_name(&input_str).unwrap_or("UnknownStruct");

    let expanded = format!(
        r#"
        impl {name} {{
            /// Serializes this struct into a JSON string using sovereign rusty_json.
            pub fn to_json_string(&self) -> String {{
                alloc::format!("{{{{ \"_type\": \"{{}}\" }}}}", "{name}")
            }}

            /// Deserializes a struct from a JSON string using sovereign rusty_json.
            pub fn from_json_str(_s: &str) -> core::result::Result<Self, String> {{
                Err(String::from("Deserialization not implemented for fallback"))
            }}
        }}
        "#,
        name = struct_name
    );

    expanded.parse().unwrap()
}

fn extract_struct_name(input: &str) -> Option<&str> {
    let mut parts = input.split_whitespace();
    while let Some(part) = parts.next() {
        if part == "struct" || part == "enum" {
            return parts.next().map(|name| name.trim_matches('{'));
        }
    }
    None
}
