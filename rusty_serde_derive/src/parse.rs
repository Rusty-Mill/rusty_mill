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
    Named(Vec<NamedField>),
    Unnamed(usize),
    Unit,
}

/// A `#[rusty_serde(...)]` attribute, recognized on named struct/variant
/// fields, on enum variants (`rename` only), and on the struct/enum item
/// itself (`rename_all`, and - enums only - `tag`). Any other attribute
/// (`#[serde(...)]`, `#[doc = "..."]`, `#[cfg(...)]`, ...) is left alone -
/// only our own namespace is inspected.
#[derive(Default, Clone)]
pub struct Attrs {
    /// `#[rusty_serde(rename = "x")]` (both directions) or
    /// `#[rusty_serde(rename(serialize = "..", deserialize = ".."))]`
    /// (either or both, independently).
    pub rename: RenameAttr,
    /// `#[rusty_serde(default)]` (falls back to `Default::default()`) or
    /// `#[rusty_serde(default = "path")]` (falls back to calling an
    /// arbitrary zero-arg path) when the field is missing on deserialize.
    pub default: Option<DefaultAttr>,
    /// `#[rusty_serde(skip)]` - shorthand for setting both
    /// `skip_serializing` and `skip_deserializing`.
    pub skip: bool,
    /// `#[rusty_serde(skip_serializing)]` - the field is never serialized
    /// (as if it didn't exist on the wire), but is still read on
    /// deserialize if present.
    pub skip_serializing: bool,
    /// `#[rusty_serde(skip_deserializing)]` - the field is never read from
    /// the wire (always falls back to `default`/`Default::default()`,
    /// exactly like `skip`'s read side), but is still serialized.
    pub skip_deserializing: bool,
    /// A raw Rust path (e.g. `"Option::is_none"`), called as `path(&self.field)`
    /// during serialization; the field is omitted from the output (and from
    /// the struct's computed length) whenever it returns `true`.
    pub skip_serializing_if: Option<String>,
    /// Merges this field's own serialized shape (must be a map/struct) into
    /// the parent's, instead of nesting it under its own key.
    pub flatten: bool,
    /// `#[rusty_serde(alias = "...")]`, repeatable: extra wire names that
    /// deserialize accepts for this field in addition to its primary
    /// `wire_name()`. Serialize is unaffected - always uses the primary name.
    pub aliases: Vec<String>,
    /// Container-only: a case-conversion style (`"camelCase"`, ...) applied
    /// to every named field/variant that didn't set its own `rename`.
    pub rename_all: Option<String>,
    /// Container-only, enums only: switches from external tagging
    /// (`{"Variant": ...}`) to internal tagging (`{"<tag>": "Variant",
    /// ...fields}`).
    pub tag: Option<String>,
    /// Container-only, enums only: no tag/wrapper at all - try each
    /// variant's own shape in turn until one deserializes successfully.
    pub untagged: bool,
    /// Container-only: errors on deserialize instead of silently ignoring
    /// an unrecognized field/key. Mutually exclusive with a `flatten`
    /// field, which needs somewhere to put unrecognized keys.
    pub deny_unknown_fields: bool,
    /// Variant-only: this (unit) variant is the deserialize catch-all for
    /// a tag value that doesn't match any other variant, instead of that
    /// being a hard error. At most one variant per enum.
    pub other: bool,
    /// Container-only, structs only: serialize/deserialize exactly as the
    /// single field would on its own, with no wrapping - like a
    /// tuple-struct-of-one's existing behavior, but opt-in for a *named*
    /// single-field struct too. Only valid on a struct with exactly one
    /// field.
    pub transparent: bool,
    /// Container-only: `#[rusty_serde(bound = "T: Trait")]` - replaces the
    /// derive's auto-generated `T: Serialize`/`T: Deserialize` where-clause
    /// entirely (both directions) with this raw predicate text.
    pub bound: Option<String>,
}

/// Where a field's `#[rusty_serde(default...)]` value comes from.
#[derive(Clone)]
pub enum DefaultAttr {
    /// Bare `#[rusty_serde(default)]` - `Default::default()`.
    Trait,
    /// `#[rusty_serde(default = "path")]` - `path()`.
    Path(String),
}

/// A field/variant's wire name, independently per direction. Bare
/// `rename = "x"` sets both; `rename(serialize = "..")` and/or
/// `rename(deserialize = "..")` set either independently, leaving the
/// other to fall back to the field/variant's own Rust name.
#[derive(Clone, Default)]
pub struct RenameAttr {
    pub serialize: Option<String>,
    pub deserialize: Option<String>,
}

impl RenameAttr {
    fn is_unset(&self) -> bool {
        self.serialize.is_none() && self.deserialize.is_none()
    }
}

impl Attrs {
    /// The field is never written to the wire - `skip` or `skip_serializing`.
    pub fn skips_serializing(&self) -> bool {
        self.skip || self.skip_serializing
    }

