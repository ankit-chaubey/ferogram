// Copyright (c) Ankit Chaubey <ankitchaubey.dev@gmail.com>
//
// ferogram: async Telegram MTProto client in Rust
// https://github.com/ankit-chaubey/ferogram
//
// Licensed under either the MIT License or the Apache License 2.0.
// See the LICENSE-MIT or LICENSE-APACHE file in this repository:
// https://github.com/ankit-chaubey/ferogram
//
// Feel free to use, modify, and share this code.
// Please keep this notice when redistributing.

#![deny(unsafe_code)]
#![cfg_attr(docsrs, feature(doc_cfg))]
#![doc(html_root_url = "https://docs.rs/ferogram-tl-gen/0.3.7")]
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
//!     let mut outputs = Outputs {
//!         common:    Vec::new(),
//!         types:     Vec::new(),
//!         functions: Vec::new(),
//!         enums:     Vec::new(),
//!     };
//!     generate(&defs, &config, &mut outputs).unwrap();
//!
//!     let mut combined = outputs.common;
//!     combined.extend(outputs.types);
//!     combined.extend(outputs.functions);
//!     combined.extend(outputs.enums);
//!     fs::write("src/generated.rs", combined).unwrap();
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
