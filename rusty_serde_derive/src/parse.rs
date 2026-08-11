//! A tiny hand-written parser over `proc_macro::TokenStream` - just enough
//! to recognize the shape of a struct/enum declaration (its name, and its
//! fields'/variants' names and arities). Field *types* are deliberately not
//! parsed: the generated impls never need to name a field's type, since
//! `Serialize`/`Deserialize` are called generically and Rust's own type
//! inference fills in the rest. That's what keeps this parser small enough
//! to write by hand instead of pulling in `syn`.

use std::iter::Peekable;
use proc_macro::{Delimiter, TokenStream, TokenTree};

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
    Struct { name: String, fields: Fields },
    Enum { name: String, variants: Vec<Variant> },
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
    reject_generics(tokens, &name)?;

    let fields = match tokens.peek() {
        Some(TokenTree::Group(g)) if g.delimiter() == Delimiter::Brace => {
            let group = take_group(tokens);
            parse_named_fields(group)?
        }
        Some(TokenTree::Group(g)) if g.delimiter() == Delimiter::Parenthesis => {
            let group = take_group(tokens);
            Fields::Unnamed(count_top_level_fields(group))
        }
        Some(TokenTree::Punct(p)) if p.as_char() == ';' => Fields::Unit,
        _ => {
            return Err(compile_error(&format!(
                "expected `{{ ... }}`, `( ... )`, or `;` after `struct {name}`"
            )))
        }
    };

    Ok(Data::Struct { name, fields })
}

fn parse_enum(tokens: &mut Tokens) -> Result<Data, TokenStream> {
    let name = expect_ident(tokens, "an enum name")?;
    reject_generics(tokens, &name)?;

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

    Ok(Data::Enum { name, variants })
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
        // Consume the type, i.e. everything up to the next top-level comma.
        while let Some(tt) = tokens.peek() {
            match tt {
                TokenTree::Punct(p) if p.as_char() == ',' => {
                    tokens.next();
                    break;
                }
                _ => {
                    tokens.next();
                }
            }
        }
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
        let mut saw_token = false;
        while let Some(tt) = tokens.peek() {
            saw_token = true;
            match tt {
                TokenTree::Punct(p) if p.as_char() == ',' => {
                    tokens.next();
                    break;
                }
                _ => {
                    tokens.next();
                }
            }
        }
        if saw_token {
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

/// Generic parameters aren't supported yet - surfaced as a clear compile
/// error rather than a confusing downstream failure.
fn reject_generics(tokens: &mut Tokens, name: &str) -> Result<(), TokenStream> {
    if let Some(TokenTree::Punct(p)) = tokens.peek() {
        if p.as_char() == '<' {
            return Err(compile_error(&format!(
                "rusty_serde_derive does not support generic parameters (on `{name}`)"
            )));
        }
    }
    Ok(())
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
