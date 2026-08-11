//! `#[derive(Serialize)]` / `#[derive(Deserialize)]` for `rusty_serde`,
//! written directly against the compiler-provided `proc_macro` crate - no
//! `syn`, no `quote`, no crates.io dependencies at all.
//!
//! Field/variant *types* are never parsed: the generated code calls
//! `Serialize`/`Deserialize` generically and lets Rust's own type inference
//! fill in the rest, which is what keeps a hand-written parser tractable.
//! The tradeoff is that generic structs/enums aren't supported yet (see
//! `parse::reject_generics`).

use proc_macro::TokenStream;

mod codegen_de;
mod codegen_ser;
mod parse;

#[proc_macro_derive(Serialize)]
pub fn derive_serialize(input: TokenStream) -> TokenStream {
    expand(input, codegen_ser::generate)
}

#[proc_macro_derive(Deserialize)]
pub fn derive_deserialize(input: TokenStream) -> TokenStream {
    expand(input, codegen_de::generate)
}

fn expand(input: TokenStream, codegen: fn(&parse::Data) -> String) -> TokenStream {
    let data = match parse::parse(input) {
        Ok(data) => data,
        Err(err) => return err,
    };

    let code = codegen(&data);
    code.parse().unwrap_or_else(|err| {
        panic!("rusty_serde_derive generated code that failed to parse: {err:?}\n---\n{code}\n---")
    })
}
