//! Procedural macros for the Rust ADK.
//!
//! [`macro@adk_tool`] turns an ordinary `async fn` into an `adk_tools::Tool` whose
//! declaration is derived from the signature and doc comment — the same thing
//! ADK's other SDKs do by reflecting over a function at run time, done here at
//! compile time instead.

use proc_macro::TokenStream;
use quote::{format_ident, quote};
use syn::{
    parse_macro_input, Attribute, Expr, ExprLit, FnArg, ItemFn, Lit, Meta, Pat, PathArguments,
    ReturnType, Type,
};

/// Turns an `async fn` into a tool.
///
/// The generated type implements `adk_tools::Tool`:
///
/// - the tool's **name** is the function's name;
/// - its **description** is the function's doc comment, which is what the
///   model reads to decide when to call it, so write it for the model;
/// - its **parameter schema** is derived from the argument types, with
///   `Option<T>` arguments becoming optional properties;
/// - an argument of type `&ToolContext` is injected by the framework and
///   hidden from the model, matching ADK's convention.
///
/// The function returns `adk_core::Result<T>` where `T: Serialize`; the value
/// is normalized to ADK's object convention before it reaches the model.
///
/// # Example
///
/// ```ignore
/// use adk_macros::adk_tool;
/// use adk_tools::ToolContext;
///
/// /// Retrieves the current weather for a city.
/// #[adk_tool]
/// async fn get_weather(city: String, unit: Option<String>) -> adk_core::Result<serde_json::Value> {
///     Ok(serde_json::json!({ "status": "success", "city": city, "unit": unit }))
/// }
///
/// // The macro also generates a constructor returning the tool.
/// let tool = get_weather_tool();
/// ```
///
/// # Reaching the ADK through the facade
///
/// The generated code resolves its paths through `::adk_tools` by default. A
/// crate that depends only on `rusty-adk` names the re-export instead:
///
/// ```ignore
/// #[adk_tool(crate = ::rusty_adk::tools)]
/// async fn get_weather(city: String) -> rusty_adk::core::Result<serde_json::Value> { .. }
/// ```
#[proc_macro_attribute]
pub fn adk_tool(attr: TokenStream, item: TokenStream) -> TokenStream {
    let func = parse_macro_input!(item as ItemFn);
    let root = match parse_crate_path(attr.into()) {
        Ok(root) => root,
        Err(err) => return err.to_compile_error().into(),
    };
    match expand(func, root) {
        Ok(tokens) => tokens.into(),
        Err(err) => err.to_compile_error().into(),
    }
}

/// Parses the optional `crate = <path>` argument.
///
/// Defaults to `::adk_tools`. A crate that reaches the ADK through the
/// `rusty-adk` facade passes `crate = ::rusty_adk::tools` so the generated code
/// resolves without adding `adk-tools` to its own manifest.
fn parse_crate_path(attr: proc_macro2::TokenStream) -> syn::Result<syn::Path> {
    if attr.is_empty() {
        return Ok(syn::parse_quote!(::adk_tools));
    }
    let meta: Meta = syn::parse2(attr)?;
    let Meta::NameValue(nv) = meta else {
        return Err(syn::Error::new_spanned(meta, "expected `crate = <path>`"));
    };
    if !nv.path.is_ident("crate") {
        return Err(syn::Error::new_spanned(
            nv.path,
            "the only supported argument is `crate = <path>`",
        ));
    }
    match nv.value {
        Expr::Path(path) => Ok(path.path),
        other => Err(syn::Error::new_spanned(other, "expected a path")),
    }
}

/// One declared parameter of the tool.
struct Param {
    name: String,
    ident: syn::Ident,
    ty: Type,
    optional: bool,
}

