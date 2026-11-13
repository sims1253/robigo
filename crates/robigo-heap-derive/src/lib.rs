//! `#[derive(Trace)]` for `robigo_heap::Trace` (docs/g3.md §3, "What a
//! collection marks", item 1): the derived `trace` shows the visitor every
//! field through the field's own `Trace`, so a field that holds an object
//! and has no `Trace` (a raw pointer, a foreign type) does not compile.
//! A field the collector must not see takes `#[trace(skip = "reason")]`;
//! the reason is required, so every exception is written down where it is
//! made.
//!
//! Each type parameter gets a `Trace` bound. Unions are rejected: the
//! derive cannot know which field is live.

#![forbid(unsafe_code)]

use proc_macro2::TokenStream;
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
    let name = input.ident.clone();
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
            for v in &e.variants {
                let (pattern, calls) = destructure(&v.fields)?;
                let ident = &v.ident;
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
    for p in input.generics.type_params_mut() {
        p.bounds.push(parse_quote!(::robigo_heap::Trace));
    }
    let (impl_generics, ty_generics, where_clause) = input.generics.split_for_impl();
    Ok(quote! {
        impl #impl_generics ::robigo_heap::Trace for #name #ty_generics #where_clause {
            #[inline]
            fn trace<__RobigoV: ::robigo_heap::Visitor + ?::core::marker::Sized>(&mut self, __robigo_v: &mut __RobigoV) {
                // (used by no call when every field is skipped)
                let _ = &__robigo_v;
                #body
            }
        }
    })
}

/// A pattern binding every traced field of `fields` by mutable reference,
/// and the calls that trace them.
fn destructure(fields: &Fields) -> syn::Result<(TokenStream, TokenStream)> {
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
                calls.push(quote! { ::robigo_heap::Trace::trace(#id, __robigo_v); });
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
                let id = format_ident!("__robigo_f{}", i);
                binds.push(quote! { #id });
                calls.push(quote! { ::robigo_heap::Trace::trace(#id, __robigo_v); });
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
        a.parse_nested_meta(|meta| {
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
    }
    Ok(skip)
}
