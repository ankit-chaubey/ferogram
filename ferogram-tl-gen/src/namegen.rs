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

//! Functions that convert TL names to idiomatic Rust identifiers.

use ferogram_tl_parser::tl::{Definition, Parameter, ParameterType, Type};

// primitive → Rust type

/// Map a TL primitive name to a Rust built-in type string, if applicable.
pub(crate) fn builtin_type(name: &str) -> Option<&'static str> {
    Some(match name {
        "Bool" => "bool",
        "true" => "bool",
        "int" => "i32",
        "long" => "i64",
        "double" => "f64",
        "string" => "String",
        "bytes" => "Vec<u8>",
        "int128" => "[u8; 16]",
        "int256" => "[u8; 32]",
        "Vector" => "Vec",
        "vector" => "crate::RawVec",
        _ => return None,
    })
}

// PascalCase conversion

/// Converts `some_ok_name` or `SomeOKName` into `SomeOkName` (PascalCase).
pub(crate) fn to_pascal(name: &str) -> String {
    // Strip leading namespace if present
    let name = if let Some(pos) = name.rfind('.') {
        &name[pos + 1..]
    } else {
        name
    };

    let mut out = String::with_capacity(name.len());
    let mut next_upper = true;
    let mut prev_upper = false;

    for ch in name.chars() {
        if ch == '_' {
            next_upper = true;
            prev_upper = false;
            continue;
        }
        if next_upper {
            // Forced capitalisation (start of string or after `_`).
            out.push(ch.to_ascii_uppercase());
            next_upper = false;
            // If the source char was already uppercase we are entering a
            // cap-run (e.g. the 'O' in `some_OK_name`); set prev_upper so
            // subsequent caps get lowercased.  If it was lowercase we just
            // started a normal word, so prev_upper stays false.
            prev_upper = ch.is_ascii_uppercase();
        } else if ch.is_ascii_uppercase() {
            if prev_upper {
                // Continuation of a cap-run (e.g. 'K' in "OK") → lowercase
                // so "someOKName" → "SomeOkName" not "SomeOKName".
                out.push(ch.to_ascii_lowercase());
            } else {
                // camelCase word boundary (e.g. 'P' in "inputPeer") → keep
                // uppercase: "inputPeerSelf" → "InputPeerSelf".
                out.push(ch);
            }
            prev_upper = true;
        } else {
            out.push(ch);
            prev_upper = false;
        }
    }
    out
}

// Definition helpers

/// `struct` / `fn` name for a definition (PascalCase).
pub(crate) fn def_type_name(def: &Definition) -> String {
    to_pascal(&def.name)
}

/// Fully-qualified `crate::types::ns::Name` path for a definition.
pub(crate) fn def_qual_name(def: &Definition) -> String {
    let mut s = String::from("crate::types::");
    for ns in &def.namespace {
        s.push_str(ns);
        s.push_str("::");
    }
    s.push_str(&def_type_name(def));
    s
}

/// Enum variant name derived from the definition (strips the return type prefix).
pub(crate) fn def_variant_name(def: &Definition) -> String {
    let full = def_type_name(def);
    let ty = type_name(&def.ty);

    let variant = if full.starts_with(&ty) {
        &full[ty.len()..]
    } else {
        &full
    };

    match variant {
        // `Self` is a reserved keyword
        "Self" => {
            let pos = full.as_bytes()[..full.len() - variant.len()]
                .iter()
                .rposition(|c| c.is_ascii_uppercase())
                .unwrap_or(0);
            full[pos..].to_owned()
        }
        // All-numeric suffix: use from last uppercase
        v if !v.is_empty() && v.chars().all(char::is_numeric) => {
            let pos = full
                .as_bytes()
                .iter()
                .rposition(|c| c.is_ascii_uppercase())
                .unwrap_or(0);
            full[pos..].to_owned()
        }
        // Empty: fall back to full name
        "" => full,
        v => v.to_owned(),
    }
}

