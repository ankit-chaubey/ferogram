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

use ferogram_tl_parser::tl::{Category, Definition, Type};
use std::collections::HashMap;

/// Group definitions of `category` by their (first-level) namespace.
pub(crate) fn group_by_ns(
    defs: &[Definition],
    category: Category,
) -> HashMap<String, Vec<&Definition>> {
    let mut map: HashMap<String, Vec<&Definition>> = HashMap::new();

    for def in defs.iter().filter(|d| d.category == category) {
        assert!(
            def.namespace.len() <= 1,
            "only one namespace level supported"
        );
        let ns = def.namespace.first().map(|s| s.as_str()).unwrap_or("");
        map.entry(ns.to_owned()).or_default().push(def);
    }

    // Sort each bucket alphabetically for deterministic output
    for bucket in map.values_mut() {
        bucket.sort_by_key(|d| &d.name);
    }

    map
}

/// Group the *return types* of constructors by namespace.
/// Used to emit `enum` blocks.
pub(crate) fn group_types_by_ns(defs: &[Definition]) -> HashMap<Option<String>, Vec<&Type>> {
    let mut map: HashMap<Option<String>, Vec<&Type>> = HashMap::new();

    for def in defs
        .iter()
        .filter(|d| d.category == Category::Types && !d.ty.generic_ref)
    {
        assert!(def.namespace.len() <= 1);
        map.entry(def.namespace.first().cloned())
            .or_default()
            .push(&def.ty);
    }

    for bucket in map.values_mut() {
        bucket.sort_by_key(|t| &t.name);
        bucket.dedup_by_key(|t| &t.name);
    }

    map
}
