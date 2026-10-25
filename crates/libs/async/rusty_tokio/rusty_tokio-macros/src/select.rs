//! The engine behind `rusty_tokio::select!`. Not called directly: the
//! main crate's `macro_rules! select` forwards to it as
//! `$crate::__select_internal! { ($crate) <tokens> }`, because a proc
//! macro has no `$crate` of its own -- it can't otherwise name the
//! runtime crate's support items reliably under a renamed dependency.
//! The leading parenthesized group carries that path through verbatim.
//!
//! ## Grammar (tokio's, unchanged)
//!
//! ```text
//! select! {
//!     [biased;]
//!     <pattern> = <async expression> [, if <precondition>] => <handler>,
//!     ...
//!     [else => <expression>]
//! }
//! ```
//!
//! A handler that is a `{ block }` may omit its trailing comma; an
//! expression handler needs one unless it is the last arm.
//!
//! ## Expansion shape, and why each piece is where it is
//!
//! ```text
//! {
//!     enum Out<T0, T1, ..> { B0(T0), B1(T1), .., Disabled }
//!     let mut disabled: u64 = 0;
//!     if !(<cond0>) { disabled |= 1 << 0; }      // every guard first
//!     let output = {
//!         let mut f0 = pin!(into_future(<fut0>)); // then every future
//!         ..
//!         poll_fn(|cx| { /* start-rotated loop over branches */ }).await
//!     };                                          // futures dropped here
//!     match output { Out::B0(<pat0>) => <handler0>, .., Out::Disabled => <else> }
//! }
//! ```
//!
//! - **Guards before futures**, so a precondition sees the world before
//!   any branch's future expression has run -- tokio's documented order.
//!   A disabled branch's future expression is still *evaluated* (tokio
//!   documents that too, and real code depends on it: an expression like
//!   `recv(&maybe_socket), if maybe_socket.is_some()` must type-check and
//!   build whether or not the guard holds); it is just never polled.
//! - **Handlers run outside the `poll_fn` closure**, after the inner
//!   block has ended. That is what lets a handler `.await`, `break`,
//!   `continue`, `return` or `?` in the caller's own context, and what
//!   lets it borrow anything the (now dropped) futures borrowed.
//! - **A completed future whose output doesn't match its pattern
//!   disables that branch** and polling moves on to the others. The
//!   check inside the closure matches `&out` against the pattern with
//!   `mut`/`ref` binding modifiers stripped (they would be errors or
//!   moves through a reference); the real, owning match against the
//!   original pattern happens once, in the handler `match`.
//! - **Fairness**: unless `biased;`, each poll starts at a branch picked
//!   by the runtime crate's thread-local PRNG and wraps around, so a
//!   permanently ready early branch can't starve a later one.
//! - **All branches disabled** resolves to `Out::Disabled`, which runs
//!   `else` or panics with tokio's exact message.
//! - The disabled set is a `u64` bit mask, so at most 64 branches --
//!   tokio's own limit, for the same reason.

use proc_macro2::{Span, TokenStream};
use quote::{format_ident, quote};
use syn::parse::{Parse, ParseStream};
use syn::{braced, token, Expr, Pat, Token};

/// The most branches the `u64` disabled mask can track (tokio's limit).
const MAX_BRANCHES: usize = 64;

struct Branch {
    pat: Pat,
    fut: Expr,
    cond: Option<Expr>,
    handler: Expr,
}

struct Select {
    crate_path: TokenStream,
    biased: bool,
    branches: Vec<Branch>,
    else_handler: Option<Expr>,
}

impl Parse for Select {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let path_group;
        syn::parenthesized!(path_group in input);
        let crate_path: TokenStream = path_group.parse()?;

        let biased = input.peek(syn::Ident) && input.peek2(Token![;]) && {
            let fork = input.fork();
            let ident: syn::Ident = fork.parse()?;
            ident == "biased"
        };
        if biased {
            input.parse::<syn::Ident>()?;
            input.parse::<Token![;]>()?;
        }

        let mut branches = Vec::new();
        let mut else_handler = None;
        while !input.is_empty() {
            if input.peek(Token![else]) {
                let else_token: Token![else] = input.parse()?;
                if else_handler.is_some() {
                    return Err(syn::Error::new_spanned(
                        else_token,
                        "`select!` accepts at most one `else` branch",
                    ));
                }
                input.parse::<Token![=>]>()?;
                else_handler = Some(parse_handler(input)?);
                continue;
            }
            branches.push(parse_branch(input)?);
        }

