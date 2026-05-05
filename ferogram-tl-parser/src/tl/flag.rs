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

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
/// A conditional-field flag reference: the flags field name and the bit index.
pub struct Flag {
    /// The name of the flags field that holds this bit (usually `"flags"`).
    pub name: String,
    /// The bit index (0-based).
    pub index: u32,
}