    /// The field is never read from the wire (always defaulted) - `skip`
    /// or `skip_deserializing`.
    pub fn skips_deserializing(&self) -> bool {
        self.skip || self.skip_deserializing
    }

    fn is_default(&self) -> bool {
        self.rename.is_unset()
            && self.default.is_none()
            && !self.skip
            && !self.skip_serializing
            && !self.skip_deserializing
            && self.skip_serializing_if.is_none()
            && !self.flatten
            && self.aliases.is_empty()
            && self.rename_all.is_none()
            && self.tag.is_none()
            && !self.untagged
            && !self.deny_unknown_fields
            && !self.other
            && !self.transparent
            && self.bound.is_none()
    }
}

pub struct NamedField {
    pub name: String,
    pub attrs: Attrs,
}

impl NamedField {
    /// The key to serialize under: `rename`'s serialize-direction name if
    /// given, else the field's own Rust name.
    pub fn wire_name(&self) -> &str {
        self.attrs.rename.serialize.as_deref().unwrap_or(&self.name)
    }

    /// The key to match on deserialize: `rename`'s deserialize-direction
    /// name if given, else the field's own Rust name. Independent of
    /// `wire_name()` - `rename(serialize = "..")` alone doesn't affect
    /// this, and vice versa.
    pub fn de_wire_name(&self) -> &str {
        self.attrs
            .rename
            .deserialize
            .as_deref()
            .unwrap_or(&self.name)
    }
}

pub struct Variant {
    pub name: String,
    pub attrs: Attrs,
    pub fields: Fields,
}

impl Variant {
    pub fn wire_name(&self) -> &str {
        self.attrs.rename.serialize.as_deref().unwrap_or(&self.name)
    }

    pub fn de_wire_name(&self) -> &str {
        self.attrs
            .rename
            .deserialize
            .as_deref()
            .unwrap_or(&self.name)
    }
}

