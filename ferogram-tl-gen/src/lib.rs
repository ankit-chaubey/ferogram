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
#![doc(html_root_url = "https://docs.rs/ferogram-tl-gen/0.3.3")]
//! Build-time code generator from a parsed TL schema to Rust source files.
//!
//! This crate is part of [ferogram](https://crates.io/crates/ferogram), an async Rust
//! MTProto client built by [Ankit Chaubey](https://github.com/ankit-chaubey).
//!
//! - Channel: [t.me/Ferogram](https://t.me/Ferogram)
//! - Chat: [t.me/FerogramChat](https://t.me/FerogramChat)
//!
//! Use this crate from a `build.rs` script to regenerate `ferogram-tl-types`
//! when you want to update to a new Telegram API layer. You feed it a parsed
//! TL schema (from `ferogram-tl-parser`) and it writes the Rust source for
//! `types`, `functions`, and `enums` modules.
//!
//! # Usage from build.rs
//!
//! ```no_run
//! use ferogram_tl_gen::{Config, Outputs, generate};
//! use ferogram_tl_parser::parse_tl_file;
//! use std::fs;
//!
//! fn main() {
//!     let schema = fs::read_to_string("tl/api.tl").unwrap();
//!     let defs: Vec<_> = parse_tl_file(&schema)
//!         .filter_map(|r| r.ok())
//!         .collect();
//!
//!     let config = Config::default();
//!     let outputs = generate(&defs, &config).unwrap();
//!
//!     fs::write("src/generated.rs", outputs.combined()).unwrap();
//! }
//! ```
//!
//! # What it generates
//!
//! - `types` module: one `struct` per TL bare constructor, with named fields.
//! - `functions` module: one `struct` per TL function, implementing `RemoteCall`.
//! - `enums` module: one `enum` per TL boxed type, with one variant per constructor.
//!
//! All types implement `Serializable`. All enums implement `Deserializable`.
//!
//! Most users never touch this crate. It only matters when you are upgrading
//! the TL layer or maintaining a fork of `ferogram-tl-types`.
mod codegen;
mod grouper;
mod metadata;
mod namegen;

pub use codegen::{Config, Outputs, generate};
