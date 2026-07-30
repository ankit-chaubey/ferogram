/*
 * Copyright (c) 2026 Ankit Chaubey <ankitchaubey.dev@gmail.com>
 * https://github.com/ankit-chaubey
 *
 * Project: ferogram
 * Website: https://ferogram.dev
 *
 * Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
 * https://www.apache.org/licenses/LICENSE-2.0> or the MIT license
 * <LICENSE-MIT or https://opensource.org/licenses/MIT>, at your option.
 * This file may not be copied, modified, or distributed except according
 * to those terms.
 */

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
