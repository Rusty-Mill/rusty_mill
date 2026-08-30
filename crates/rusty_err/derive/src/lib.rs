//! Proc-macro `#[derive(Error)]` for [`rusty_err`](https://docs.rs/rusty_err),
//! matching `thiserror`'s `#[error("...")]` / `#[from]` shape.
//!
//! Supports enums only. Each variant needs an `#[error("...")]` message; a
//! field may additionally be marked `#[from]` (generates a `From` impl and
//! makes the field the error's source) or `#[source]` (makes the field the
//! error's source without generating a `From` impl).

use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::quote;
use syn::{Data, DeriveInput, Field, Fields, LitStr, parse_macro_input};

#[proc_macro_derive(Error, attributes(error, from, source))]
pub fn derive_error(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    expand(input)
        .unwrap_or_else(|err| err.to_compile_error())
        .into()
}

fn expand(input: DeriveInput) -> syn::Result<TokenStream2> {
    let name = &input.ident;
    let (impl_generics, ty_generics, where_clause) = input.generics.split_for_impl();

    let data = match &input.data {
        Data::Enum(data) => data,
        _ => {
            return Err(syn::Error::new_spanned(
                &input,
                "`#[derive(rusty_err::Error)]` currently supports enums only",
            ));
        }
    };

    let mut display_arms = Vec::new();
    let mut source_arms = Vec::new();
    let mut from_impls = Vec::new();
    let mut has_source = false;

    for variant in &data.variants {
        let variant_ident = &variant.ident;

        let error_attr = variant
            .attrs
            .iter()
            .find(|attr| attr.path().is_ident("error"))
            .ok_or_else(|| {
                syn::Error::new_spanned(
                    variant,
                    "every variant must have an `#[error(\"...\")]` attribute",
                )
            })?;
        let lit: LitStr = error_attr.parse_args()?;
        let rewritten = LitStr::new(&rewrite_format_string(&lit.value()), lit.span());

        let VariantShape {
            pattern,
            source_field,
            from_field,
        } = shape_variant(variant_ident, &variant.fields)?;

        display_arms.push(quote! {
            #pattern => ::core::write!(f, #rewritten)
        });

        if let Some(source_binder) = &source_field {
            has_source = true;
            source_arms.push(quote! {
                #pattern => ::core::option::Option::Some(
                    #source_binder as &(dyn ::rusty_err::Error + 'static)
                )
            });
        }

        if let Some((binder, ty)) = from_field {
            from_impls.push(quote! {
                impl #impl_generics ::core::convert::From<#ty> for #name #ty_generics #where_clause {
                    fn from(#binder: #ty) -> Self {
                        #pattern
                    }
                }
            });
        }
    }

    let display_impl = quote! {
        impl #impl_generics ::core::fmt::Display for #name #ty_generics #where_clause {
            #[allow(unused_variables)]
            fn fmt(&self, f: &mut ::core::fmt::Formatter<'_>) -> ::core::fmt::Result {
                match self {
                    #(#display_arms,)*
                }
            }
        }
    };

    let error_impl = if has_source {
        quote! {
            impl #impl_generics ::rusty_err::Error for #name #ty_generics #where_clause {
                #[allow(unused_variables)]
                fn source(&self) -> ::core::option::Option<&(dyn ::rusty_err::Error + 'static)> {
                    match self {
                        #(#source_arms,)*
                        _ => ::core::option::Option::None,
                    }
                }
            }
        }
    } else {
        quote! {
            impl #impl_generics ::rusty_err::Error for #name #ty_generics #where_clause {}
        }
    };

    Ok(quote! {
        #display_impl
        #error_impl
        #(#from_impls)*
    })
}

struct VariantShape {
    /// Usable both as a match pattern and, for single-field `#[from]`
    /// variants, as a constructor expression.
    pattern: TokenStream2,
    source_field: Option<TokenStream2>,
    from_field: Option<(syn::Ident, syn::Type)>,
}

