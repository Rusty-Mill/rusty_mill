//! Parses the optional argument list on `#[rusty_tokio::main(...)]` /
//! `#[rusty_tokio::test(...)]` -- `worker_threads = N`, and (behind the
//! `thread-per-core` feature) `flavor = "thread_per_core"` to select
//! [`rusty_tokio::Builder::new_thread_per_core`] instead of the default
//! multi-threaded builder. More of tokio's real options (`start_paused`,
//! ...) don't apply -- no pausable clock yet (issue #56), and there's no
//! `current_thread` macro flavor since nothing here builds a `LocalSet`
//! for it (`Builder::build_local`/`LocalRuntime` have to be called
//! directly for that).

use proc_macro2::TokenStream;
use quote::quote;
use syn::parse::{Parse, ParseStream};
use syn::punctuated::Punctuated;
use syn::{Expr, ExprLit, Lit, Meta, Token};

pub(crate) struct MacroArgs {
    worker_threads: Option<usize>,
    flavor: Option<String>,
}

impl Parse for MacroArgs {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let metas = Punctuated::<Meta, Token![,]>::parse_terminated(input)?;
        let mut worker_threads = None;
        let mut flavor = None;

        for meta in metas {
            match meta {
                Meta::NameValue(nv) if nv.path.is_ident("worker_threads") => {
                    worker_threads = Some(literal_usize(&nv.value)?);
                }
                Meta::NameValue(nv) if nv.path.is_ident("flavor") => {
                    let value = literal_str(&nv.value)?;
                    if value != "thread_per_core" {
                        return Err(syn::Error::new_spanned(
                            &nv.value,
                            "unsupported `flavor` -- only \"thread_per_core\" is supported \
                             (the default, multi-threaded flavor needs no `flavor` argument \
                             at all)",
                        ));
                    }
                    flavor = Some(value);
                }
                other => {
                    return Err(syn::Error::new_spanned(
                        other,
                        "unsupported argument -- only `worker_threads = N` and \
                         `flavor = \"thread_per_core\"` are supported",
                    ));
                }
            }
        }

        Ok(MacroArgs {
            worker_threads,
            flavor,
        })
    }
}

fn literal_str(expr: &Expr) -> syn::Result<String> {
    match expr {
        Expr::Lit(ExprLit {
            lit: Lit::Str(s), ..
        }) => Ok(s.value()),
        other => Err(syn::Error::new_spanned(other, "expected a string literal")),
    }
}

fn literal_usize(expr: &Expr) -> syn::Result<usize> {
    match expr {
        Expr::Lit(ExprLit {
            lit: Lit::Int(int), ..
        }) => int.base10_parse::<usize>(),
        other => Err(syn::Error::new_spanned(
            other,
            "expected an integer literal",
        )),
    }
}

impl MacroArgs {
    /// The `rusty_tokio::Runtime` construction expression to block on
    /// the annotated function's body with.
    pub(crate) fn runtime_expr(&self) -> TokenStream {
        let builder = match self.flavor.as_deref() {
            Some("thread_per_core") => quote! { ::rusty_tokio::Builder::new_thread_per_core() },
            _ => quote! { ::rusty_tokio::Builder::new() },
        };
        match self.worker_threads {
            Some(n) => quote! {
                #builder.worker_threads(#n).build().unwrap()
            },
            None => quote! {
                #builder.build().unwrap()
            },
        }
    }
}
