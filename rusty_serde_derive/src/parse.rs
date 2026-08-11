//! A tiny hand-written parser over `proc_macro::TokenStream` - just enough
//! to recognize the shape of a struct/enum declaration (its name, its
//! generic parameters, and its fields'/variants' names and arities). Field
//! *types* are deliberately not parsed: the generated impls never need to
//! name a field's type, since `Serialize`/`Deserialize` are called
//! generically and Rust's own type inference fills in the rest. That's what
//! keeps this parser small enough to write by hand instead of pulling in
//! `syn`.
//!
//! Generic parameters *are* parsed (just their declaration list, not the
//! fields that use them) so the derived `impl` can be generic too. Every
//! declared type parameter gets a blanket `Serialize`/`Deserialize` bound
//! tacked on - always sound (any field type built from `T` already needs
//! that bound to compile) if occasionally more conservative than a
//! hand-written impl would be (e.g. an unused `PhantomData<T>` field would
//! still force `T: Serialize`).

use proc_macro::{Delimiter, TokenStream, TokenTree};
use std::iter::Peekable;

pub enum Fields {
    Named(Vec<String>),
    Unnamed(usize),
    Unit,
}

pub struct Variant {
    pub name: String,
    pub fields: Fields,
}

pub enum Data {
    Struct {
        name: String,
        generics: Generics,
        fields: Fields,
    },
    Enum {
        name: String,
        generics: Generics,
        variants: Vec<Variant>,
    },
}

/// A type parameter's name plus any bounds it already declared (raw source
/// text, re-emitted as-is), e.g. `T` with bounds `Clone + AsRef<str>`.
pub struct TypeParam {
    pub name: String,
    pub bounds: String,
}

#[derive(Default)]
pub struct Generics {
    /// Raw lifetime declarations, e.g. `["'a", "'b: 'a"]`.
    pub lifetimes: Vec<String>,
    pub type_params: Vec<TypeParam>,
}

impl Generics {
    fn is_empty(&self) -> bool {
        self.lifetimes.is_empty() && self.type_params.is_empty()
    }

    /// The bare `<'a, T, U>` used to name the type at a use site (e.g.
    /// `Foo<'a, T, U>` in `impl ... for Foo<'a, T, U>`).
    pub fn use_site(&self) -> String {
        if self.is_empty() {
            return String::new();
        }
        let lifetime_names = self.lifetimes.iter().map(|l| {
            // A lifetime declaration may carry its own bound (`'a: 'b`);
            // only the name before `:` is valid at a use site.
            l.split(':').next().unwrap().trim().to_string()
        });
        let type_names = self.type_params.iter().map(|t| t.name.clone());
        let parts: Vec<String> = lifetime_names.chain(type_names).collect();
        format!("<{}>", parts.join(", "))
    }

    /// `name` plus [`Self::use_site`], e.g. `Foo<'a, T>` or just `Foo`.
    pub fn ty(&self, name: &str) -> String {
        format!("{name}{}", self.use_site())
    }

    /// The `impl<...>` declaration list: every lifetime as declared, then
    /// every type parameter with `bound_suffix` appended to its existing
    /// bounds. `extra_lifetime` (typically `'de`) is prepended first, since
    /// it must be declared before anything that uses it.
    pub fn impl_decl(&self, extra_lifetime: Option<&str>, bound_suffix: &str) -> String {
        let mut parts: Vec<String> = Vec::new();
        if let Some(l) = extra_lifetime {
            parts.push(l.to_string());
        }
        parts.extend(self.lifetimes.iter().cloned());
        for tp in &self.type_params {
            let bound = if tp.bounds.trim().is_empty() {
                bound_suffix.to_string()
            } else {
                format!("{} + {}", tp.bounds, bound_suffix)
            };
            parts.push(format!("{}: {}", tp.name, bound));
        }
        if parts.is_empty() {
            String::new()
        } else {
            format!("<{}>", parts.join(", "))
        }
    }
}

type Tokens = Peekable<proc_macro::token_stream::IntoIter>;