fn shape_variant(variant_ident: &syn::Ident, fields: &Fields) -> syn::Result<VariantShape> {
    match fields {
        Fields::Unit => Ok(VariantShape {
            pattern: quote! { Self::#variant_ident },
            source_field: None,
            from_field: None,
        }),
        Fields::Unnamed(unnamed) => {
            let mut binders = Vec::new();
            let mut source_field = None;
            let mut from_field = None;
            for (i, field) in unnamed.unnamed.iter().enumerate() {
                let binder = quote::format_ident!("_{}", i);
                record_attrs(field, &binder, &mut source_field, &mut from_field)?;
                binders.push(binder);
            }
            if from_field.is_some() && binders.len() != 1 {
                return Err(syn::Error::new_spanned(
                    unnamed,
                    "`#[from]` requires the variant to have exactly one field",
                ));
            }
            Ok(VariantShape {
                pattern: quote! { Self::#variant_ident(#(#binders),*) },
                source_field: source_field.map(|b: syn::Ident| quote! { #b }),
                from_field,
            })
        }
        Fields::Named(named) => {
            let mut binders = Vec::new();
            let mut source_field = None;
            let mut from_field = None;
            for field in &named.named {
                let binder = field.ident.clone().expect("named field has an ident");
                record_attrs(field, &binder, &mut source_field, &mut from_field)?;
                binders.push(binder);
            }
            if from_field.is_some() && binders.len() != 1 {
                return Err(syn::Error::new_spanned(
                    named,
                    "`#[from]` requires the variant to have exactly one field",
                ));
            }
            Ok(VariantShape {
                pattern: quote! { Self::#variant_ident { #(#binders),* } },
                source_field: source_field.map(|b: syn::Ident| quote! { #b }),
                from_field,
            })
        }
    }
}

fn record_attrs(
    field: &Field,
    binder: &syn::Ident,
    source_field: &mut Option<syn::Ident>,
    from_field: &mut Option<(syn::Ident, syn::Type)>,
) -> syn::Result<()> {
    let is_from = field.attrs.iter().any(|attr| attr.path().is_ident("from"));
    let is_source = field
        .attrs
        .iter()
        .any(|attr| attr.path().is_ident("source"));

    if is_from || is_source {
        if source_field.is_some() {
            return Err(syn::Error::new_spanned(
                field,
                "only one field per variant may be marked `#[source]`/`#[from]`",
            ));
        }
        *source_field = Some(binder.clone());
    }
    if is_from {
        *from_field = Some((binder.clone(), field.ty.clone()));
    }
    Ok(())
}

/// Rewrites bare positional placeholders (`{0}`, `{1:?}`, ...) into named
/// captures (`{_0}`, `{_1:?}`, ...) of the identifiers the derive binds
/// tuple-variant fields to, so the generated `write!` can rely on Rust's
/// implicit-capture format strings instead of passing positional arguments
/// (which would error on fields the message doesn't reference). Named
/// placeholders (`{field}`) are left untouched, since the field's own name
/// is already in scope after destructuring.
fn rewrite_format_string(input: &str) -> String {
    let chars: Vec<char> = input.chars().collect();
    let mut out = String::with_capacity(input.len());
    let mut i = 0;
    while i < chars.len() {
        match chars[i] {
            '{' if chars.get(i + 1) == Some(&'{') => {
                out.push_str("{{");
                i += 2;
            }
            '{' => {
                if let Some(rel_end) = chars[i + 1..].iter().position(|&c| c == '}') {
                    let end = i + 1 + rel_end;
                    let inner: String = chars[i + 1..end].iter().collect();
                    let (name_part, rest) = match inner.find(':') {
                        Some(pos) => (&inner[..pos], &inner[pos..]),
                        None => (inner.as_str(), ""),
                    };
                    if !name_part.is_empty() && name_part.bytes().all(|b| b.is_ascii_digit()) {
                        out.push_str("{_");
                        out.push_str(name_part);
                        out.push_str(rest);
                        out.push('}');
                    } else {
                        out.push('{');
                        out.push_str(&inner);
                        out.push('}');
                    }
                    i = end + 1;
                } else {
                    out.push('{');
                    i += 1;
                }
            }
            '}' if chars.get(i + 1) == Some(&'}') => {
                out.push_str("}}");
                i += 2;
            }
            c => {
                out.push(c);
                i += 1;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::rewrite_format_string;

    #[test]
    fn rewrites_bare_positional_index() {
        assert_eq!(
            rewrite_format_string("index `{0}` not found"),
            "index `{_0}` not found"
        );
    }

    #[test]
    fn rewrites_positional_index_with_format_spec() {
        assert_eq!(rewrite_format_string("{0:?}"), "{_0:?}");
    }

    #[test]
    fn leaves_named_placeholders_untouched() {
        assert_eq!(
            rewrite_format_string("bad value: {field}"),
            "bad value: {field}"
        );
    }

    #[test]
    fn leaves_escaped_braces_untouched() {
        assert_eq!(rewrite_format_string("{{literal}} {0}"), "{{literal}} {_0}");
    }

    #[test]
    fn rewrites_multiple_positional_indices() {
        assert_eq!(rewrite_format_string("{0} and {1}"), "{_0} and {_1}");
    }
}
