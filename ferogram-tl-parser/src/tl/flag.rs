// Copyright (c) Ankit Chaubey <ankitchaubey.dev@gmail.com>
// SPDX-License-Identifier: MIT OR Apache-2.0
//
// ferogram: async Telegram MTProto client in Rust
// https://github.com/ankit-chaubey/ferogram
//
// If you use or modify this code, keep this notice at the top of your file
// and include the LICENSE-MIT or LICENSE-APACHE file from this repository:
// https://github.com/ankit-chaubey/ferogram

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
/// A conditional-field flag reference: the flags field name and the bit index.
pub struct Flag {
    /// The name of the flags field that holds this bit (usually `"flags"`).
    pub name: String,
    /// The bit index (0-based).
    pub index: u32,
}
