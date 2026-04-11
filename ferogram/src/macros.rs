// Copyright (c) Ankit Chaubey <ankitchaubey.dev@gmail.com>
// SPDX-License-Identifier: MIT OR Apache-2.0
//
// ferogram: async Telegram MTProto client in Rust
// https://github.com/ankit-chaubey/ferogram
//
// Based on layer: https://github.com/ankit-chaubey/layer
// Follows official Telegram client behaviour (tdesktop, TDLib).
//
// If you use or modify this code, keep this notice at the top of your file
// and include the LICENSE-MIT or LICENSE-APACHE file from this repository:
// https://github.com/ankit-chaubey/ferogram

//! The [`dispatch!`] macro for pattern-matching over updates.
//!
//! Instead of writing giant `match` blocks, `dispatch!` lets you register
//! named handlers with optional guard clauses:
//!
//! ```rust,no_run
//! use ferogram::{Client, dispatch};
//! use ferogram::update::Update;
//!
//! # async fn example(client: Client, update: Update) -> Result<(), Box<dyn std::error::Error>> {
//! dispatch!(client, update,
//! NewMessage(msg) if !msg.outgoing() => {
//!     println!("Got: {:?}", msg.text());
//! },
//! MessageEdited(msg) => {
//!     println!("Edited: {:?}", msg.text());
//! },
//! CallbackQuery(cb) => {
//!     client.answer_callback_query(cb.query_id, Some("✓"), false).await?;
//! },
//! _ => {} // catch-all for unhandled variants
//! );
//! # Ok(()) }
//! ```
//!
//! Each arm is `VariantName(binding) [if guard] => { body }`.
//! The macro expands to a plain `match` statement: zero overhead.

/// Route a [`crate::update::Update`] to the first matching arm.
///
/// # Syntax
/// ```text
/// dispatch!(client, update,
/// VariantName(binding) [if guard] => { body },
/// ...
/// [_ => { fallback }]
/// );
/// ```
///
/// - `client` : a `ferogram::Client` (available inside every arm body)
/// - `update` : the `Update` value to dispatch
/// - Each arm mirrors a variant of [`crate::update::Update`]
/// - Guards (`if expr`) are optional
/// - A catch-all `_ => {}` arm is optional but recommended to avoid warnings
#[macro_export]
macro_rules! dispatch {
    // Entry point: client, update, then one or more arms
    ($client:expr, $update:expr, $( $pattern:tt )+ ) => {
        match $update {
            $crate::__dispatch_arms!($client; $( $pattern )+ )
        }
    };
}

/// Internal helper: do not use directly.
#[macro_export]
#[doc(hidden)]
macro_rules! __dispatch_arms {
    // Catch-all arm
    ($client:expr; _ => $body:block $( , $( $rest:tt )* )? ) => {
        _ => $body
    };

    // Variant arm WITH guard
    ($client:expr;
        $variant:ident ( $binding:pat ) if $guard:expr => $body:block
        $( , $( $rest:tt )* )?
    ) => {
        $crate::update::Update::$variant($binding) if $guard => $body,
        $( $crate::__dispatch_arms!($client; $( $rest )* ) )?
    };

    // Variant arm WITHOUT guard
    ($client:expr;
        $variant:ident ( $binding:pat ) => $body:block
        $( , $( $rest:tt )* )?
    ) => {
        $crate::update::Update::$variant($binding) => $body,
        $( $crate::__dispatch_arms!($client; $( $rest )* ) )?
    };

    // Trailing comma / empty: emit wildcard to make sure exhaustiveness
    ($client:expr; $(,)?) => {
        _ => {}
    };
}