fn expand(func: ItemFn, root: syn::Path) -> syn::Result<proc_macro2::TokenStream> {
    if func.sig.asyncness.is_none() {
        return Err(syn::Error::new_spanned(
            &func.sig,
            "#[adk_tool] requires an `async fn`",
        ));
    }

    let fn_name = func.sig.ident.clone();
    let description = doc_comment(&func.attrs);
    if description.is_empty() {
        return Err(syn::Error::new_spanned(
            &func.sig.ident,
            "#[adk_tool] requires a doc comment: it becomes the tool description \
             the model reads to decide when to call this tool",
        ));
    }

    let struct_name = format_ident!("{}Tool", to_pascal_case(&fn_name.to_string()));
    let ctor_name = format_ident!("{}_tool", fn_name);

    let mut params = Vec::new();
    let mut wants_context = false;

    for arg in &func.sig.inputs {
        let FnArg::Typed(pat_type) = arg else {
            return Err(syn::Error::new_spanned(
                arg,
                "#[adk_tool] does not support `self` arguments",
            ));
        };

        if is_tool_context(&pat_type.ty) {
            wants_context = true;
            continue;
        }

        let Pat::Ident(pat_ident) = &*pat_type.pat else {
            return Err(syn::Error::new_spanned(
                &pat_type.pat,
                "#[adk_tool] arguments must be plain identifiers",
            ));
        };

        let (inner, optional) = match unwrap_option(&pat_type.ty) {
            Some(inner) => (inner.clone(), true),
            None => ((*pat_type.ty).clone(), false),
        };

        params.push(Param {
            name: pat_ident.ident.to_string(),
            ident: pat_ident.ident.clone(),
            ty: inner,
            optional,
        });
    }

    if !matches!(func.sig.output, ReturnType::Type(..)) {
        return Err(syn::Error::new_spanned(
            &func.sig,
            "#[adk_tool] requires an explicit return type of `adk_core::Result<T>`",
        ));
    }

    // Build the parameter schema.
    let schema_entries = params.iter().map(|p| {
        let name = &p.name;
        let ty = &p.ty;
        let doc = format!("The {} parameter.", p.name);
        if p.optional {
            quote! {
                schema = schema.optional_property(
                    #name,
                    <#ty as #root::__macro_support::HasSchema>::schema().describe(#doc),
                );
            }
        } else {
            quote! {
                schema = schema.property(
                    #name,
                    <#ty as #root::__macro_support::HasSchema>::schema().describe(#doc),
                );
            }
        }
    });

    // Extract each argument from the JSON argument map.
    let extractions = params.iter().map(|p| {
        let ident = &p.ident;
        let name = &p.name;
        let ty = &p.ty;
        if p.optional {
            quote! {
                let #ident: ::core::option::Option<#ty> = match args.get(#name) {
                    ::core::option::Option::None | ::core::option::Option::Some(
                        #root::__macro_support::serde_json::Value::Null
                    ) => ::core::option::Option::None,
                    ::core::option::Option::Some(raw) => ::core::option::Option::Some(
                        #root::__macro_support::serde_json::from_value(raw.clone()).map_err(|e| {
                            #root::__macro_support::AdkError::validation(#name, e.to_string())
                        })?,
                    ),
                };
            }
        } else {
            quote! {
                let #ident: #ty = {
                    let raw = args.get(#name).ok_or_else(|| {
                        #root::__macro_support::AdkError::validation(#name, "required argument is missing")
                    })?;
                    #root::__macro_support::serde_json::from_value(raw.clone()).map_err(|e| {
                        #root::__macro_support::AdkError::validation(#name, e.to_string())
                    })?
                };
            }
        }
    });

    let call_args = params.iter().map(|p| {
        let ident = &p.ident;
        quote! { #ident }
    });
    let call = if wants_context {
        quote! { #fn_name(#(#call_args,)* ctx).await? }
    } else {
        quote! { #fn_name(#(#call_args),*).await? }
    };

    let name_literal = fn_name.to_string();
    let vis = &func.vis;

    Ok(quote! {
        #func

        #[doc = concat!("Tool wrapper generated by `#[adk_tool]` for [`", stringify!(#fn_name), "`].")]
        #[derive(Debug, Clone, Copy, Default)]
        #vis struct #struct_name;

        #[#root::__macro_support::async_trait]
        impl #root::__macro_support::Tool for #struct_name {
            fn name(&self) -> &str {
                #name_literal
            }

            fn description(&self) -> &str {
                #description
            }

            fn declaration(&self) -> ::core::option::Option<#root::__macro_support::FunctionDeclaration> {
                let mut schema = #root::__macro_support::Schema::object();
                #(#schema_entries)*
                ::core::option::Option::Some(
                    #root::__macro_support::FunctionDeclaration::new(#name_literal, #description)
                        .with_parameters(schema),
                )
            }

            async fn run(
                &self,
                args: #root::__macro_support::Args,
                ctx: &#root::__macro_support::ToolContext,
            ) -> #root::__macro_support::Result<#root::__macro_support::serde_json::Value> {
                let _ = ctx;
                #(#extractions)*
                let result = #call;
                ::core::result::Result::Ok(#root::__macro_support::serde_json::to_value(result)?)
            }
        }

        #[doc = concat!("Builds the [`", stringify!(#struct_name), "`] tool.")]
        #vis fn #ctor_name() -> #root::__macro_support::Arc<dyn #root::__macro_support::Tool> {
            #root::__macro_support::Arc::new(#struct_name)
        }
    })
}

/// Joins `///` lines into the tool description.
fn doc_comment(attrs: &[Attribute]) -> String {
    let mut lines = Vec::new();
    for attr in attrs {
        if !attr.path().is_ident("doc") {
            continue;
        }
        if let Meta::NameValue(nv) = &attr.meta {
            if let Expr::Lit(ExprLit {
                lit: Lit::Str(s), ..
            }) = &nv.value
            {
                lines.push(s.value().trim().to_string());
            }
        }
    }
    lines.join(" ").trim().to_string()
}

/// True for `&ToolContext` and `ToolContext` arguments.
fn is_tool_context(ty: &Type) -> bool {
    let inner = match ty {
        Type::Reference(r) => &*r.elem,
        other => other,
    };
    match inner {
        Type::Path(p) => p
            .path
            .segments
            .last()
            .map(|s| s.ident == "ToolContext")
            .unwrap_or(false),
        _ => false,
    }
}

/// Returns the inner type of an `Option<T>`.
fn unwrap_option(ty: &Type) -> Option<&Type> {
    let Type::Path(path) = ty else {
        return None;
    };
    let segment = path.path.segments.last()?;
    if segment.ident != "Option" {
        return None;
    }
    let PathArguments::AngleBracketed(args) = &segment.arguments else {
        return None;
    };
    args.args.iter().find_map(|arg| match arg {
        syn::GenericArgument::Type(t) => Some(t),
        _ => None,
    })
}

fn to_pascal_case(name: &str) -> String {
    name.split('_')
        .filter(|s| !s.is_empty())
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
                None => String::new(),
            }
        })
        .collect()
}
