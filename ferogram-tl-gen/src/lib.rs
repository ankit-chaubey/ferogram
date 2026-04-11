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
#![doc(html_root_url = "https://docs.rs/ferogram-tl-gen/0.4.6")]
//! Build-time code generator that transforms a parsed TL schema into Rust source files.
//!
//! Intended to be used from a `build.rs` script.
//!
//! # Usage
//!
//! ```no_run
//! // build.rs
//! use ferogram_tl_gen::{Config, Outputs, generate};
//! use ferogram_tl_parser::parse_tl_file;
//! use std::fs;
//!
//! fn main() {
//! let schema = fs::read_to_string("tl/api.tl").unwrap();
//! let defs: Vec<_> = parse_tl_file(&schema)
//!     .filter_map(|r| r.ok())
//!     .collect();
//!
//! let out = std::env::var("OUT_DIR").unwrap();
//! let mut outputs = Outputs::from_dir(&out).unwrap();
//! generate(&defs, &Config::default(), &mut outputs).unwrap();
//! }
//! ```

#![deny(unsafe_code)]
#![warn(missing_docs)]

pub mod codegen;
mod grouper;
mod metadata;
mod namegen;

pub use codegen::{Config, Outputs, generate};
