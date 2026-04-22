// Copyright (c) Ankit Chaubey <ankitchaubey.dev@gmail.com>
// SPDX-License-Identifier: MIT OR Apache-2.0
//
// ferogram: async Telegram MTProto client in Rust
// https://github.com/ankit-chaubey/ferogram
//
// If you use or modify this code, keep this notice at the top of your file
// and include the LICENSE-MIT or LICENSE-APACHE file from this repository:
// https://github.com/ankit-chaubey/ferogram

use ferogram_tl_parser::tl::{Category, Definition, Parameter, ParameterType, Type};
use std::collections::{HashMap, HashSet};

pub(crate) struct Metadata<'a> {
    /// Definitions that contain themselves (directly or transitively).
    recursive_ids: HashSet<u32>,
    /// Map from (type namespace, type name) → concrete constructors.
    defs_by_type: HashMap<(&'a Vec<String>, &'a String), Vec<&'a Definition>>,
    /// Parameters whose flags bit is never used by any optional parameter.
    unused_flags: HashMap<(&'a Vec<String>, &'a String), Vec<&'a Parameter>>,
}

impl<'a> Metadata<'a> {
    pub(crate) fn build(defs: &'a [Definition]) -> Self {
        let mut meta = Self {
            recursive_ids: HashSet::new(),
            defs_by_type: HashMap::new(),
            unused_flags: HashMap::new(),
        };

        // Build defs_by_type
        for def in defs.iter().filter(|d| d.category == Category::Types) {
            meta.defs_by_type
                .entry((&def.ty.namespace, &def.ty.name))
                .or_default()
                .push(def);
        }

        // Detect unused flags
        for def in defs {
            for flag_param in def.params.iter().filter(|p| p.ty == ParameterType::Flags) {
                let used = def.params.iter().any(|p| {
                    matches!(&p.ty,
                        ParameterType::Normal { flag: Some(f), .. } if f.name == flag_param.name
                    )
                });
                if !used {
                    meta.unused_flags
                        .entry((&def.namespace, &def.name))
                        .or_default()
                        .push(flag_param);
                }
            }
        }

        // Detect recursion
        let type_defs: Vec<&Definition> = defs
            .iter()
            .filter(|d| d.category == Category::Types)
            .collect();

        for def in &type_defs {
            if self_refs(def, def, &meta.defs_by_type, &mut HashSet::new()) {
                meta.recursive_ids.insert(def.id);
            }
        }

        meta
    }

    pub(crate) fn is_recursive(&self, def: &Definition) -> bool {
        self.recursive_ids.contains(&def.id)
    }

    pub(crate) fn defs_for_type(&self, ty: &'a Type) -> &[&Definition] {
        self.defs_by_type
            .get(&(&ty.namespace, &ty.name))
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    pub(crate) fn is_unused_flag(&self, def: &Definition, param: &Parameter) -> bool {
        self.unused_flags
            .get(&(&def.namespace, &def.name))
            .map(|v| v.iter().any(|p| std::ptr::eq(*p, param)))
            .unwrap_or(false)
    }
}

fn self_refs<'a>(
    root: &Definition,
    current: &Definition,
    defs_by_type: &HashMap<(&'a Vec<String>, &'a String), Vec<&'a Definition>>,
    visited: &mut HashSet<u32>,
) -> bool {
    visited.insert(current.id);
    for param in &current.params {
        if let ParameterType::Normal { ty, .. } = &param.ty {
            // Direct self-reference
            if ty.namespace == root.ty.namespace && ty.name == root.ty.name {
                return true;
            }
            // Indirect via another constructor
            if let Some(sub_defs) = defs_by_type.get(&(&ty.namespace, &ty.name)) {
                for sub in sub_defs {
                    if !visited.contains(&sub.id) && self_refs(root, sub, defs_by_type, visited) {
                        return true;
                    }
                }
            }
        }
    }
    false
}
