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

//! Rust types representing the Abstract Syntax Tree of a TL definition.

mod category;
mod definition;
mod flag;
mod parameter;
mod parameter_type;
mod ty;

pub use category::Category;
pub use definition::Definition;
pub use flag::Flag;
pub use parameter::Parameter;
pub use parameter_type::ParameterType;
pub use ty::Type;
