/*
 * Copyright (c) 2026 Ankit Chaubey <ankitchaubey.dev@gmail.com>
 * https://github.com/ankit-chaubey
 *
 * Project: ferogram
 * Website: https://ferogram.dev
 *
 * Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
 * https://www.apache.org/licenses/LICENSE-2.0> or the MIT license
 * <LICENSE-MIT or https://opensource.org/licenses/MIT>, at your option.
 * This file may not be copied, modified, or distributed except according
 * to those terms.
 */

#![cfg_attr(docsrs, feature(doc_cfg))]
#![doc(html_root_url = "https://docs.rs/ferogram-derive/0.6.5")]
//! Procedural macros for ferogram.
//!
//! This crate is part of [ferogram](https://crates.io/crates/ferogram), an async Rust
//! MTProto client built by [Ankit Chaubey](https://github.com/ankit-chaubey).
//!
//! - Channel: [t.me/Ferogram](https://t.me/Ferogram)
//! - Chat: [t.me/FerogramChat](https://t.me/FerogramChat)
//!
//! You do not depend on this crate directly. It is re-exported through
//! `ferogram` and `ferogram-fsm`. Add those crates to your `Cargo.toml`
//! instead.
//!
//! # What's in here
//!
//! - **`#[derive(FsmState)]`**: Implements the `ferogram_fsm::FsmState`
//!   trait for a unit-variant enum. Generates `as_key` (module path + enum
//!   name + variant name → `String`) and `from_key` (string → `Option<Self>`,
//!   with a fallback for keys written by older versions of this macro).
//!   Tuple/struct variants and generic enums are rejected at compile time.
//!
//! # Example
//!
//! ```rust,ignore
//! use ferogram::FsmState;
//!
//! #[derive(FsmState, Clone, Debug, PartialEq)]
//! enum CheckoutState {
//!     Cart,
//!     Address,
//!     Payment,
//!     Confirmation,
//! }
//! ```

#![deny(unsafe_code)]

use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::quote;
use syn::{Data, DeriveInput, Fields, parse_macro_input, spanned::Spanned};

/// Derive the `ferogram_fsm::FsmState` trait for an enum.
///
/// Only **unit variants** (no fields) are supported. Tuple or struct variants
/// are rejected with a compile error. **Generic enums are also rejected**:
/// the key can't disambiguate different type parameter instantiations, so
/// this fails to compile rather than silently colliding at runtime.
///
/// # What gets generated
///
/// - `as_key(&self) -> String` - returns `"module::path::EnumName::Variant"`,
///   namespaced by full module path and enum name so identically-named
///   variants -- even on identically-named enums in different modules --
///   don't collide.
/// - `from_key(key: &str) -> Option<Self>` - parses that key back into the
///   enum. Falls back to matching on the trailing `"::"`-segment so state
///   written by older versions of this derive (bare `"Variant"` or
///   `"EnumName::Variant"`) still deserializes after an upgrade, on a
///   best-effort basis.
///
/// # Example
///
/// ```rust,ignore
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

    // Reject generics outright. A generic enum like `State<T>` would need the
    // key to disambiguate by concrete `T` as well, which `module_path!()` +
    // enum/variant name cannot do (it's resolved once, at the enum's
    // declaration site, not per-monomorphization). Rather than silently
    // producing colliding keys for `State<Deposit>` vs `State<Withdraw>`,
    // refuse to compile so the gap is visible instead of a runtime bug.
    if !input.generics.params.is_empty() {
        return Err(syn::Error::new(
            input.generics.span(),
            "`#[derive(FsmState)]` does not support generic enums. \
             The generated key cannot disambiguate different type parameter \
             instantiations (e.g. `State<Deposit>` vs `State<Withdraw>` would \
             collide). Define separate concrete enums instead.",
        ));
    }

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
    //
    // Keys are namespaced as "module::path::EnumName::Variant" using
    // `module_path!()`, resolved at the enum's declaration site. This
    // disambiguates not just same-named variants on differently-named enums
    // (DepositState::AwaitingAmount vs WithdrawState::AwaitingAmount), but
    // also identically-named enums declared in different modules
    // (deposit::State::AwaitingAmount vs withdraw::State::AwaitingAmount).
    let as_key_arms = data_enum.variants.iter().map(|v| {
        let ident = &v.ident;
        quote! {
            #name::#ident => ::std::concat!(
                ::std::module_path!(), "::", ::std::stringify!(#name), "::", ::std::stringify!(#ident)
            )
        }
    });

    // Generate `from_key` match arms for the current, fully-qualified format.
    let from_key_arms = data_enum.variants.iter().map(|v| {
        let ident = &v.ident;
        quote! {
            ::std::concat!(
                ::std::module_path!(), "::", ::std::stringify!(#name), "::", ::std::stringify!(#ident)
            ) => ::std::option::Option::Some(#name::#ident)
        }
    });

    // Legacy fallback arms, matched against just the variant name (the
    // segment after the last "::"). This covers keys written by older
    // versions of this macro: bare `"Variant"` (pre-namespacing) and
    // `"EnumName::Variant"` (namespaced but without the module path). Those
    // older formats were themselves ambiguous across enums/modules that
    // shared a name -- this is a best-effort migration path so existing
    // persisted state doesn't just vanish across an upgrade, not a
    // guarantee that old, already-colliding data resolves correctly.
    let legacy_from_key_arms = data_enum.variants.iter().map(|v| {
        let ident = &v.ident;
        let key = ident.to_string();
        quote! { #key => ::std::option::Option::Some(#name::#ident) }
    });

    Ok(quote! {
        #[automatically_derived]
        impl #impl_generics ::ferogram::FsmState
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
                    #(#from_key_arms,)*
                    _ => {
                        // Not the current fully-qualified format. Fall back
                        // to matching on the last "::"-delimited segment so
                        // state written by older versions of this derive
                        // still deserializes instead of being dropped.
                        let short = key.rsplit("::").next().unwrap_or(key);
                        match short {
                            #(#legacy_from_key_arms,)*
                            _ => ::std::option::Option::None,
                        }
                    }
                }
            }
        }
    })
}
