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

use std::fmt;
use std::num::ParseIntError;

/// Errors produced while parsing a single parameter token.
#[derive(Clone, Debug, PartialEq)]
pub enum ParamParseError {
    /// An empty string was encountered where a name/type was expected.
    Empty,
    /// A `{X:Type}` generic type definition (not a real error; used as a signal).
    TypeDef {
        /// The name of the generic type parameter (e.g. `"X"` from `{X:Type}`).
        name: String,
    },
    /// A `{...}` block that isn't a valid type definition.
    MissingDef,
    /// A flag expression (`name.N?Type`) was malformed.
    InvalidFlag,
    /// A generic `<...>` argument was malformed (missing closing `>`).
    InvalidGeneric,
    /// A bare `name` with no `:type`: e.g. old-style `? = Int`.
    NotImplemented,
}

impl fmt::Display for ParamParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => write!(f, "empty token"),
            Self::TypeDef { name } => write!(f, "generic type definition: {name}"),
            Self::MissingDef => write!(f, "unknown generic or flag definition"),
            Self::InvalidFlag => write!(f, "invalid flag expression"),
            Self::InvalidGeneric => write!(f, "invalid generic argument (unclosed `<`)"),
            Self::NotImplemented => write!(f, "parameter without `:type` is not supported"),
        }
    }
}

impl std::error::Error for ParamParseError {}

/// Errors produced while parsing a complete TL definition.
#[derive(Debug, PartialEq)]
pub enum ParseError {
    /// The input was blank.
    Empty,
    /// No `= Type` was found.
    MissingType,
    /// The name (before `=`) was missing or had empty namespace components.
    MissingName,
    /// The `#id` hex literal was unparseable.
    InvalidId(ParseIntError),
    /// A parameter was invalid.
    InvalidParam(ParamParseError),
    /// The definition uses a syntax we don't support yet.
    NotImplemented,
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => write!(f, "empty definition"),
            Self::MissingType => write!(f, "missing `= Type`"),
            Self::MissingName => write!(f, "missing or malformed name"),
            Self::InvalidId(e) => write!(f, "invalid constructor ID: {e}"),
            Self::InvalidParam(e) => write!(f, "invalid parameter: {e}"),
            Self::NotImplemented => write!(f, "unsupported TL syntax"),
        }
    }
}

impl std::error::Error for ParseError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::InvalidId(e) => Some(e),
            Self::InvalidParam(e) => Some(e),
            _ => None,
        }
    }
}
