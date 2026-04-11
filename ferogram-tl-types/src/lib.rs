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

#![cfg_attr(docsrs, feature(doc_cfg))]
#![doc(html_root_url = "https://docs.rs/ferogram-tl-types/0.4.6")]
//! Generated Telegram API types, functions and enums.
//!
//! This crate is **auto-generated** from the TL schema files in `tl/`.
//! To update for a new API layer, replace `tl/api.tl` and rebuild.
//!
//! # Overview
//!
//! | Module        | Contents                                                   |
//! |---------------|------------------------------------------------------------|
//! | [`types`]     | Concrete constructors (bare types) as `struct`s            |
//! | [`functions`] | RPC functions as `struct`s implementing [`RemoteCall`]     |
//! | [`enums`]     | Boxed types as `enum`s implementing [`Deserializable`]     |
//!
//! # Raw API usage
//!
//! ```rust,no_run
//! use ferogram_tl_types::{functions, Serializable};
//!
//! let req = functions::auth::SendCode {
//! phone_number: "+1234567890".into(),
//! api_id: 12345,
//! api_hash: "abc".into(),
//! settings: Default::default(),
//! };
//!
//! let bytes = req.to_bytes();
//! // Send `bytes` over an MTProto connection…
//! ```
//!
//! # Updating to a new layer
//!
//! 1. Replace `tl/api.tl` with the new schema.
//! 2. `cargo build`: the build script regenerates everything.

#![deny(unsafe_code)]
#![allow(clippy::large_enum_variant)]

pub mod deserialize;
mod generated;
pub mod serialize;

pub use deserialize::{Cursor, Deserializable};
#[cfg(feature = "name-for-id")]
pub use generated::name_for_id;
pub use generated::{LAYER, enums, functions, types};
pub use serialize::Serializable;

/// Bare vector: `vector` (lowercase) as opposed to the boxed `Vector`.
///
/// Used in rare cases where Telegram sends a length-prefixed list without
/// the usual `0x1cb5c415` constructor ID header.
#[derive(Clone, Debug, PartialEq)]
pub struct RawVec<T>(pub Vec<T>);

/// Opaque blob of bytes that should be passed through without interpretation.
///
/// Returned by functions whose response type is generic (e.g. `X`).
#[derive(Clone, Debug, PartialEq)]
pub struct Blob(pub Vec<u8>);

impl From<Vec<u8>> for Blob {
    fn from(v: Vec<u8>) -> Self {
        Self(v)
    }
}

// Core traits

/// Every generated type has a unique 32-bit constructor ID.
pub trait Identifiable {
    /// The constructor ID as specified in the TL schema.
    const CONSTRUCTOR_ID: u32;
}

/// Marks a function type that can be sent to Telegram as an RPC call.
///
/// `Return` is the type Telegram will respond with.
pub trait RemoteCall: Serializable {
    /// The deserialized response type.
    type Return: Deserializable;
}
