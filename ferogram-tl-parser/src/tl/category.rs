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

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
/// Whether a TL definition is a data constructor or an RPC function.
pub enum Category {
    /// A concrete data constructor (the section before `---functions---`).
    Types,
    /// An RPC function definition (the section after `---functions---`).
    Functions,
}
