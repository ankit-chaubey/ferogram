// Copyright (c) Ankit Chaubey <ankitchaubey.dev@gmail.com>
// SPDX-License-Identifier: MIT OR Apache-2.0
//
// ferogram: async Telegram MTProto client in Rust
// https://github.com/ankit-chaubey/ferogram
//
// If you use or modify this code, keep this notice at the top of your file
// and include the LICENSE-MIT or LICENSE-APACHE file from this repository:
// https://github.com/ankit-chaubey/ferogram

use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::quote;
use syn::{Data, DeriveInput, Fields, parse_macro_input, spanned::Spanned};

/// Derive the [`ferogram::fsm::FsmState`] trait for an enum.
///
/// Only **unit variants** (no fields) are supported. Tuple or struct variants
/// are rejected with a compile error.
///
/// # What gets generated
///
/// - `as_key(&self) -> String` - returns the variant name as a `&'static str`-backed `String`.
/// - `from_key(key: &str) -> Option<Self>` - parses the variant name back into the enum.
///
/// # Example
///
/// ```rust,no_run
/// use ferogram::FsmState;
///
/// #[derive(FsmState, Clone, Debug, PartialEq)]
/// enum RegistrationState {
///     Start,
///     WaitingName,
///     WaitingPhone,
///     WaitingCity,
///     Done,
/// }
/// ```
#[proc_macro_derive(FsmState)]
pub fn derive_fsm_state(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    match fsm_state_impl(input) {
        Ok(ts) => ts.into(),
        Err(e) => e.to_compile_error().into(),
    }
}

fn fsm_state_impl(input: DeriveInput) -> syn::Result<TokenStream2> {
    let name = &input.ident;
    let (impl_generics, ty_generics, where_clause) = input.generics.split_for_impl();

    let data_enum = match &input.data {
        Data::Enum(e) => e,
        _ => {
            return Err(syn::Error::new(
                input.ident.span(),
                "`#[derive(FsmState)]` can only be applied to enums",
            ));
        }
    };

    // Validate: only unit variants allowed.
    for variant in &data_enum.variants {
        match &variant.fields {
            Fields::Unit => {}
            _ => {
                return Err(syn::Error::new(
                    variant.span(),
                    "`#[derive(FsmState)]` only supports unit variants (no fields). \
                     Tuple and struct variants are not supported.",
                ));
            }
        }
    }

    // Generate `as_key` match arms.
    let as_key_arms = data_enum.variants.iter().map(|v| {
        let ident = &v.ident;
        let key = ident.to_string();
        quote! { #name::#ident => #key }
    });

    // Generate `from_key` match arms.
    let from_key_arms = data_enum.variants.iter().map(|v| {
        let ident = &v.ident;
        let key = ident.to_string();
        quote! { #key => ::std::option::Option::Some(#name::#ident) }
    });

    Ok(quote! {
        #[automatically_derived]
        impl #impl_generics ::ferogram::fsm::FsmState
            for #name #ty_generics
            #where_clause
        {
            fn as_key(&self) -> ::std::string::String {
                match self {
                    #(#as_key_arms),*
                }
                .to_string()
            }

            fn from_key(key: &str) -> ::std::option::Option<Self> {
                match key {
                    #(#from_key_arms),*
                    _ => ::std::option::Option::None,
                }
            }
        }
    })
}
