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
#![doc(html_root_url = "https://docs.rs/ferogram-tl-parser/0.4.6")]
//! Parser for Telegram's [Type Language] (TL) schema files.
//!
//! This crate converts raw `.tl` text into a structured [`Definition`] AST
//! which can then be used by code-generators (see `ferogram-tl-gen`).
//!
//! # Quick start
//!
//! ```rust
//! use ferogram_tl_parser::parse_tl_file;
//!
//! let src = "user#12345 id:long name:string = User;";
//! for def in parse_tl_file(src) {
//! println!("{:#?}", def.unwrap());
//! }
//! ```
//!
//! [Type Language]: https://core.telegram.org/mtproto/TL

#![deny(unsafe_code)]
#![warn(missing_docs)]

/// Parse error types for TL schema parsing.
pub mod errors;
mod iterator;
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
