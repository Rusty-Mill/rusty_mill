//! `stream!` and `try_stream!` for `rusty_stream`.
//!
//! Both rewrite `yield <expr>;` into `<yielder>.send(<expr>).await;` and wrap
//! the body in an `async move` block handed to `rusty_stream::__private`.
//! The rewrite is token-level, with no `syn`: a `yield` is every token up to
//! the next `;` at the same nesting level.
//!
//! `try_stream!`'s body returns `Result<(), E>`, so `?` works as written: an
//! error is yielded as the stream's last item. A bare `return;` is rewritten
//! to `return Ok(())`, except inside closure and `async` block bodies.
//! A bare `return;` inside a nested `fn` item is not supported.

use proc_macro::{Delimiter, Group, Ident, Punct, Spacing, Span, TokenStream, TokenTree};

/// An async stream from a block that `yield`s items.
///
/// ```ignore
/// let s = rusty_stream::stream! {
///     for i in 0..3 {
///         yield i;
///     }
/// };
/// ```
#[proc_macro]
pub fn stream(input: TokenStream) -> TokenStream {
    expand(input, false)
}

/// An async stream of `Result`s. `yield v` yields `Ok(v)`; `?` yields the
/// error and ends the stream.
#[proc_macro]
pub fn try_stream(input: TokenStream) -> TokenStream {
    expand(input, true)
}

fn expand(input: TokenStream, is_try: bool) -> TokenStream {
    let yielder = Ident::new("__rusty_stream_yielder", Span::mixed_site());
    let mut block = rewrite(input, &yielder, is_try, is_try);
    if is_try {
        block.extend(tokens(
            "; #[allow(unreachable_code)] ::core::result::Result::Ok(())",
        ));
    }
    // move |yielder| async move { block }
    let mut closure = tokens("move");
    closure.extend([
        TokenTree::Punct(Punct::new('|', Spacing::Alone)),
        TokenTree::Ident(yielder),
        TokenTree::Punct(Punct::new('|', Spacing::Alone)),
    ]);
    closure.extend(tokens("async move"));
    closure.extend([TokenTree::Group(Group::new(Delimiter::Brace, block))]);

    let mut call = tokens(if is_try {
        "::rusty_stream::__private::try_stream"
    } else {
        "::rusty_stream::__private::stream"
    });
    call.extend([TokenTree::Group(Group::new(
        Delimiter::Parenthesis,
        closure,
    ))]);
    call
}

/// Rewrite `yield` everywhere, and bare `return;` when `fix_return` holds.
fn rewrite(input: TokenStream, yielder: &Ident, is_try: bool, fix_return: bool) -> TokenStream {
    let trees: Vec<TokenTree> = input.into_iter().collect();
    let mut out = Vec::with_capacity(trees.len());
    let mut i = 0;
    while i < trees.len() {
        match &trees[i] {
            TokenTree::Ident(id) if id.to_string() == "yield" => {
                let end = statement_end(&trees, i + 1);
                let expr: TokenStream = trees[i + 1..end].iter().cloned().collect();
                let expr = rewrite(expr, yielder, is_try, fix_return);
                out.extend(send_call(yielder, expr, is_try));
                i = end;
            }
            TokenTree::Ident(id) if fix_return && is_bare_return(id, trees.get(i + 1)) => {
                out.extend(tokens("return ::core::result::Result::Ok(())"));
                i += 1;
            }
            // A macro invocation's arguments (`name!(..)`, `name! { .. }`)
            // belong to that macro: a nested `stream!` owns its own yields.
            TokenTree::Group(_) if follows_bang(&out) => {
                out.push(trees[i].clone());
                i += 1;
            }
            TokenTree::Group(g) => {
                let nested_fn = fix_return && opens_closure_or_async(&out);
                let inner = rewrite(g.stream(), yielder, is_try, fix_return && !nested_fn);
                let mut group = Group::new(g.delimiter(), inner);
                group.set_span(g.span());
                out.push(TokenTree::Group(group));
                i += 1;
            }
            other => {
                out.push(other.clone());
                i += 1;
            }
        }
    }
    out.into_iter().collect()
}

/// The index of the `;` ending the statement that starts at `from`, or the end.
fn statement_end(tokens: &[TokenTree], from: usize) -> usize {
    tokens[from..]
        .iter()
        .position(|t| matches!(t, TokenTree::Punct(p) if p.as_char() == ';'))
        .map_or(tokens.len(), |offset| from + offset)
}

/// `return` with no value: followed by `;`, `,` or the end of its block.
fn is_bare_return(id: &Ident, next: Option<&TokenTree>) -> bool {
    id.to_string() == "return"
        && match next {
            None => true,
            Some(TokenTree::Punct(p)) => matches!(p.as_char(), ';' | ','),
            Some(_) => false,
        }
}

/// Whether the last token emitted is `!`, so a following group is a macro's
/// arguments (or a negated expression, which holds no statements).
fn follows_bang(emitted: &[TokenTree]) -> bool {
    matches!(emitted.last(), Some(TokenTree::Punct(p)) if p.as_char() == '!')
}

/// Whether the tokens just emitted end a closure head (`|..|`) or `async`/`move`.
fn opens_closure_or_async(emitted: &[TokenTree]) -> bool {
    match emitted.last() {
        Some(TokenTree::Punct(p)) => p.as_char() == '|',
        Some(TokenTree::Ident(id)) => matches!(id.to_string().as_str(), "async" | "move"),
        _ => false,
    }
}

/// `<yielder>.send(<expr>).await`, wrapping `expr` in `Ok(..)` for `try_stream!`.
fn send_call(yielder: &Ident, expr: TokenStream, is_try: bool) -> TokenStream {
    let value = if expr.is_empty() { tokens("()") } else { expr };
    let value = if is_try {
        let mut ok: TokenStream = tokens("::core::result::Result::Ok");
        ok.extend([TokenTree::Group(Group::new(Delimiter::Parenthesis, value))]);
        ok
    } else {
        value
    };
    let mut out = TokenStream::from(TokenTree::Ident(yielder.clone()));
    out.extend(tokens(".send"));
    out.extend([TokenTree::Group(Group::new(Delimiter::Parenthesis, value))]);
    out.extend(tokens(".await"));
    out
}

/// Tokens from a fixed source string. Every caller passes a literal that
/// lexes, so the parse cannot fail.
fn tokens(src: &'static str) -> TokenStream {
    src.parse().expect("fixed token source lexes")
}
