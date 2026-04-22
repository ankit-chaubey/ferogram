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
#![doc(html_root_url = "https://docs.rs/ferogram-tl-parser/0.3.3")]
//! Parser for Telegram's Type Language (TL) schema files.
//!
//! This crate is part of [ferogram](https://crates.io/crates/ferogram), an async Rust
//! MTProto client built by [Ankit Chaubey](https://github.com/ankit-chaubey).
//!
//! - Channel: [t.me/Ferogram](https://t.me/Ferogram)
//! - Chat: [t.me/FerogramChat](https://t.me/FerogramChat)
//!
//! Converts raw `.tl` schema text into a [`tl::Definition`] AST. That AST is
//! then consumed by `ferogram-tl-gen` to produce the Rust type bindings in
//! `ferogram-tl-types`.
//!
//! Most users never touch this crate. It matters when you are upgrading the
//! TL layer, writing a custom code generator, or doing schema introspection.
//!
//! # Usage
//!
//! ```rust
//! use ferogram_tl_parser::parse_tl_file;
//!
//! let src = "user#12345678 id:long name:string = User;";
//! for def in parse_tl_file(src) {
//!     let def = def.unwrap();
//!     println!("{} #{:08x}", def.name, def.id);
//! }
//! ```
//!
//! [`parse_tl_file`] returns an iterator of `Result<Definition, ParseError>`.
//! It handles both constructor and function sections of a `.tl` file.
//!
//! # AST
//!
//! The main types are in the [`tl`] module:
//! - [`tl::Definition`]: one parsed TL declaration (constructor or function).
//! - [`tl::Parameter`]: a field name and type.
//! - [`tl::Type`]: a TL type reference (with optional generic argument).
//!
//! [Type Language]: https://core.telegram.org/mtproto/TL
#![deny(unsafe_code)]
#![warn(missing_docs)]

/// Parse error types for TL schema parsing.
pub mod errors;
mod iterator;
/// Core TL schema types: definitions, parameters, types, flags, and categories.
pub mod tl;
mod utils;

use errors::ParseError;
use tl::Definition;

/// Parses a complete TL schema file, yielding [`Definition`]s one by one.
///
/// Lines starting with `//` are treated as comments and skipped.
/// The special `---functions---` and `---types---` section markers switch
/// the [`tl::Category`] applied to the following definitions.
///
/// Returns an iterator of `Result<Definition, ParseError>` so callers can
/// decide whether to skip or hard-fail on bad lines.
pub fn parse_tl_file(contents: &str) -> impl Iterator<Item = Result<Definition, ParseError>> + '_ {
    iterator::TlIterator::new(contents)
}
