// Copyright (c) Ankit Chaubey <ankitchaubey.dev@gmail.com>
// SPDX-License-Identifier: MIT OR Apache-2.0
//
// ferogram: async Telegram MTProto client in Rust
// https://github.com/ankit-chaubey/ferogram
//
// If you use or modify this code, keep this notice at the top of your file
// and include the LICENSE-MIT or LICENSE-APACHE file from this repository:
// https://github.com/ankit-chaubey/ferogram

#![cfg_attr(docsrs, feature(doc_cfg))]
#![doc(html_root_url = "https://docs.rs/ferogram-tl-types/0.3.3")]
//! Auto-generated Telegram API types, functions, and enums for TL Layer 224.
//!
//! This crate is part of [ferogram](https://crates.io/crates/ferogram), an async Rust
//! MTProto client built by [Ankit Chaubey](https://github.com/ankit-chaubey).
//!
//! - Channel: [t.me/Ferogram](https://t.me/Ferogram)
//! - Chat: [t.me/FerogramChat](https://t.me/FerogramChat)
//!
//! The entire contents are generated at build time from the TL schema in `tl/`.
//! Do not edit the generated source by hand.
//!
//! Most users access this through `ferogram::tl`, which re-exports everything
//! here. Use this crate directly only if you are building on top of the raw TL
//! layer without the high-level `ferogram` client.
//!
//! # Modules
//!
//! | Module | Contents |
//! |---|---|
//! | [`types`] | Concrete constructors as `struct`s (bare types) |
//! | [`functions`] | RPC functions as `struct`s implementing [`RemoteCall`] |
//! | [`enums`] | Boxed types as `enum`s implementing [`Deserializable`] |
//!
//! # Serialization
//!
//! Every type in [`types`] and every function in [`functions`] implements
//! [`Serializable`]. Every enum in [`enums`] implements [`Deserializable`].
//!
//! ```rust,no_run
//! use ferogram_tl_types::{functions, Serializable};
//!
//! let req = functions::help::GetConfig {};
//! let bytes = req.to_bytes();
//! // bytes is the TL-serialized wire form, ready to send over MTProto.
//! ```
//!
//! # Feature flags
//!
//! | Flag | Effect |
//! |---|---|
//! | `tl-api` | Layer 224 API schema types (default in `ferogram`) |
//! | `tl-mtproto` | MTProto internal types (DH, transport, etc.) |
//! | `name-for-id` | `name_for_id(u32) -> &'static str` for debug printing |
//!
//! # Updating to a new TL layer
//!
//! Replace `tl/api.tl` with the new schema and run `cargo build`.
//! The build script regenerates all source. The `LAYER` constant reflects
//! the current layer number.

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