pub fn parse(input: TokenStream) -> Result<Data, TokenStream> {
    let mut tokens = input.into_iter().peekable();

    skip_outer_attributes(&mut tokens);
    skip_visibility(&mut tokens);

    let keyword = expect_ident(&mut tokens, "a `struct` or `enum` item")?;
    match keyword.as_str() {
        "struct" => parse_struct(&mut tokens),
        "enum" => parse_enum(&mut tokens),
        other => Err(compile_error(&format!(
            "rusty_serde_derive only supports structs and enums, found `{other}`"
        ))),
    }
}

fn parse_struct(tokens: &mut Tokens) -> Result<Data, TokenStream> {
    let name = expect_ident(tokens, "a struct name")?;
    let generics = parse_generics(tokens, &name)?;

    let fields = match tokens.peek() {
        Some(TokenTree::Group(g)) if g.delimiter() == Delimiter::Brace => {
            reject_where_clause(tokens, &name)?;
            let group = take_group(tokens);
            parse_named_fields(group)?
        }
        Some(TokenTree::Group(g)) if g.delimiter() == Delimiter::Parenthesis => {
            let group = take_group(tokens);
            reject_where_clause(tokens, &name)?;
            Fields::Unnamed(count_top_level_fields(group))
        }
        Some(TokenTree::Ident(id)) if id.to_string() == "where" => {
            return Err(compile_error(&format!(
                "rusty_serde_derive does not support `where` clauses (on `{name}`)"
            )));
        }
        Some(TokenTree::Punct(p)) if p.as_char() == ';' => Fields::Unit,
        _ => {
            return Err(compile_error(&format!(
                "expected `{{ ... }}`, `( ... )`, or `;` after `struct {name}`"
            )))
        }
    };

    Ok(Data::Struct {
        name,
        generics,
        fields,
    })
}

fn parse_enum(tokens: &mut Tokens) -> Result<Data, TokenStream> {
    let name = expect_ident(tokens, "an enum name")?;
    let generics = parse_generics(tokens, &name)?;
    reject_where_clause(tokens, &name)?;

    let body = match tokens.next() {
        Some(TokenTree::Group(g)) if g.delimiter() == Delimiter::Brace => g.stream(),
        _ => return Err(compile_error(&format!("expected `{{ ... }}` after `enum {name}`"))),
    };

    let mut variants = Vec::new();
    let mut variant_tokens = body.into_iter().peekable();
    while variant_tokens.peek().is_some() {
        skip_outer_attributes(&mut variant_tokens);
        if variant_tokens.peek().is_none() {
            break;
        }
        let variant_name = expect_ident(&mut variant_tokens, "a variant name")?;
        let fields = match variant_tokens.peek() {
            Some(TokenTree::Group(g)) if g.delimiter() == Delimiter::Brace => {
                let group = take_group(&mut variant_tokens);
                parse_named_fields(group)?
            }
            Some(TokenTree::Group(g)) if g.delimiter() == Delimiter::Parenthesis => {
                let group = take_group(&mut variant_tokens);
                Fields::Unnamed(count_top_level_fields(group))
            }
            _ => Fields::Unit,
        };

        // Skip an optional `= <discriminant>` and the trailing comma.
        while let Some(tt) = variant_tokens.peek() {
            match tt {
                TokenTree::Punct(p) if p.as_char() == ',' => {
                    variant_tokens.next();
                    break;
                }
                _ => {
                    variant_tokens.next();
                }
            }
        }

        variants.push(Variant {
            name: variant_name,
            fields,
        });
    }

    Ok(Data::Enum {
        name,
        generics,
        variants,
    })
}

/// Parses an optional `<...>` generic parameter list right after a type
/// name. Supports lifetime and type parameters (with bounds and/or a
/// default); const generics aren't recognized and are surfaced as a clear
/// error instead of silently mis-parsing.
fn parse_generics(tokens: &mut Tokens, owner: &str) -> Result<Generics, TokenStream> {
    match tokens.peek() {
        Some(TokenTree::Punct(p)) if p.as_char() == '<' => {}
        _ => return Ok(Generics::default()),
    }
    tokens.next();

    let mut depth = 1i32;
    let mut raw = Vec::new();
    loop {
        match tokens.next() {
            Some(TokenTree::Punct(p)) if p.as_char() == '<' => {
                depth += 1;
                raw.push(TokenTree::Punct(p));
            }
            Some(TokenTree::Punct(p)) if p.as_char() == '>' => {
                depth -= 1;
                if depth == 0 {
                    break;
                }
                raw.push(TokenTree::Punct(p));
            }
            Some(tt) => raw.push(tt),
            None => {
                return Err(compile_error(&format!(
                    "unterminated generic parameter list on `{owner}`"
                )))
            }
        }
    }

    let mut generics = Generics::default();
    for chunk in split_top_level(raw, |depth, tt| {
        depth == 0 && matches!(tt, TokenTree::Punct(p) if p.as_char() == ',')
    }) {
        if chunk.is_empty() {
            continue;
        }
        parse_one_generic_param(chunk, owner, &mut generics)?;
    }
    Ok(generics)
}

