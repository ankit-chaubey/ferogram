// Copyright (c) Ankit Chaubey <ankitchaubey.dev@gmail.com>
// SPDX-License-Identifier: MIT OR Apache-2.0
//
// ferogram: async Telegram MTProto client in Rust
// https://github.com/ankit-chaubey/ferogram
//
// If you use or modify this code, keep this notice at the top of your file
// and include the LICENSE-MIT or LICENSE-APACHE file from this repository:
// https://github.com/ankit-chaubey/ferogram

use std::fmt;
use std::str::FromStr;

use crate::errors::ParamParseError;
use crate::tl::{Flag, Type};

/// The kind of a single TL parameter.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum ParameterType {
    /// A flags field (`name:#`). Its value is computed from the optional
    /// parameters at serialization time, not stored directly.
    Flags,

    /// A regular typed parameter, optionally guarded by a flag bit.
    Normal {
        /// The Rust type that this parameter maps to.
        ty: Type,
        /// If `Some`, this parameter only exists when the given flag bit is set.
        flag: Option<Flag>,
    },
}

impl fmt::Display for ParameterType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Flags => write!(f, "#"),
            Self::Normal { ty, flag } => {
                if let Some(fl) = flag {
                    write!(f, "{}.{}?", fl.name, fl.index)?;
                }
                write!(f, "{ty}")
            }
        }
    }
}

impl FromStr for ParameterType {
    type Err = ParamParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        // Bare flags field
        if s == "#" {
            return Ok(Self::Flags);
        }

        // Possibly `flagname.N?Type`
        if let Some((flag_part, ty_part)) = s.split_once('?') {
            // flag_part must be `name.N`
            let (flag_name, flag_idx) = flag_part
                .split_once('.')
                .ok_or(ParamParseError::InvalidFlag)?;

            if flag_name.is_empty() || flag_idx.is_empty() {
                return Err(ParamParseError::InvalidFlag);
            }

            let index = flag_idx
                .parse::<u32>()
                .map_err(|_| ParamParseError::InvalidFlag)?;

            let ty = Type::from_str(ty_part)?;
            return Ok(Self::Normal {
                ty,
                flag: Some(Flag {
                    name: flag_name.to_owned(),
                    index,
                }),
            });
        }

        // Plain type: must not contain `?`
        if s.contains('?') {
            return Err(ParamParseError::InvalidFlag);
        }

        Ok(Self::Normal {
            ty: Type::from_str(s)?,
            flag: None,
        })
    }
}
