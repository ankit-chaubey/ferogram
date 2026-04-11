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
use std::str::FromStr;

use crate::errors::ParamParseError;
use crate::tl::ParameterType;

/// A single `name:Type` parameter inside a TL definition.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Parameter {
    /// The parameter name as it appears in the TL schema.
    pub name: String,
    /// The resolved type of this parameter.
    pub ty: ParameterType,
}

impl fmt::Display for Parameter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.name, self.ty)
    }
}

impl FromStr for Parameter {
    type Err = ParamParseError;

    /// Parses a single parameter token such as `flags:#`, `id:long`, or
    /// `photo:flags.0?InputPhoto`.
    ///
    /// Returns `Err(ParamParseError::TypeDef { name })` for the special
    /// `{X:Type}` generic-parameter-definition syntax so callers can handle it
    /// without the overhead of `?`.
    fn from_str(token: &str) -> Result<Self, Self::Err> {
        // Generic type-definition `{X:Type}`: not a real parameter
        if let Some(inner) = token.strip_prefix('{') {
            return Err(match inner.strip_suffix(":Type}") {
                Some(name) => ParamParseError::TypeDef { name: name.into() },
                None => ParamParseError::MissingDef,
            });
        }

        let (name, ty_str) = token
            .split_once(':')
            .ok_or(ParamParseError::NotImplemented)?;

        if name.is_empty() || ty_str.is_empty() {
            return Err(ParamParseError::Empty);
        }

        Ok(Self {
            name: name.to_owned(),
            ty: ty_str.parse()?,
        })
    }
}
