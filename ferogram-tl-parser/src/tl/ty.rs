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

/// The type of a definition or a parameter, e.g. `ns.Vector<!X>`.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Type {
    /// Namespace components, e.g. `["upload"]` for `upload.File`.
    pub namespace: Vec<String>,

    /// The bare type name, e.g. `"Vector"`.
    pub name: String,

    /// `true` when the first letter of the name is lowercase (bare type).
    pub bare: bool,

    /// `true` when this type is a generic parameter reference (prefixed with `!`).
    pub generic_ref: bool,

    /// The generic argument, e.g. `long` in `Vector<long>`.
    pub generic_arg: Option<Box<Type>>,
}

impl fmt::Display for Type {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for ns in &self.namespace {
            write!(f, "{ns}.")?;
        }
        if self.generic_ref {
            write!(f, "!")?;
        }
        write!(f, "{}", self.name)?;
        if let Some(arg) = &self.generic_arg {
            write!(f, "<{arg}>")?;
        }
        Ok(())
    }
}

impl Type {
    /// Collect all nested generic references into `output`.
    pub(crate) fn collect_generic_refs<'a>(&'a self, output: &mut Vec<&'a str>) {
        if self.generic_ref {
            output.push(&self.name);
        }
        if let Some(arg) = &self.generic_arg {
            arg.collect_generic_refs(output);
        }
    }
}

impl FromStr for Type {
    type Err = ParamParseError;

    /// Parses a TL type expression such as `ns.Vector<!X>`.
    ///
    /// # Examples
    /// ```
    /// use ferogram_tl_parser::tl::Type;
    /// assert!("Vector<long>".parse::<Type>().is_ok());
    /// assert!("!X".parse::<Type>().is_ok());
    /// ```
    fn from_str(raw: &str) -> Result<Self, Self::Err> {
        // Strip leading `!` → generic reference
        let (raw, generic_ref) = match raw.strip_prefix('!') {
            Some(r) => (r, true),
            None => (raw, false),
        };

        // Split off `<generic_arg>`
        let (name_part, generic_arg) = match raw.split_once('<') {
            Some((name, rest)) => match rest.strip_suffix('>') {
                Some(arg) => (name, Some(Box::new(Type::from_str(arg)?))),
                None => return Err(ParamParseError::InvalidGeneric),
            },
            None => (raw, None),
        };

        // Split namespace from name
        let (namespace, name) = match name_part.rsplit_once('.') {
            Some((ns_part, n)) => (ns_part.split('.').map(String::from).collect::<Vec<_>>(), n),
            None => (Vec::new(), name_part),
        };

        if namespace.iter().any(|p| p.is_empty()) {
            return Err(ParamParseError::Empty);
        }

        let first = name.chars().next().ok_or(ParamParseError::Empty)?;
        let bare = first.is_ascii_lowercase();

        Ok(Self {
            namespace,
            name: name.to_owned(),
            bare,
            generic_ref,
            generic_arg,
        })
    }
}