        Ok(Select {
            crate_path,
            biased,
            branches,
            else_handler,
        })
    }
}

fn parse_branch(input: ParseStream) -> syn::Result<Branch> {
    let pat = Pat::parse_multi_with_leading_vert(input)?;
    input.parse::<Token![=]>()?;
    let fut: Expr = input.parse()?;
    let cond = if input.peek(Token![,]) && input.peek2(Token![if]) {
        input.parse::<Token![,]>()?;
        input.parse::<Token![if]>()?;
        Some(input.parse::<Expr>()?)
    } else {
        None
    };
    input.parse::<Token![=>]>()?;
    let handler = parse_handler(input)?;
    Ok(Branch {
        pat,
        fut,
        cond,
        handler,
    })
}

/// A `{ block }` handler ends at its closing brace (trailing comma
/// optional); any other expression must be followed by a comma unless
/// it ends the input -- tokio's rule, and the only unambiguous one,
/// since an expression handler could otherwise run on into the next
/// branch's pattern.
fn parse_handler(input: ParseStream) -> syn::Result<Expr> {
    if input.peek(token::Brace) {
        let content;
        let brace_token = braced!(content in input);
        let stmts = content.call(syn::Block::parse_within)?;
        if input.peek(Token![,]) {
            input.parse::<Token![,]>()?;
        }
        return Ok(Expr::Block(syn::ExprBlock {
            attrs: Vec::new(),
            label: None,
            block: syn::Block { brace_token, stmts },
        }));
    }
    let handler: Expr = input.parse()?;
    if !input.is_empty() {
        input.parse::<Token![,]>()?;
    }
    Ok(handler)
}

/// Strips `ref`/`ref mut`/`mut` from every binding so the pattern can be
/// matched against `&out` (default binding modes then bind by reference)
/// purely as a "does it match" test, without moving out of `out`.
fn clean_pattern(pat: &mut Pat) {
    match pat {
        Pat::Ident(ident) => {
            ident.by_ref = None;
            ident.mutability = None;
            if let Some((_, sub)) = ident.subpat.as_mut() {
                clean_pattern(sub);
            }
        }
        Pat::Or(or) => or.cases.iter_mut().for_each(clean_pattern),
        Pat::Paren(paren) => clean_pattern(&mut paren.pat),
        Pat::Reference(reference) => {
            reference.mutability = None;
            clean_pattern(&mut reference.pat);
        }
        Pat::Slice(slice) => slice.elems.iter_mut().for_each(clean_pattern),
        Pat::Struct(strukt) => strukt
            .fields
            .iter_mut()
            .for_each(|field| clean_pattern(&mut field.pat)),
        Pat::Tuple(tuple) => tuple.elems.iter_mut().for_each(clean_pattern),
        Pat::TupleStruct(tuple) => tuple.elems.iter_mut().for_each(clean_pattern),
        Pat::Type(typed) => clean_pattern(&mut typed.pat),
        _ => {}
    }
}

pub(crate) fn expand(input: TokenStream) -> TokenStream {
    match syn::parse2::<Select>(input) {
        Ok(select) => expand_select(select),
        Err(err) => err.to_compile_error(),
    }
}