fn parse_one_generic_param(
    chunk: Vec<TokenTree>,
    owner: &str,
    out: &mut Generics,
) -> Result<(), TokenStream> {
    let mut it = chunk.into_iter().peekable();
    match it.peek() {
        Some(TokenTree::Punct(p)) if p.as_char() == '\'' => {
            // Lifetime parameter: `'a` or `'a: 'b + 'c`. Rebuilding a
            // TokenStream from the collected tokens (rather than
            // stringifying each one and joining with spaces) preserves the
            // original Joint spacing, so `'` stays glued to its name.
            let text = TokenStream::from_iter(it.collect::<Vec<_>>()).to_string();
            out.lifetimes.push(text);
            Ok(())
        }
        Some(TokenTree::Ident(id)) if id.to_string() == "const" => Err(compile_error(&format!(
            "rusty_serde_derive does not support const generic parameters (on `{owner}`)"
        ))),
        Some(TokenTree::Ident(_)) => {
            let name = match it.next() {
                Some(TokenTree::Ident(ident)) => ident.to_string(),
                _ => unreachable!("caller already peeked an Ident"),
            };
            // Skip attributes like `#[cfg(..)]` preceding a param - rare,
            // but be lenient rather than erroring.
            let mut bound_tokens = Vec::new();
            if let Some(TokenTree::Punct(p)) = it.peek() {
                if p.as_char() == ':' {
                    it.next();
                    let mut depth = 0i32;
                    while let Some(tt) = it.peek() {
                        match tt {
                            TokenTree::Punct(p) if p.as_char() == '<' => {
                                depth += 1;
                                bound_tokens.push(it.next().unwrap());
                            }
                            TokenTree::Punct(p) if p.as_char() == '>' => {
                                depth -= 1;
                                bound_tokens.push(it.next().unwrap());
                            }
                            TokenTree::Punct(p) if p.as_char() == '=' && depth == 0 => break,
                            _ => bound_tokens.push(it.next().unwrap()),
                        }
                    }
                }
            }
            // Anything left (a `= Default` type) is intentionally dropped:
            // defaults aren't valid inside an `impl<...>` header anyway.
            let bounds = TokenStream::from_iter(bound_tokens).to_string();
            out.type_params.push(TypeParam { name, bounds });
            Ok(())
        }
        Some(other) => Err(compile_error(&format!(
            "unrecognized generic parameter `{other}` on `{owner}`"
        ))),
        None => Ok(()),
    }
}

/// Splits `tokens` at top-level positions where `is_split` returns true,
/// tracking `<...>` nesting depth (Group tokens are already atomic, so only
/// angle brackets need manual depth tracking here).
fn split_top_level(
    tokens: Vec<TokenTree>,
    is_split: impl Fn(i32, &TokenTree) -> bool,
) -> Vec<Vec<TokenTree>> {
    let mut chunks = Vec::new();
    let mut current = Vec::new();
    let mut depth = 0i32;
    for tt in tokens {
        if is_split(depth, &tt) {
            chunks.push(std::mem::take(&mut current));
            continue;
        }
        match &tt {
            TokenTree::Punct(p) if p.as_char() == '<' => depth += 1,
            TokenTree::Punct(p) if p.as_char() == '>' => depth = (depth - 1).max(0),
            _ => {}
        }
        current.push(tt);
    }
    chunks.push(current);
    chunks
}

