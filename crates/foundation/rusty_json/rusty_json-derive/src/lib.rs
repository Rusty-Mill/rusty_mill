//! Derive proc-macro reserved for sovereign `RustyJson` serialization.
//!
//! Not implemented: `#[derive(RustyJson)]` fails to compile, naming the
//! working path, rather than generating methods that serialize nothing and
//! never deserialize.
//!
//! ```compile_fail
//! extern crate alloc;
//! use rusty_json_derive::RustyJson;
//!
//! #[derive(RustyJson)]
//! struct Point {
//!     x: i32,
//! }
//! ```

extern crate proc_macro;
use proc_macro::{Delimiter, Group, Ident, Literal, Punct, Spacing, Span, TokenStream, TokenTree};

const NOT_IMPLEMENTED: &str = "#[derive(RustyJson)] is not implemented yet; derive \
     serde::Serialize/Deserialize and use rusty_json's `serde` feature \
     (rusty_json::to_string / rusty_json::from_str) instead";

/// Reserved derive for `RustyJson`. Always expands to a `compile_error!`
/// explaining that it is not implemented and what to use instead.
#[proc_macro_derive(RustyJson)]
pub fn derive_rusty_json(_input: TokenStream) -> TokenStream {
    compile_error(NOT_IMPLEMENTED)
}

/// `compile_error!("<message>");`, built token by token.
fn compile_error(message: &str) -> TokenStream {
    let span = Span::call_site();
    let mut args = TokenTree::Group(Group::new(
        Delimiter::Parenthesis,
        TokenTree::Literal(Literal::string(message)).into(),
    ));
    args.set_span(span);
    [
        TokenTree::Ident(Ident::new("compile_error", span)),
        TokenTree::Punct(Punct::new('!', Spacing::Alone)),
        args,
        TokenTree::Punct(Punct::new(';', Spacing::Alone)),
    ]
    .into_iter()
    .collect()
}
