// Copyright (c) Ankit Chaubey <ankitchaubey.dev@gmail.com>
// SPDX-License-Identifier: MIT OR Apache-2.0
//
// ferogram: async Telegram MTProto client in Rust
// https://github.com/ankit-chaubey/ferogram
//
//
// If you use or modify this code, keep this notice at the top of your file
// and include the LICENSE-MIT or LICENSE-APACHE file from this repository:
// https://github.com/ankit-chaubey/ferogram

use std::fmt;
use std::str::FromStr;

use crate::errors::{ParamParseError, ParseError};
use crate::tl::{Category, Flag, Parameter, ParameterType, Type};
use crate::utils::tl_id;

/// A single TL definition: either a constructor or a function.
///
/// For example:
/// ```text
/// user#12345 id:long first_name:string = User;
/// ```
/// becomes a `Definition` with `name = "user"`, `id = 0x12345`,
/// `params = [id:long, first_name:string]` and `ty = User`.
#[derive(Clone, Debug, PartialEq)]
pub struct Definition {
    /// Namespace parts.  Empty when the definition is in the global namespace.
    pub namespace: Vec<String>,

    /// The constructor/method name (e.g. `"user"`, `"messages.sendMessage"`).
    pub name: String,

    /// 32-bit constructor ID, either parsed from `#XXXXXXXX` or CRC32-derived.
    pub id: u32,

    /// Ordered list of parameters.
    pub params: Vec<Parameter>,

    /// The boxed type this definition belongs to (e.g. `User`).
    pub ty: Type,

    /// Whether this is a data constructor or an RPC function.
    pub category: Category,
}

impl Definition {
    /// Returns `namespace.name` joined with dots.
    pub fn full_name(&self) -> String {
        let cap = self.namespace.iter().map(|ns| ns.len() + 1).sum::<usize>() + self.name.len();
        let mut s = String::with_capacity(cap);
        for ns in &self.namespace {
            s.push_str(ns);
            s.push('.');
        }
        s.push_str(&self.name);
        s
    }
}

impl fmt::Display for Definition {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for ns in &self.namespace {
            write!(f, "{ns}.")?;
        }
        write!(f, "{}#{:x}", self.name, self.id)?;

        // Emit any `{X:Type}` generic parameter defs that appear in params
        let mut generics: Vec<&str> = Vec::new();
        for p in &self.params {
            if let ParameterType::Normal { ty, .. } = &p.ty {
                ty.collect_generic_refs(&mut generics);
            }
        }
        generics.sort_unstable();
        generics.dedup();
        for g in generics {
            write!(f, " {{{g}:Type}}")?;
        }

        for p in &self.params {
            write!(f, " {p}")?;
        }
        write!(f, " = {}", self.ty)
    }
}

impl FromStr for Definition {
    type Err = ParseError;

    fn from_str(raw: &str) -> Result<Self, Self::Err> {
        let raw = raw.trim();
        if raw.is_empty() {
            return Err(ParseError::Empty);
        }

        // Split at `=`
        let (lhs, ty_str) = raw.split_once('=').ok_or(ParseError::MissingType)?;
        let lhs = lhs.trim();
        let ty_str = ty_str.trim().trim_end_matches(';').trim();

        if ty_str.is_empty() {
            return Err(ParseError::MissingType);
        }

        let mut ty = Type::from_str(ty_str).map_err(|_| ParseError::MissingType)?;

        // Split head (name + optional id) from parameter tokens
        let (head, rest) = match lhs.split_once(|c: char| c.is_whitespace()) {
            Some((h, r)) => (h.trim_end(), r.trim_start()),
            None => (lhs, ""),
        };

        // Parse optional `#id`
        let (full_name, explicit_id) = match head.split_once('#') {
            Some((n, id)) => (n, Some(id)),
            None => (head, None),
        };

        // Parse namespace
        let (namespace, name) = match full_name.rsplit_once('.') {
            Some((ns_part, n)) => (ns_part.split('.').map(String::from).collect::<Vec<_>>(), n),
            None => (Vec::new(), full_name),
        };

        if namespace.iter().any(|p| p.is_empty()) || name.is_empty() {
            return Err(ParseError::MissingName);
        }

        let id = match explicit_id {
            Some(hex) => u32::from_str_radix(hex.trim(), 16).map_err(ParseError::InvalidId)?,
            None => tl_id(raw),
        };

        // Parse parameters
        let mut type_defs: Vec<String> = Vec::new();
        let mut flag_defs: Vec<String> = Vec::new();

        let params = rest
            .split_whitespace()
            .filter_map(|token| match Parameter::from_str(token) {
                // `{X:Type}` → record the generic name and skip
                Err(ParamParseError::TypeDef { name }) => {
                    type_defs.push(name);
                    None
                }
                Ok(p) => {
                    match &p {
                        Parameter {
                            ty: ParameterType::Flags,
                            ..
                        } => {
                            flag_defs.push(p.name.clone());
                        }
                        // Validate generic ref is declared
                        Parameter {
                            ty:
                                ParameterType::Normal {
                                    ty:
                                        Type {
                                            name: tn,
                                            generic_ref: true,
                                            ..
                                        },
                                    ..
                                },
                            ..
                        } if !type_defs.contains(tn) => {
                            return Some(Err(ParseError::InvalidParam(
                                ParamParseError::MissingDef,
                            )));
                        }
                        // Validate flag field is declared
                        Parameter {
                            ty:
                                ParameterType::Normal {
                                    flag: Some(Flag { name: fn_, .. }),
                                    ..
                                },
                            ..
                        } if !flag_defs.contains(fn_) => {
                            return Some(Err(ParseError::InvalidParam(
                                ParamParseError::MissingDef,
                            )));
                        }
                        _ => {}
                    }
                    Some(Ok(p))
                }
                Err(ParamParseError::NotImplemented) => Some(Err(ParseError::NotImplemented)),
                Err(e) => Some(Err(ParseError::InvalidParam(e))),
            })
            .collect::<Result<Vec<_>, ParseError>>()?;

        // If the return type is itself a declared generic, mark it
        if type_defs.contains(&ty.name) {
            ty.generic_ref = true;
        }

        Ok(Definition {
            namespace,
            name: name.to_owned(),
            id,
            params,
            ty,
            category: Category::Types, // caller sets the real category
        })
    }
}