// Type helpers

/// PascalCase name for a TL type.
pub(crate) fn type_name(ty: &Type) -> String {
    to_pascal(&ty.name)
}

/// Fully-qualified Rust type path, e.g. `crate::enums::InputPeer` or `Vec<i64>`.
pub(crate) fn type_qual_name(ty: &Type) -> String {
    type_path(ty, false)
}

/// Same as `type_qual_name` but uses `::<…>` turbofish syntax.
pub(crate) fn type_item_path(ty: &Type) -> String {
    type_path(ty, true)
}

fn type_path(ty: &Type, turbofish: bool) -> String {
    if ty.generic_ref {
        return ty.name.clone();
    }

    let mut s = if let Some(b) = builtin_type(&ty.name) {
        // When emitting a turbofish path (for method calls like `Vec::<u8>::deserialize`),
        // two classes of builtin need special treatment:
        //
        // 1. Builtins that already carry a generic arg baked into the string
        //  (e.g. `"Vec<u8>"`): insert `::` before the `<`.
        //  Without it rustc parses `Vec<u8>::` as a comparison expression.
        //
        // 2. Array builtins (`"[u8; 16]"`, `"[u8; 32]"`): the type is not a
        //  named path, so `[u8; 16]::deserialize` is a hard syntax error.
        //  Wrap in `<…>` to get `<[u8; 16]>::deserialize`.
        if turbofish {
            if b.starts_with('[') {
                // Array type: wrap in angle brackets for path syntax.
                format!("<{b}>")
            } else if let Some(pos) = b.find('<') {
                // Generic builtin: insert `::` before the `<`.
                let mut out = b[..pos].to_owned();
                out.push_str("::");
                out.push_str(&b[pos..]);
                out
            } else {
                b.to_owned()
            }
        } else {
            b.to_owned()
        }
    } else if ty.bare {
        let mut p = String::from("crate::types::");
        for ns in &ty.namespace {
            p.push_str(ns);
            p.push_str("::");
        }
        p.push_str(&type_name(ty));
        p
    } else {
        let mut p = String::from("crate::enums::");
        for ns in &ty.namespace {
            p.push_str(ns);
            p.push_str("::");
        }
        p.push_str(&type_name(ty));
        p
    };

    if let Some(arg) = &ty.generic_arg {
        if turbofish {
            s.push_str("::");
        }
        s.push('<');
        s.push_str(&type_qual_name(arg));
        s.push('>');
    }

    s
}

// Parameter helpers

/// The Rust attribute name for a parameter (handles reserved keywords).
pub(crate) fn param_attr_name(param: &Parameter) -> String {
    match param.name.as_str() {
        "final" => "r#final".into(),
        "loop" => "r#loop".into(),
        "self" => "is_self".into(),
        "static" => "r#static".into(),
        "type" => "r#type".into(),
        other => other.to_ascii_lowercase(),
    }
}

/// The full Rust type expression for a parameter, e.g. `Option<i32>`.
pub(crate) fn param_qual_name(param: &Parameter) -> String {
    match &param.ty {
        ParameterType::Flags => "u32".into(),
        ParameterType::Normal { ty, flag } => {
            // `flags.N?true` → `bool`
            if flag.is_some() && ty.name == "true" {
                return "bool".into();
            }
            let inner = type_qual_name(ty);
            if flag.is_some() {
                format!("Option<{inner}>")
            } else {
                inner
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pascal_basic() {
        assert_eq!(to_pascal("user_empty"), "UserEmpty");
        assert_eq!(to_pascal("inputPeerSelf"), "InputPeerSelf");
        assert_eq!(to_pascal("some_OK_name"), "SomeOkName");
    }

    #[test]
    fn pascal_namespaced() {
        assert_eq!(to_pascal("upload.fileCdnRedirect"), "FileCdnRedirect");
    }
}