pub enum Data {
    Struct {
        name: String,
        generics: Generics,
        fields: Fields,
        /// From a container-level `#[rusty_serde(deny_unknown_fields)]`.
        deny_unknown_fields: bool,
        /// From a container-level `#[rusty_serde(transparent)]` - always
        /// `false` unless `fields` is `Fields::Named` with exactly one
        /// field (any other shape is a compile error at parse time).
        transparent: bool,
    },
    Enum {
        name: String,
        generics: Generics,
        variants: Vec<Variant>,
        /// From a container-level `#[rusty_serde(tag = "...")]`.
        tag: Option<String>,
        /// From a container-level `#[rusty_serde(untagged)]`.
        untagged: bool,
        /// From a container-level `#[rusty_serde(deny_unknown_fields)]`.
        deny_unknown_fields: bool,
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
    /// Raw predicate text from a user-written `where` clause (empty if
    /// none), e.g. `"T: MyTrait, U: OtherTrait"`.
    pub extra_where: String,
    /// From a container-level `#[rusty_serde(bound = "...")]`: replaces
    /// the entire auto-generated where-clause (both the per-type-param
    /// `Serialize`/`Deserialize` bounds and `extra_where` above) with this
    /// raw predicate text, verbatim.
    pub bound_override: Option<String>,
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

    /// The `impl<...>` declaration list: every lifetime as declared
    /// (bounds and all - `'a: 'b` is valid directly in this position), then
    /// every type parameter's *bare name* (no bounds - those go in
    /// [`Self::where_clause`] instead, since a user's own `where` clause has
    /// to be merged in after the `for Type` part, which the `<...>` list
    /// comes before). `extra_lifetime` (typically `'de`) is prepended
    /// first, since it must be declared before anything that uses it - and,
    /// when the type has lifetimes of its own, bounded by all of them
    /// (`'de: 'a`) so a borrowed field (`&'a str`, `Cow<'a, str>`, ...)
    /// can actually be built from data a `Deserializer<'de>` hands back;
    /// callers that don't need `'de` (Serialize) just pass `None` and get
    /// the type's own lifetimes back unbounded.
    pub fn impl_decl(&self, extra_lifetime: Option<&str>) -> String {
        let mut parts: Vec<String> = Vec::new();
        if let Some(l) = extra_lifetime {
            if self.lifetimes.is_empty() {
                parts.push(l.to_string());
            } else {
                let bare_names = self
                    .lifetimes
                    .iter()
                    .map(|decl| decl.split(':').next().unwrap().trim().to_string());
                let bounds = bare_names.collect::<Vec<_>>().join(" + ");
                parts.push(format!("{l}: {bounds}"));
            }
        }
        parts.extend(self.lifetimes.iter().cloned());
        parts.extend(self.type_params.iter().map(|t| t.name.clone()));
        if parts.is_empty() {
            String::new()
        } else {
            format!("<{}>", parts.join(", "))
        }
    }

    /// The trailing `where T: Bound, ...` clause (empty string if there's
    /// nothing to say), combining every type parameter's own bounds plus
    /// `bound_suffix`, and the user's own `where` clause predicates (if
    /// any) verbatim. Include a leading space when non-empty, so it can be
    /// spliced directly after `for Type` and before the opening `{`.
    pub fn where_clause(&self, bound_suffix: &str) -> String {
        if let Some(bound) = &self.bound_override {
            return if bound.trim().is_empty() {
                // `bound = ""` - no bound at all, e.g. for a `PhantomData<T>`
                // field the macro can't see doesn't actually need `T` to be
                // (De)Serialize.
                String::new()
            } else {
                format!(" where {}", bound.trim())
            };
        }
        let mut preds: Vec<String> = self
            .type_params
            .iter()
            .map(|tp| {
                if tp.bounds.trim().is_empty() {
                    format!("{}: {}", tp.name, bound_suffix)
                } else {
                    format!("{}: {} + {}", tp.name, tp.bounds, bound_suffix)
                }
            })
            .collect();
        if !self.extra_where.trim().is_empty() {
            preds.push(self.extra_where.trim().to_string());
        }
        if preds.is_empty() {
            String::new()
        } else {
            format!(" where {}", preds.join(", "))
        }
    }
}

type Tokens = Peekable<proc_macro::token_stream::IntoIter>;

pub fn parse(input: TokenStream) -> Result<Data, TokenStream> {
    let mut tokens = input.into_iter().peekable();

    let container_attrs = parse_attrs(&mut tokens, "container")?;
    skip_visibility(&mut tokens);

    let keyword = expect_ident(&mut tokens, "a `struct` or `enum` item")?;
    match keyword.as_str() {
        "struct" => parse_struct(&mut tokens, container_attrs),
        "enum" => parse_enum(&mut tokens, container_attrs),
        other => Err(compile_error(&format!(
            "rusty_serde_derive only supports structs and enums, found `{other}`"
        ))),
    }
}

fn parse_struct(tokens: &mut Tokens, container_attrs: Attrs) -> Result<Data, TokenStream> {
    let name = expect_ident(tokens, "a struct name")?;
    if container_attrs.tag.is_some() {
        return Err(compile_error(&format!(
            "`tag` is only supported on enums (on `{name}`)"
        )));
    }
    if container_attrs.untagged {
        return Err(compile_error(&format!(
            "`untagged` is only supported on enums (on `{name}`)"
        )));
    }
    let mut generics = parse_generics(tokens, &name)?;
    generics.bound_override = container_attrs.bound.clone();

    // A tuple struct's `where` clause (if any) comes *after* the `(...)`
    // fields, unlike every other case (named struct/unit struct/enum),
    // where it comes right after the generics - so the tuple-fields arm
    // handles its own `where` parsing, positioned after the `take_group`.
    let mut fields = match tokens.peek() {
        Some(TokenTree::Group(g)) if g.delimiter() == Delimiter::Parenthesis => {
            let group = take_group(tokens);
            generics.extra_where = parse_where_clause(tokens)?;
            Fields::Unnamed(count_top_level_fields(group)?)
        }
        _ => {
            generics.extra_where = parse_where_clause(tokens)?;
            match tokens.peek() {
                Some(TokenTree::Group(g)) if g.delimiter() == Delimiter::Brace => {
                    let group = take_group(tokens);
                    parse_named_fields(group, "struct")?
                }
                Some(TokenTree::Punct(p)) if p.as_char() == ';' => Fields::Unit,
                _ => {
                    return Err(compile_error(&format!(
                        "expected `{{ ... }}`, `( ... )`, or `;` after `struct {name}`"
                    )))
                }
            }
        }
    };

    if let (Fields::Named(named), Some(style)) = (&mut fields, &container_attrs.rename_all) {
        apply_rename_all_fields(named, style);
    }

    if container_attrs.deny_unknown_fields {
        if let Fields::Named(named) = &fields {
            if named.iter().any(|f| f.attrs.flatten) {
                return Err(compile_error(&format!(
                    "`deny_unknown_fields` and `flatten` can't both be set (on `{name}`) - a \
                     flattened field needs somewhere to put keys that don't match another field"
                )));
            }
        }
    }

    if container_attrs.transparent {
        match &fields {
            Fields::Named(named) if named.len() == 1 => {}
            _ => {
                return Err(compile_error(&format!(
                    "`#[rusty_serde(transparent)]` is only supported on a struct with exactly \
                     one named field (on `{name}`) - a tuple struct with one field is already \
                     transparent without the attribute"
                )))
            }
        }
    }

    Ok(Data::Struct {
        name,
        generics,
        fields,
        deny_unknown_fields: container_attrs.deny_unknown_fields,
        transparent: container_attrs.transparent,
    })
}

fn parse_enum(tokens: &mut Tokens, container_attrs: Attrs) -> Result<Data, TokenStream> {
    let name = expect_ident(tokens, "an enum name")?;
    if container_attrs.tag.is_some() && container_attrs.untagged {
        return Err(compile_error(&format!(
            "`tag` and `untagged` can't both be set (on `{name}`)"
        )));
    }
    if container_attrs.transparent {
        return Err(compile_error(&format!(
            "`transparent` is only supported on structs, not enums (on `{name}`)"
        )));
    }
    let mut generics = parse_generics(tokens, &name)?;
    generics.bound_override = container_attrs.bound.clone();
    generics.extra_where = parse_where_clause(tokens)?;

    let body = match tokens.next() {
        Some(TokenTree::Group(g)) if g.delimiter() == Delimiter::Brace => g.stream(),
        _ => {
            return Err(compile_error(&format!(
                "expected `{{ ... }}` after `enum {name}`"
            )))
        }
    };

    let mut variants = Vec::new();
    let mut variant_tokens = body.into_iter().peekable();
    while variant_tokens.peek().is_some() {
        let attrs = parse_attrs(&mut variant_tokens, "variant")?;
        if variant_tokens.peek().is_none() {
            break;
        }
        let variant_name = expect_ident(&mut variant_tokens, "a variant name")?;
        let fields = match variant_tokens.peek() {
            Some(TokenTree::Group(g)) if g.delimiter() == Delimiter::Brace => {
                let group = take_group(&mut variant_tokens);
                parse_named_fields(group, "variant")?
            }
            Some(TokenTree::Group(g)) if g.delimiter() == Delimiter::Parenthesis => {
                let group = take_group(&mut variant_tokens);
                Fields::Unnamed(count_top_level_fields(group)?)
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
            attrs,
            fields,
        });
    }

    if let Some(style) = &container_attrs.rename_all {
        apply_rename_all_variants(&mut variants, style);
    }

    if let Some(tag) = &container_attrs.tag {
        for v in &variants {
            if matches!(v.fields, Fields::Unnamed(n) if n > 0) {
                return Err(compile_error(&format!(
                    "rusty_serde_derive's internally-tagged enums (`tag = \"{tag}\"`) only \
                     support unit and named-field variants, not tuple variant `{}`",
                    v.name
                )));
            }
        }
    }

    let other_variants: Vec<&Variant> = variants.iter().filter(|v| v.attrs.other).collect();
    if other_variants.len() > 1 {
        return Err(compile_error(&format!(
            "at most one variant can be `#[rusty_serde(other)]` (on `{name}`)"
        )));
    }
    if let Some(other) = other_variants.first() {
        if !matches!(other.fields, Fields::Unit) {
            return Err(compile_error(&format!(
                "`#[rusty_serde(other)]` is only supported on a unit variant (on `{}`)",
                other.name
            )));
        }
        if container_attrs.untagged {
            return Err(compile_error(&format!(
                "`#[rusty_serde(other)]` is not supported on an untagged enum - every \
                 variant is already tried in turn (on `{name}`)"
            )));
        }
    }

    Ok(Data::Enum {
        name,
        generics,
        variants,
        tag: container_attrs.tag,
        untagged: container_attrs.untagged,
        deny_unknown_fields: container_attrs.deny_unknown_fields,
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

/// Parses an optional `where ...` clause, stopping (without consuming)
/// at the item body - a `{ ... }` group or a top-level `;` - since that's
/// unambiguous regardless of what's inside the predicates themselves
/// (parenthesized `Fn(i32) -> bool` bounds and the like are already atomic
/// `Group` tokens, so they can't be mistaken for the body). Returns the
/// raw predicate text (empty string if there was no `where` at all).
fn parse_where_clause(tokens: &mut Tokens) -> Result<String, TokenStream> {
    match tokens.peek() {
        Some(TokenTree::Ident(id)) if id.to_string() == "where" => {}
        _ => return Ok(String::new()),
    }
    tokens.next();

    let mut raw = Vec::new();
    loop {
        match tokens.peek() {
            Some(TokenTree::Group(g)) if g.delimiter() == Delimiter::Brace => break,
            Some(TokenTree::Punct(p)) if p.as_char() == ';' => break,
            Some(_) => raw.push(tokens.next().unwrap()),
            None => break,
        }
    }
    Ok(TokenStream::from_iter(raw).to_string())
}

/// Applies a `rename_all` case style to every named field that didn't set
/// its own `rename`.
fn apply_rename_all_fields(fields: &mut [NamedField], style: &str) {
    for f in fields.iter_mut() {
        if f.attrs.rename.is_unset() {
            let converted = convert_case(&f.name, style);
            f.attrs.rename.serialize = Some(converted.clone());
            f.attrs.rename.deserialize = Some(converted);
        }
    }
}

/// Applies a `rename_all` case style to every variant that didn't set its
/// own `rename`. Only the variant's own tag is affected, not its fields'
/// names (put `rename_all` inside a per-variant `#[rusty_serde(...)]` if
/// that's ever needed - not currently supported).
fn apply_rename_all_variants(variants: &mut [Variant], style: &str) {
    for v in variants.iter_mut() {
        if v.attrs.rename.is_unset() {
            let converted = convert_case(&v.name, style);
            v.attrs.rename.serialize = Some(converted.clone());
            v.attrs.rename.deserialize = Some(converted);
        }
    }
}

const RENAME_ALL_STYLES: &[&str] = &[
    "lowercase",
    "UPPERCASE",
    "PascalCase",
    "camelCase",
    "snake_case",
    "SCREAMING_SNAKE_CASE",
    "kebab-case",
    "SCREAMING-KEBAB-CASE",
];

/// Splits a Rust identifier into lowercase words, regardless of whether it
/// was originally `snake_case` (splits on `_`) or `PascalCase`/`camelCase`
/// (splits at case boundaries, treating a run of capitals followed by a
/// lowercase letter - e.g. the `S` before `erver` in `HTTPServer` - as the
/// start of a new word).
fn split_words(ident: &str) -> Vec<String> {
    let chars: Vec<char> = ident.chars().collect();
    let mut words = Vec::new();
    let mut current = String::new();
    for (i, &c) in chars.iter().enumerate() {
        if c == '_' || c == '-' {
            if !current.is_empty() {
                words.push(std::mem::take(&mut current));
            }
            continue;
        }
        if c.is_uppercase() && !current.is_empty() {
            let prev = chars[i - 1];
            let next_lower = chars.get(i + 1).is_some_and(|n| n.is_lowercase());
            if prev.is_lowercase() || prev.is_numeric() || (prev.is_uppercase() && next_lower) {
                words.push(std::mem::take(&mut current));
            }
        }
        current.push(c.to_ascii_lowercase());
    }
    if !current.is_empty() {
        words.push(current);
    }
    words
}

fn capitalize(word: &str) -> String {
    let mut chars = word.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

/// Converts a Rust identifier to one of [`RENAME_ALL_STYLES`]. `style` is
/// assumed already validated (see its only caller in `parse_one_meta_item`).
fn convert_case(ident: &str, style: &str) -> String {
    let words = split_words(ident);
    match style {
        "lowercase" => words.concat(),
        "UPPERCASE" => words.concat().to_uppercase(),
        "PascalCase" => words.iter().map(|w| capitalize(w)).collect(),
        "camelCase" => words
            .iter()
            .enumerate()
            .map(|(i, w)| if i == 0 { w.clone() } else { capitalize(w) })
            .collect(),
        "snake_case" => words.join("_"),
        "SCREAMING_SNAKE_CASE" => words.join("_").to_uppercase(),
        "kebab-case" => words.join("-"),
        "SCREAMING-KEBAB-CASE" => words.join("-").to_uppercase(),
        other => unreachable!("`{other}` should have been rejected at attribute-parse time"),
    }
}

/// Parses the inside of a `{ ... }` field list: `ident : <type tokens>`,
/// repeated and comma-separated. Attributes (including `#[rusty_serde(...)]`)
/// and `pub`/`pub(...)` visibility ahead of a field name are consumed.
/// `owner` is `"struct"` for a top-level struct's fields or `"variant"` for
/// an enum variant's - `flatten` is only supported on the former (see the
/// check below).
fn parse_named_fields(group: proc_macro::Group, owner: &str) -> Result<Fields, TokenStream> {
    let mut tokens = group.stream().into_iter().peekable();
    let mut fields = Vec::new();

    loop {
        let attrs = parse_attrs(&mut tokens, "field")?;
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
        if attrs.flatten && attrs.skip {
            return Err(compile_error(&format!(
                "`flatten` and `skip` can't both be set (on field `{name}`)"
            )));
        }
        if attrs.flatten && !attrs.rename.is_unset() {
            return Err(compile_error(&format!(
                "`flatten` and `rename` can't both be set - a flattened field has no wire key \
                 of its own (on field `{name}`)"
            )));
        }
        if attrs.flatten && owner == "variant" {
            return Err(compile_error(&format!(
                "rusty_serde_derive only supports `flatten` on top-level struct fields, not \
                 enum variant fields (on field `{name}`)"
            )));
        }
        fields.push(NamedField { name, attrs });
    }

    let flatten_count = fields.iter().filter(|f| f.attrs.flatten).count();
    if flatten_count > 1 {
        return Err(compile_error(
            "rusty_serde_derive only supports one `#[rusty_serde(flatten)]` field per struct",
        ));
    }

    Ok(Fields::Named(fields))
}

/// Counts comma-separated entries inside a `( ... )` tuple field/variant
/// list, ignoring nested delimiters and visibility per entry.
/// `#[rusty_serde(...)]` isn't supported here (only named fields can carry
/// it - a tuple field has no name to key it by), so one is a clear error
/// rather than a silently-ignored attribute.
fn count_top_level_fields(group: proc_macro::Group) -> Result<usize, TokenStream> {
    let mut tokens = group.stream().into_iter().peekable();
    let mut count = 0;

    loop {
        let attrs = parse_attrs(&mut tokens, "field")?;
        if !attrs.is_default() {
            return Err(compile_error(
                "rusty_serde_derive only supports `#[rusty_serde(...)]` on named fields, not tuple fields",
            ));
        }
        skip_visibility(&mut tokens);
        if tokens.peek().is_none() {
            break;
        }
        skip_to_top_level_comma(&mut tokens);
        count += 1;
    }

    Ok(count)
}

/// Consumes every `#[...]` attribute ahead of a field/variant, merging any
/// `#[rusty_serde(...)]` found (at most one - a second is an error) into an
/// `Attrs`. Any other attribute is discarded, same as before field
/// attributes existed.
fn parse_attrs(tokens: &mut Tokens, context: &str) -> Result<Attrs, TokenStream> {
    let mut attrs = Attrs::default();
    let mut seen = false;
    loop {
        match tokens.peek() {
            Some(TokenTree::Punct(p)) if p.as_char() == '#' => {
                tokens.next();
                match tokens.next() {
                    Some(TokenTree::Group(g)) if g.delimiter() == Delimiter::Bracket => {
                        if let Some(parsed) = parse_one_attr_group(g, context)? {
                            if seen {
                                return Err(compile_error(&format!(
                                    "duplicate `#[rusty_serde(...)]` attribute on this {context}"
                                )));
                            }
                            seen = true;
                            attrs = parsed;
                        }
                    }
                    _ => return Err(compile_error("expected `[...]` after `#`")),
                }
            }
            _ => return Ok(attrs),
        }
    }
}

/// Parses one `#[...]` attribute's contents. Returns `Ok(None)` for any
/// attribute that isn't `rusty_serde(...)` (left untouched, same as
/// `#[derive(...)]`'s own siblings like `#[doc = "..."]`).
fn parse_one_attr_group(
    group: proc_macro::Group,
    context: &str,
) -> Result<Option<Attrs>, TokenStream> {
    let mut it = group.stream().into_iter().peekable();
    match it.peek() {
        Some(TokenTree::Ident(id)) if id.to_string() == "rusty_serde" => {}
        _ => return Ok(None),
    }
    it.next();
    let inner = match it.next() {
        Some(TokenTree::Group(inner)) if inner.delimiter() == Delimiter::Parenthesis => inner,
        Some(other) => {
            return Err(compile_error(&format!(
                "expected `rusty_serde(...)`, found `rusty_serde {other}`"
            )))
        }
        None => return Err(compile_error("expected `rusty_serde(...)`")),
    };
    if it.peek().is_some() {
        return Err(compile_error("unexpected tokens after `rusty_serde(...)`"));
    }

    let mut attrs = Attrs::default();
    let raw: Vec<TokenTree> = inner.stream().into_iter().collect();
    for chunk in split_top_level(raw, |depth, tt| {
        depth == 0 && matches!(tt, TokenTree::Punct(p) if p.as_char() == ',')
    }) {
        if chunk.is_empty() {
            continue;
        }
        parse_one_meta_item(chunk, context, &mut attrs)?;
    }
    Ok(Some(attrs))
}

fn parse_one_meta_item(
    chunk: Vec<TokenTree>,
    context: &str,
    attrs: &mut Attrs,
) -> Result<(), TokenStream> {
    let mut it = chunk.into_iter().peekable();
    let key = match it.next() {
        Some(TokenTree::Ident(id)) => id.to_string(),
        Some(other) => {
            return Err(compile_error(&format!(
                "expected an attribute name, found `{other}`"
            )))
        }
        None => return Ok(()),
    };
    match key.as_str() {
        "rename" => {
            if context == "container" {
                return Err(compile_error(
                    "`rename` is not supported on the container - did you mean `rename_all`?",
                ));
            }
            match it.next() {
                Some(TokenTree::Punct(p)) if p.as_char() == '=' => {
                    let value = match it.next() {
                        Some(TokenTree::Literal(lit)) => parse_string_literal(&lit)?,
                        _ => {
                            return Err(compile_error("expected a string literal after `rename =`"))
                        }
                    };
                    if it.peek().is_some() {
                        return Err(compile_error("unexpected tokens after `rename = \"...\"`"));
                    }
                    attrs.rename.serialize = Some(value.clone());
                    attrs.rename.deserialize = Some(value);
                }
                Some(TokenTree::Group(group)) if group.delimiter() == Delimiter::Parenthesis => {
                    let mut inner = group.stream().into_iter().peekable();
                    let mut set_any = false;
                    loop {
                        if inner.peek().is_none() {
                            break;
                        }
                        let key = match inner.next() {
                            Some(TokenTree::Ident(id)) => id.to_string(),
                            _ => {
                                return Err(compile_error(
                                    "expected `serialize` or `deserialize` in `rename(...)`",
                                ))
                            }
                        };
                        match inner.next() {
                            Some(TokenTree::Punct(p)) if p.as_char() == '=' => {}
                            _ => return Err(compile_error(&format!("expected `{key} = \"...\"`"))),
                        }
                        let value = match inner.next() {
                            Some(TokenTree::Literal(lit)) => parse_string_literal(&lit)?,
                            _ => {
                                return Err(compile_error(&format!(
                                    "expected a string literal after `{key} =`"
                                )))
                            }
                        };
                        match key.as_str() {
                            "serialize" => attrs.rename.serialize = Some(value),
                            "deserialize" => attrs.rename.deserialize = Some(value),
                            other => {
                                return Err(compile_error(&format!(
                                    "unknown key `{other}` in `rename(...)`, expected \
                                     `serialize` or `deserialize`"
                                )))
                            }
                        }
                        set_any = true;
                        match inner.next() {
                            Some(TokenTree::Punct(p)) if p.as_char() == ',' => continue,
                            None => break,
                            Some(other) => {
                                return Err(compile_error(&format!(
                                    "unexpected token `{other}` in `rename(...)`"
                                )))
                            }
                        }
                    }
                    if !set_any {
                        return Err(compile_error(
                            "expected `rename(serialize = \"...\")`, `rename(deserialize = \"...\")`, \
                             or both",
                        ));
                    }
                }
                _ => {
                    return Err(compile_error(
                        "expected `rename = \"...\"` or `rename(serialize = \"...\", deserialize = \"...\")`",
                    ))
                }
            }
        }
        "rename_all" => {
            if context != "container" {
                return Err(compile_error(&format!(
                    "`rename_all` is only supported on the container, not on a {context}"
                )));
            }
            match it.next() {
                Some(TokenTree::Punct(p)) if p.as_char() == '=' => {}
                _ => return Err(compile_error("expected `rename_all = \"...\"`")),
            }
            let value = match it.next() {
                Some(TokenTree::Literal(lit)) => parse_string_literal(&lit)?,
                _ => {
                    return Err(compile_error(
                        "expected a string literal after `rename_all =`",
                    ))
                }
            };
            if it.peek().is_some() {
                return Err(compile_error(
                    "unexpected tokens after `rename_all = \"...\"`",
                ));
            }
            if !RENAME_ALL_STYLES.contains(&value.as_str()) {
                return Err(compile_error(&format!(
                    "unknown `rename_all` style `{value}`, expected one of {RENAME_ALL_STYLES:?}"
                )));
            }
            attrs.rename_all = Some(value);
        }
        "tag" => {
            if context != "container" {
                return Err(compile_error(&format!(
                    "`tag` is only supported on the container, not on a {context}"
                )));
            }
            match it.next() {
                Some(TokenTree::Punct(p)) if p.as_char() == '=' => {}
                _ => return Err(compile_error("expected `tag = \"...\"`")),
            }
            let value = match it.next() {
                Some(TokenTree::Literal(lit)) => parse_string_literal(&lit)?,
                _ => return Err(compile_error("expected a string literal after `tag =`")),
            };
            if it.peek().is_some() {
                return Err(compile_error("unexpected tokens after `tag = \"...\"`"));
            }
            attrs.tag = Some(value);
        }
        "default" => {
            if context != "field" {
                return Err(compile_error(&format!(
                    "`default` is not supported on {context}s"
                )));
            }
            match it.peek() {
                None => {
                    attrs.default = Some(DefaultAttr::Trait);
                }
                Some(TokenTree::Punct(p)) if p.as_char() == '=' => {
                    it.next();
                    let value = match it.next() {
                        Some(TokenTree::Literal(lit)) => parse_string_literal(&lit)?,
                        _ => {
                            return Err(compile_error(
                                "expected a string literal after `default =`",
                            ))
                        }
                    };
                    if it.peek().is_some() {
                        return Err(compile_error("unexpected tokens after `default = \"...\"`"));
                    }
                    attrs.default = Some(DefaultAttr::Path(value));
                }
                Some(_) => return Err(compile_error("expected `default` or `default = \"...\"`")),
            }
        }
        "skip" => {
            if context != "field" {
                return Err(compile_error(&format!(
                    "`skip` is not supported on {context}s"
                )));
            }
            if it.peek().is_some() {
                return Err(compile_error("`skip` does not take a value"));
            }
            attrs.skip = true;
        }
        "skip_serializing" => {
            if context != "field" {
                return Err(compile_error(&format!(
                    "`skip_serializing` is not supported on {context}s"
                )));
            }
            if it.peek().is_some() {
                return Err(compile_error("`skip_serializing` does not take a value"));
            }
            attrs.skip_serializing = true;
        }
        "skip_deserializing" => {
            if context != "field" {
                return Err(compile_error(&format!(
                    "`skip_deserializing` is not supported on {context}s"
                )));
            }
            if it.peek().is_some() {
                return Err(compile_error("`skip_deserializing` does not take a value"));
            }
            attrs.skip_deserializing = true;
        }
        "skip_serializing_if" => {
            if context != "field" {
                return Err(compile_error(&format!(
                    "`skip_serializing_if` is not supported on {context}s"
                )));
            }
            match it.next() {
                Some(TokenTree::Punct(p)) if p.as_char() == '=' => {}
                _ => return Err(compile_error("expected `skip_serializing_if = \"...\"`")),
            }
            let value = match it.next() {
                Some(TokenTree::Literal(lit)) => parse_string_literal(&lit)?,
                _ => {
                    return Err(compile_error(
                        "expected a string literal after `skip_serializing_if =`",
                    ))
                }
            };
            if it.peek().is_some() {
                return Err(compile_error(
                    "unexpected tokens after `skip_serializing_if = \"...\"`",
                ));
            }
            attrs.skip_serializing_if = Some(value);
        }
        "flatten" => {
            if context != "field" {
                return Err(compile_error(&format!(
                    "`flatten` is not supported on {context}s"
                )));
            }
            if it.peek().is_some() {
                return Err(compile_error("`flatten` does not take a value"));
            }
            attrs.flatten = true;
        }
        "alias" => {
            if context != "field" {
                return Err(compile_error(&format!(
                    "`alias` is not supported on {context}s"
                )));
            }
            match it.next() {
                Some(TokenTree::Punct(p)) if p.as_char() == '=' => {}
                _ => return Err(compile_error("expected `alias = \"...\"`")),
            }
            let value = match it.next() {
                Some(TokenTree::Literal(lit)) => parse_string_literal(&lit)?,
                _ => return Err(compile_error("expected a string literal after `alias =`")),
            };
            if it.peek().is_some() {
                return Err(compile_error("unexpected tokens after `alias = \"...\"`"));
            }
            attrs.aliases.push(value);
        }
        "untagged" => {
            if context != "container" {
                return Err(compile_error(&format!(
                    "`untagged` is only supported on the container, not on a {context}"
                )));
            }
            if it.peek().is_some() {
                return Err(compile_error("`untagged` does not take a value"));
            }
            attrs.untagged = true;
        }
        "deny_unknown_fields" => {
            if context != "container" {
                return Err(compile_error(&format!(
                    "`deny_unknown_fields` is only supported on the container, not on a {context}"
                )));
            }
            if it.peek().is_some() {
                return Err(compile_error("`deny_unknown_fields` does not take a value"));
            }
            attrs.deny_unknown_fields = true;
        }
        "other" => {
            if context != "variant" {
                return Err(compile_error(&format!(
                    "`other` is only supported on an enum variant, not on a {context}"
                )));
            }
            if it.peek().is_some() {
                return Err(compile_error("`other` does not take a value"));
            }
            attrs.other = true;
        }
        "transparent" => {
            if context != "container" {
                return Err(compile_error(&format!(
                    "`transparent` is only supported on the container, not on a {context}"
                )));
            }
            if it.peek().is_some() {
                return Err(compile_error("`transparent` does not take a value"));
            }
            attrs.transparent = true;
        }
        "bound" => {
            if context != "container" {
                return Err(compile_error(&format!(
                    "`bound` is only supported on the container, not on a {context}"
                )));
            }
            match it.next() {
                Some(TokenTree::Punct(p)) if p.as_char() == '=' => {}
                _ => return Err(compile_error("expected `bound = \"...\"`")),
            }
            let value = match it.next() {
                Some(TokenTree::Literal(lit)) => parse_string_literal(&lit)?,
                _ => return Err(compile_error("expected a string literal after `bound =`")),
            };
            if it.peek().is_some() {
                return Err(compile_error("unexpected tokens after `bound = \"...\"`"));
            }
            attrs.bound = Some(value);
        }
        other => {
            return Err(compile_error(&format!(
                "unknown rusty_serde attribute `{other}`"
            )))
        }
    }
    Ok(())
}

/// Unescapes a source-text string literal (`proc_macro::Literal::to_string`
/// returns the raw token text, quotes included). Handles the escapes that
/// realistically show up in a `rename = "..."` value; anything else is
/// passed through unchanged rather than rejected.
fn parse_string_literal(lit: &proc_macro::Literal) -> Result<String, TokenStream> {
    let text = lit.to_string();
    if !(text.starts_with('"') && text.ends_with('"') && text.len() >= 2) {
        return Err(compile_error(&format!(
            "expected a string literal, found `{text}`"
        )));
    }
    let inner = &text[1..text.len() - 1];
    let mut out = String::with_capacity(inner.len());
    let mut chars = inner.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('"') => out.push('"'),
            Some('\\') => out.push('\\'),
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('r') => out.push('\r'),
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
            None => out.push('\\'),
        }
    }
    Ok(out)
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
        None => Err(compile_error(&format!(
            "expected {what}, found end of input"
        ))),
    }
}

pub fn compile_error(msg: &str) -> TokenStream {
    format!("compile_error!({msg:?});")
        .parse()
        .expect("compile_error! invocation is always valid Rust")
}
