// Copyright (c) Ankit Chaubey <ankitchaubey.dev@gmail.com>
// SPDX-License-Identifier: MIT OR Apache-2.0
//
// ferogram: async Telegram MTProto client in Rust
// https://github.com/ankit-chaubey/ferogram
//
// If you use or modify this code, keep this notice at the top of your file
// and include the LICENSE-MIT or LICENSE-APACHE file from this repository:
// https://github.com/ankit-chaubey/ferogram

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
/// Whether a TL definition is a data constructor or an RPC function.
pub enum Category {
    /// A concrete data constructor (the section before `---functions---`).
    Types,
    /// An RPC function definition (the section after `---functions---`).
    Functions,
}