fn expand_select(select: Select) -> TokenStream {
    let Select {
        crate_path,
        biased,
        branches,
        else_handler,
    } = select;

    if branches.is_empty() {
        return match else_handler {
            Some(else_handler) => quote! {{ #else_handler }},
            None => syn::Error::new(Span::call_site(), "`select!` requires at least one branch")
                .to_compile_error(),
        };
    }
    if branches.len() > MAX_BRANCHES {
        return syn::Error::new(
            Span::call_site(),
            format!("`select!` supports at most {MAX_BRANCHES} branches"),
        )
        .to_compile_error();
    }

    // Internal locals get mixed-site hygiene so a handler or guard can
    // neither see nor shadow them.
    let hyg = Span::mixed_site();
    let out_enum = syn::Ident::new("__RustyTokioSelectOut", hyg);
    let disabled = syn::Ident::new("__disabled", hyg);
    let output = syn::Ident::new("__output", hyg);
    let out = syn::Ident::new("__out", hyg);
    let cx = syn::Ident::new("__cx", hyg);
    let is_pending = syn::Ident::new("__is_pending", hyg);
    let branch_ix = syn::Ident::new("__branch", hyg);
    let poll = syn::Ident::new("__poll", hyg);

    let count = branches.len();
    let count_u32 = count as u32;
    let type_params: Vec<_> = (0..count).map(|i| format_ident!("T{}", i)).collect();
    let variants: Vec<_> = (0..count).map(|i| format_ident!("B{}", i)).collect();
    let fut_names: Vec<_> = (0..count)
        .map(|i| syn::Ident::new(&format!("__fut{i}"), hyg))
        .collect();

    let guards = branches.iter().enumerate().filter_map(|(i, branch)| {
        let cond = branch.cond.as_ref()?;
        let bit = i as u32;
        Some(quote! {
            if !(#cond) {
                #disabled |= 1u64 << #bit;
            }
        })
    });

    let pins = branches.iter().zip(&fut_names).map(|(branch, name)| {
        let fut = &branch.fut;
        quote! {
            let mut #name = ::core::pin::pin!(::core::future::IntoFuture::into_future(#fut));
        }
    });

    let poll_arms = branches.iter().enumerate().map(|(i, branch)| {
        let index = i as u32;
        let name = &fut_names[i];
        let variant = &variants[i];
        let mut check_pat = branch.pat.clone();
        clean_pattern(&mut check_pat);
        quote! {
            #index => {
                let __mask: u64 = 1u64 << #index;
                if #disabled & __mask != 0 {
                    continue;
                }
                let #out = match ::core::future::Future::poll(#name.as_mut(), #cx) {
                    ::core::task::Poll::Ready(#out) => #out,
                    ::core::task::Poll::Pending => {
                        #is_pending = true;
                        continue;
                    }
                };
                // A completed future is never polled again, matched or not.
                #disabled |= __mask;
                match &#out {
                    #[allow(unused_variables, unused_mut)]
                    #check_pat => {}
                    #[allow(unreachable_patterns)]
                    _ => continue,
                }
                return ::core::task::Poll::Ready(#out_enum::#variant(#out));
            }
        }
    });

    let branch_order = if biased {
        quote! { let #branch_ix = __i; }
    } else {
        quote! {
            let #branch_ix = (__start + __i) % #count_u32;
        }
    };
    let start = if biased {
        quote! {}
    } else {
        quote! { let __start: u32 = #crate_path::__macro_support::thread_rng_n(#count_u32); }
    };

    let handler_arms = branches.iter().zip(&variants).map(|(branch, variant)| {
        let pat = &branch.pat;
        let handler = &branch.handler;
        quote! { #out_enum::#variant(#pat) => #handler, }
    });

    let else_arm = match else_handler {
        Some(else_handler) => quote! { #out_enum::Disabled => #else_handler, },
        None => quote! {
            #out_enum::Disabled => ::core::panic!(
                "all branches are disabled and there is no else branch"
            ),
        },
    };

    quote! {{
        #[allow(dead_code)]
        enum #out_enum<#(#type_params),*> {
            #(#variants(#type_params),)*
            Disabled,
        }

        #[allow(unused_mut)]
        let mut #disabled: u64 = 0;
        #(#guards)*

        let #output = {
            #(#pins)*
            // The rotation loop trips lints on the caller's side for small
            // branch counts (`% 1`, a one-iteration loop); they are
            // artifacts of the generated code, not the caller's.
            #[allow(clippy::all, clippy::pedantic)]
            let #poll = |#cx: &mut ::core::task::Context<'_>| {
                let mut #is_pending = false;
                #start
                for __i in 0..#count_u32 {
                    #branch_order
                    match #branch_ix {
                        #(#poll_arms)*
                        _ => {}
                    }
                }
                if #is_pending {
                    ::core::task::Poll::Pending
                } else {
                    ::core::task::Poll::Ready(#out_enum::Disabled)
                }
            };
            ::core::future::poll_fn(#poll).await
        };

        match #output {
            #(#handler_arms)*
            #else_arm
            #[allow(unreachable_patterns)]
            _ => ::core::unreachable!("`select!` output did not match the pattern it was checked against"),
        }
    }}
}
