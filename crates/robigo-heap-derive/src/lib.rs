//! `#[derive(Trace)]` for `robigo_heap::Trace` (docs/g3.md §3, "What a
//! collection marks", item 1): the derived `trace` shows the visitor every
//! field through the field's own `Trace`, so a field that holds an object
//! and has no `Trace` (a raw pointer, a foreign type) does not compile.
//! A field the collector must not see takes `#[trace(skip = "reason")]`;
//! the reason is required, so every exception is written down where it is
//! made.
//!
//! Each type parameter gets a `Trace` bound, also one that only a skipped
//! field uses: implement `Trace` by hand for such a type. Unions are
//! rejected: the derive cannot know which field is live. `#[trace(...)]`
//! belongs on fields only, and is rejected on the type, on an enum variant,
//! and when empty: an annotation that does nothing is a compile error.
//!
//! The generated names cannot collide with the input's: the visitor and the
//! tuple-field bindings are local variables with `Span::mixed_site()`
//! hygiene, and the visitor's type parameter is renamed until no type
//! parameter of the input has its name.

#![forbid(unsafe_code)]

// The checkout this crate is built from (.cargo/config.toml): cargo then
// rebuilds the crate when another worktree built it last in a shared
// target directory.
const _: Option<&str> = option_env!("ROBIGO_WORKTREE");

use proc_macro2::{Ident, Span, TokenStream};
use quote::{format_ident, quote};
use syn::spanned::Spanned;
use syn::{Data, DeriveInput, Fields, LitStr, parse_macro_input, parse_quote};

#[proc_macro_derive(Trace, attributes(trace))]
pub fn derive_trace(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    match expand(input) {
        Ok(ts) => ts.into(),
        Err(e) => e.to_compile_error().into(),
    }
}

fn expand(mut input: DeriveInput) -> syn::Result<TokenStream> {
    no_trace_attribute(&input.attrs, "a type")?;
    let v = visitor();
    let body = match &input.data {
        Data::Struct(s) => {
            let (pattern, calls) = destructure(&s.fields)?;
            quote! {
                let Self #pattern = self;
                #calls
            }
        }
        Data::Enum(e) => {
            let mut arms = Vec::new();
            for var in &e.variants {
                no_trace_attribute(&var.attrs, "an enum variant")?;
                let (pattern, calls) = destructure(&var.fields)?;
                let ident = &var.ident;
                arms.push(quote! { Self::#ident #pattern => { #calls } });
            }
            if arms.is_empty() {
                quote! { match *self {} }
            } else {
                quote! { match self { #(#arms)* } }
            }
        }
        Data::Union(u) => {
            return Err(syn::Error::new(
                u.union_token.span(),
                "Trace cannot be derived for a union: which field holds a value is not known",
            ));
        }
    };
    // the visitor's type parameter: a name no type parameter of the input has
    let mut vt = format_ident!("__RobigoV");
    while input.generics.type_params().any(|p| p.ident == vt) {
        vt = format_ident!("{}_", vt);
    }
    for p in input.generics.type_params_mut() {
        p.bounds.push(parse_quote!(::robigo_heap::Trace));
    }
    let name = &input.ident;
    let (impl_generics, ty_generics, where_clause) = input.generics.split_for_impl();
    Ok(quote! {
        impl #impl_generics ::robigo_heap::Trace for #name #ty_generics #where_clause {
            #[inline]
            fn trace<#vt: ::robigo_heap::Visitor + ?::core::marker::Sized>(&mut self, #v: &mut #vt) {
                // (used by no call when every field is skipped)
                let _ = &#v;
                #body
            }
        }
    })
}

/// The visitor's name in the generated body: a local variable with
/// definition-site hygiene, so a field bound under its own name (even
/// `__robigo_v`) cannot shadow it.
fn visitor() -> Ident {
    Ident::new("__robigo_v", Span::mixed_site())
}

/// `#[trace(...)]` is for fields: on `what` (the type, a variant) it would
/// do nothing, so it is an error.
fn no_trace_attribute(attrs: &[syn::Attribute], what: &str) -> syn::Result<()> {
    match attrs.iter().find(|a| a.path().is_ident("trace")) {
        Some(a) => Err(syn::Error::new(
            a.span(),
            format!("`#[trace(...)]` goes on fields, not on {what}: mark each field the collector must not see"),
        )),
        None => Ok(()),
    }
}

/// A pattern binding every traced field of `fields` by mutable reference,
/// and the calls that trace them.
fn destructure(fields: &Fields) -> syn::Result<(TokenStream, TokenStream)> {
    let v = visitor();
    let mut calls = Vec::new();
    let pattern = match fields {
        Fields::Named(named) => {
            let mut binds = Vec::new();
            for f in &named.named {
                if skipped(&f.attrs)? {
                    continue;
                }
                let id = f.ident.as_ref().expect("a named field has a name");
                binds.push(quote! { #id });
                calls.push(quote! { ::robigo_heap::Trace::trace(#id, #v); });
            }
            quote! { { #(#binds,)* .. } }
        }
        Fields::Unnamed(unnamed) => {
            let mut binds = Vec::new();
            for (i, f) in unnamed.unnamed.iter().enumerate() {
                if skipped(&f.attrs)? {
                    binds.push(quote! { _ });
                    continue;
                }
                let id = Ident::new(&format!("__robigo_f{i}"), Span::mixed_site());
                binds.push(quote! { #id });
                calls.push(quote! { ::robigo_heap::Trace::trace(#id, #v); });
            }
            quote! { ( #(#binds),* ) }
        }
        Fields::Unit => quote! {},
    };
    Ok((pattern, quote! { #(#calls)* }))
}

/// Whether a field carries `#[trace(skip = "reason")]`.
fn skipped(attrs: &[syn::Attribute]) -> syn::Result<bool> {
    let mut skip = false;
    for a in attrs {
        if !a.path().is_ident("trace") {
            continue;
        }
        if matches!(a.meta, syn::Meta::Path(_)) {
            return Err(syn::Error::new(
                a.span(),
                "an empty `#[trace]` does nothing: write `#[trace(skip = \"reason\")]`",
            ));
        }
        let mut items = 0;
        a.parse_nested_meta(|meta| {
            items += 1;
            if !meta.path.is_ident("skip") {
                return Err(meta.error("unknown trace attribute: the one attribute is `skip = \"reason\"`"));
            }
            if meta.input.is_empty() || meta.input.peek(syn::Token![,]) {
                return Err(meta.error("`#[trace(skip)]` needs its reason: `#[trace(skip = \"why the collector need not see this field\")]`"));
            }
            let reason: LitStr = meta.value()?.parse()?;
            if reason.value().trim().is_empty() {
                return Err(syn::Error::new(reason.span(), "the reason for a skipped field must not be empty"));
            }
            skip = true;
            Ok(())
        })?;
        if items == 0 {
            return Err(syn::Error::new(
                a.span(),
                "an empty `#[trace()]` does nothing: write `#[trace(skip = \"reason\")]`",
            ));
        }
    }
    Ok(skip)
}