/// Consumes one comma-separated field/variant "entry" worth of tokens
/// (i.e. everything up to the next top-level comma, tracking `<...>`
/// nesting so e.g. `x: HashMap<String, i32>` isn't split at the comma
/// inside `HashMap<...>`), discarding it - callers only need field/variant
/// *names*, never their types.
fn skip_to_top_level_comma(tokens: &mut Tokens) {
    let mut depth = 0i32;
    while let Some(tt) = tokens.peek() {
        match tt {
            TokenTree::Punct(p) if p.as_char() == ',' && depth == 0 => {
                tokens.next();
                return;
            }
            TokenTree::Punct(p) if p.as_char() == '<' => {
                depth += 1;
                tokens.next();
            }
            TokenTree::Punct(p) if p.as_char() == '>' => {
                depth = (depth - 1).max(0);
                tokens.next();
            }
            _ => {
                tokens.next();
            }
        }
    }
}

/// `where` clauses aren't supported - detected right after generics (named
/// structs and enums) or right after the tuple-fields group (tuple
/// structs), since that's where Rust's grammar places them.
fn reject_where_clause(tokens: &mut Tokens, owner: &str) -> Result<(), TokenStream> {
    if let Some(TokenTree::Ident(id)) = tokens.peek() {
        if id.to_string() == "where" {
            return Err(compile_error(&format!(
                "rusty_serde_derive does not support `where` clauses (on `{owner}`)"
            )));
        }
    }
    Ok(())
}

/// Parses the inside of a `{ ... }` field list: `ident : <type tokens>`,
/// repeated and comma-separated. Attributes and `pub`/`pub(...)` visibility
/// ahead of a field name are skipped.
fn parse_named_fields(group: proc_macro::Group) -> Result<Fields, TokenStream> {
    let mut tokens = group.stream().into_iter().peekable();
    let mut names = Vec::new();

    loop {
        skip_outer_attributes(&mut tokens);
        skip_visibility(&mut tokens);
        if tokens.peek().is_none() {
            break;
        }
        let name = expect_ident(&mut tokens, "a field name")?;
        match tokens.next() {
            Some(TokenTree::Punct(p)) if p.as_char() == ':' => {}
            _ => return Err(compile_error(&format!("expected `:` after field `{name}`"))),
        }
        skip_to_top_level_comma(&mut tokens);
        names.push(name);
    }

    Ok(Fields::Named(names))
}

/// Counts comma-separated entries inside a `( ... )` tuple field/variant
/// list, ignoring nested delimiters and attributes/visibility per entry.
fn count_top_level_fields(group: proc_macro::Group) -> usize {
    let mut tokens = group.stream().into_iter().peekable();
    let mut count = 0;

    loop {
        skip_outer_attributes(&mut tokens);
        skip_visibility(&mut tokens);
        if tokens.peek().is_none() {
            break;
        }
        let before = tokens.peek().is_some();
        skip_to_top_level_comma(&mut tokens);
        if before {
            count += 1;
        }
    }

    count
}

fn skip_outer_attributes(tokens: &mut Tokens) {
    loop {
        match tokens.peek() {
            Some(TokenTree::Punct(p)) if p.as_char() == '#' => {
                tokens.next();
                if let Some(TokenTree::Group(g)) = tokens.peek() {
                    if g.delimiter() == Delimiter::Bracket {
                        tokens.next();
                        continue;
                    }
                }
            }
            _ => return,
        }
    }
}

fn skip_visibility(tokens: &mut Tokens) {
    if let Some(TokenTree::Ident(ident)) = tokens.peek() {
        if ident.to_string() == "pub" {
            tokens.next();
            if let Some(TokenTree::Group(g)) = tokens.peek() {
                if g.delimiter() == Delimiter::Parenthesis {
                    tokens.next();
                }
            }
        }
    }
}

fn take_group(tokens: &mut Tokens) -> proc_macro::Group {
    match tokens.next() {
        Some(TokenTree::Group(g)) => g,
        _ => unreachable!("caller already peeked a Group"),
    }
}

fn expect_ident(tokens: &mut Tokens, what: &str) -> Result<String, TokenStream> {
    match tokens.next() {
        Some(TokenTree::Ident(ident)) => Ok(ident.to_string()),
        Some(other) => Err(compile_error(&format!("expected {what}, found `{other}`"))),
        None => Err(compile_error(&format!("expected {what}, found end of input"))),
    }
}

pub fn compile_error(msg: &str) -> TokenStream {
    format!("compile_error!({msg:?});")
        .parse()
        .expect("compile_error! invocation is always valid Rust")
}
