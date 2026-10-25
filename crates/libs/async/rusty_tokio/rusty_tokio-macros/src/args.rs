//! Parses the optional argument list on `#[rusty_tokio::main(...)]` /
//! `#[rusty_tokio::test(...)]`: tokio's `flavor = "multi_thread" |
//! "current_thread"`, `worker_threads = N` and `start_paused = bool`,
//! plus this crate's own `flavor = "thread_per_core"` (behind the main
//! crate's `thread-per-core` feature).
//!
//! The combinations tokio rejects at compile time are rejected here too
//! (`worker_threads` on `current_thread`, `start_paused = true` on any
//! other flavor), with tokio's wording, so a migrated attribute either
//! means the same thing or fails to build -- never silently runs on a
//! different runtime than the one it names.

use proc_macro2::{Span, TokenStream};
use quote::quote;
use syn::parse::{Parse, ParseStream};
use syn::punctuated::Punctuated;
use syn::{Expr, ExprLit, Lit, Meta, Token};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Flavor {
    MultiThread,
    CurrentThread,
    ThreadPerCore,
}

impl Flavor {
    fn parse(value: &str, span: Span) -> syn::Result<Self> {
        match value {
            "multi_thread" => Ok(Flavor::MultiThread),
            "current_thread" => Ok(Flavor::CurrentThread),
            "thread_per_core" => Ok(Flavor::ThreadPerCore),
            other => Err(syn::Error::new(
                span,
                format!(
                    "unknown `flavor` \"{other}\" -- expected \"multi_thread\", \
                     \"current_thread\" or \"thread_per_core\""
                ),
            )),
        }
    }
}

pub(crate) struct MacroArgs {
    flavor: Option<(Flavor, Span)>,
    worker_threads: Option<(usize, Span)>,
    start_paused: Option<(bool, Span)>,
}

impl Parse for MacroArgs {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let metas = Punctuated::<Meta, Token![,]>::parse_terminated(input)?;
        let mut args = MacroArgs {
            flavor: None,
            worker_threads: None,
            start_paused: None,
        };

        for meta in metas {
            let nv = match meta {
                Meta::NameValue(nv) => nv,
                other => return Err(unsupported(&other)),
            };
            let span = syn::spanned::Spanned::span(&nv.value);
            if nv.path.is_ident("worker_threads") {
                let n = literal_usize(&nv.value)?;
                if n == 0 {
                    return Err(syn::Error::new(span, "`worker_threads` may not be 0"));
                }
                set_once(&mut args.worker_threads, (n, span), &nv.path)?;
            } else if nv.path.is_ident("flavor") {
                let flavor = Flavor::parse(&literal_str(&nv.value)?, span)?;
                set_once(&mut args.flavor, (flavor, span), &nv.path)?;
            } else if nv.path.is_ident("start_paused") {
                let paused = literal_bool(&nv.value)?;
                set_once(&mut args.start_paused, (paused, span), &nv.path)?;
            } else {
                return Err(unsupported(&nv.path));
            }
        }
        Ok(args)
    }
}

fn unsupported(tokens: &dyn quote::ToTokens) -> syn::Error {
    syn::Error::new_spanned(
        tokens,
        "unsupported argument -- expected `flavor = \"...\"`, `worker_threads = N` \
         or `start_paused = bool`",
    )
}

fn set_once<T>(slot: &mut Option<T>, value: T, path: &syn::Path) -> syn::Result<()> {
    if slot.is_some() {
        return Err(syn::Error::new_spanned(
            path,
            "argument given more than once",
        ));
    }
    *slot = Some(value);
    Ok(())
}

fn literal_str(expr: &Expr) -> syn::Result<String> {
    match expr {
        Expr::Lit(ExprLit {
            lit: Lit::Str(s), ..
        }) => Ok(s.value()),
        other => Err(syn::Error::new_spanned(other, "expected a string literal")),
    }
}

fn literal_bool(expr: &Expr) -> syn::Result<bool> {
    match expr {
        Expr::Lit(ExprLit {
            lit: Lit::Bool(b), ..
        }) => Ok(b.value),
        other => Err(syn::Error::new_spanned(other, "expected `true` or `false`")),
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

/// What [`MacroArgs::resolve`] settled on.
pub(crate) struct RuntimeConfig {
    flavor: Flavor,
    worker_threads: Option<usize>,
    start_paused: bool,
}

impl MacroArgs {
    /// Applies the defaults and tokio's cross-argument rules.
    ///
    /// With no `flavor`, both attributes build the multi-threaded
    /// runtime, as they always have here -- unlike tokio, whose
    /// `#[tokio::test]` defaults to `current_thread`. One exception
    /// keeps a migrated `#[tokio::test(start_paused = true)]` working:
    /// on `#[rusty_tokio::test]`, `start_paused = true` with no `flavor`
    /// selects `current_thread`, the only flavor a clock can be paused
    /// on (exactly the runtime tokio would have built for it).
    pub(crate) fn resolve(self, is_test: bool) -> syn::Result<RuntimeConfig> {
        let start_paused = self.start_paused.is_some_and(|(paused, _)| paused);
        let flavor = match self.flavor {
            Some((flavor, _)) => flavor,
            None if is_test && start_paused => Flavor::CurrentThread,
            None => Flavor::MultiThread,
        };

        if let (Flavor::CurrentThread, Some((_, span))) = (flavor, self.worker_threads) {
            return Err(syn::Error::new(
                span,
                "the `worker_threads` option requires the `multi_thread` runtime flavor. \
                 Use `flavor = \"multi_thread\"`",
            ));
        }
        if let Some((true, span)) = self.start_paused {
            if flavor != Flavor::CurrentThread {
                return Err(syn::Error::new(
                    span,
                    "the `start_paused` option requires the `current_thread` runtime flavor. \
                     Use `flavor = \"current_thread\"`",
                ));
            }
        }

        Ok(RuntimeConfig {
            flavor,
            worker_threads: self.worker_threads.map(|(n, _)| n),
            start_paused,
        })
    }
}

impl RuntimeConfig {
    /// The `rusty_tokio::Runtime` construction expression to block on
    /// the annotated function's body with.
    pub(crate) fn runtime_expr(&self) -> TokenStream {
        let builder = match self.flavor {
            Flavor::MultiThread => quote! { ::rusty_tokio::Builder::new_multi_thread() },
            Flavor::CurrentThread => quote! { ::rusty_tokio::Builder::new_current_thread() },
            Flavor::ThreadPerCore => quote! { ::rusty_tokio::Builder::new_thread_per_core() },
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

    /// The annotated function's body, wrapped so the clock is paused
    /// before any of it runs when `start_paused = true` -- the first
    /// statement inside `block_on`, which is the earliest point a
    /// runtime context exists for `time::pause` to find.
    pub(crate) fn body(&self, block: &syn::Block) -> TokenStream {
        if self.start_paused {
            quote! {
                async move {
                    ::rusty_tokio::time::pause();
                    #block
                }
            }
        } else {
            quote! { async move #block }
        }
    }
}
