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

//! The public code-generation API.

use std::fs::File;
use std::io::{self, Write};
use std::path::Path;

use ferogram_tl_parser::tl::{Category, Definition, ParameterType};

use crate::grouper;
use crate::metadata::Metadata;
use crate::namegen as n;

/// Generation configuration.
pub struct Config {
    /// Emit `name_for_id(id) -> Option<&'static str>` in the common module.
    pub gen_name_for_id: bool,
    /// Also implement `Deserializable` for function types (useful for servers).
    pub deserializable_functions: bool,
    /// Derive `Debug` on all generated types.
    pub impl_debug: bool,
    /// Emit `From<types::Foo> for enums::Bar` impls.
    pub impl_from_type: bool,
    /// Emit `TryFrom<enums::Bar> for types::Foo` impls.
    pub impl_from_enum: bool,
    /// Derive `serde::{Serialize, Deserialize}` on all types.
    pub impl_serde: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            gen_name_for_id: false,
            deserializable_functions: false,
            impl_debug: true,
            impl_from_type: true,
            impl_from_enum: true,
            impl_serde: false,
        }
    }
}

// Outputs

/// Writers for each generated Rust module.
pub struct Outputs<W: Write> {
    /// Receives the layer constant, `name_for_id`, etc.
    pub common: W,
    /// Receives `pub mod types { ... }` (concrete constructors as structs).
    pub types: W,
    /// Receives `pub mod functions { ... }` (RPC functions as structs).
    pub functions: W,
    /// Receives `pub mod enums { ... }` (boxed types as enums).
    pub enums: W,
}

impl Outputs<File> {
    /// Convenience constructor that opens files inside `out_dir`.
    pub fn from_dir(out_dir: &str) -> io::Result<Self> {
        let p = Path::new(out_dir);
        Ok(Self {
            common: File::create(p.join("generated_common.rs"))?,
            types: File::create(p.join("generated_types.rs"))?,
            functions: File::create(p.join("generated_functions.rs"))?,
            enums: File::create(p.join("generated_enums.rs"))?,
        })
    }
}

impl<W: Write> Outputs<W> {
    /// Flush all writers.
    pub fn flush(&mut self) -> io::Result<()> {
        self.common.flush()?;
        self.types.flush()?;
        self.functions.flush()?;
        self.enums.flush()
    }
}

// Special-cased primitives

/// These TL types are handled as Rust primitives; we never emit structs/enums.
const BUILTIN_TYPES: &[&str] = &["Bool", "True"];

fn is_builtin(ty_name: &str) -> bool {
    BUILTIN_TYPES.contains(&ty_name)
}

// Public API

/// Generate Rust source code from a slice of parsed TL definitions.
///
/// Write results into `outputs`. Call `outputs.flush()` when done.
pub fn generate<W: Write>(
    defs: &[Definition],
    config: &Config,
    outputs: &mut Outputs<W>,
) -> io::Result<()> {
    let meta = Metadata::build(defs);

    write_common(defs, config, &mut outputs.common)?;
    write_types_mod(defs, config, &meta, &mut outputs.types)?;
    write_functions_mod(defs, config, &meta, &mut outputs.functions)?;
    write_enums_mod(defs, config, &meta, &mut outputs.enums)?;

    Ok(())
}

// Common module

fn write_common<W: Write>(defs: &[Definition], config: &Config, out: &mut W) -> io::Result<()> {
    // Extract LAYER constant from the first `// LAYER N` comment heuristic
    // for now we derive it from the highest layer seen in definitions or emit 0.
    writeln!(out, "// @generated: do not edit by hand")?;
    writeln!(out, "// Re-run the build script to regenerate.")?;
    writeln!(out)?;
    writeln!(out, "/// The API layer this code was generated from.")?;
    writeln!(out, "pub const LAYER: i32 = 0; // update via build.rs")?;
    writeln!(out)?;

    if config.gen_name_for_id {
        writeln!(out, "/// Returns the TL name for a known constructor ID.")?;
        writeln!(
            out,
            "pub fn name_for_id(id: u32) -> Option<&'static str> {{"
        )?;
        writeln!(out, "    match id {{")?;
        for def in defs {
            writeln!(
                out,
                "        {:#010x} => Some(\"{}\"),",
                def.id,
                def.full_name()
            )?;
        }
        writeln!(out, "        _ => None,")?;
        writeln!(out, "    }}")?;
        writeln!(out, "}}")?;
    }

    Ok(())
}

// Struct generation (types + functions)

fn write_types_mod<W: Write>(
    defs: &[Definition],
    config: &Config,
    meta: &Metadata,
    out: &mut W,
) -> io::Result<()> {
    writeln!(out, "// @generated: do not edit by hand")?;
    writeln!(out, "pub mod types {{")?;

    let grouped = grouper::group_by_ns(defs, Category::Types);
    let mut namespaces: Vec<&String> = grouped.keys().collect();
    namespaces.sort();

    for ns in namespaces {
        let bucket = &grouped[ns];
        let indent = if ns.is_empty() {
            "    ".to_owned()
        } else {
            writeln!(out, "    pub mod {ns} {{")?;
            "        ".to_owned()
        };

        for def in bucket {
            write_struct(out, &indent, def, meta, config)?;
            write_identifiable(out, &indent, def)?;
            write_struct_serializable(out, &indent, def, meta)?;
            write_struct_deserializable(out, &indent, def)?;
        }

        if !ns.is_empty() {
            writeln!(out, "    }}")?;
        }
    }

    writeln!(out, "}}")
}

fn write_functions_mod<W: Write>(
    defs: &[Definition],
    config: &Config,
    meta: &Metadata,
    out: &mut W,
) -> io::Result<()> {
    writeln!(out, "// @generated: do not edit by hand")?;
    writeln!(out, "pub mod functions {{")?;

    let grouped = grouper::group_by_ns(defs, Category::Functions);
    let mut namespaces: Vec<&String> = grouped.keys().collect();
    namespaces.sort();

    for ns in namespaces {
        let bucket = &grouped[ns];
        let indent = if ns.is_empty() {
            "    ".to_owned()
        } else {
            writeln!(out, "    pub mod {ns} {{")?;
            "        ".to_owned()
        };

        for def in bucket {
            write_struct(out, &indent, def, meta, config)?;
            write_identifiable(out, &indent, def)?;
            write_struct_serializable(out, &indent, def, meta)?;
            if config.deserializable_functions {
                write_struct_deserializable(out, &indent, def)?;
            }
            write_remote_call(out, &indent, def)?;
        }

        if !ns.is_empty() {
            writeln!(out, "    }}")?;
        }
    }

    writeln!(out, "}}")
}

// Struct pieces

fn generic_list(def: &Definition, bounds: &str) -> String {
    let mut params: Vec<&str> = Vec::new();
    for p in &def.params {
        if let ParameterType::Normal { ty, .. } = &p.ty
            && ty.generic_ref
            && !params.contains(&ty.name.as_str())
        {
            params.push(&ty.name);
        }
    }
    if params.is_empty() {
        String::new()
    } else {
        format!("<{}>", params.join(&format!("{bounds}, ")) + bounds)
    }
}

fn write_struct<W: Write>(
    out: &mut W,
    indent: &str,
    def: &Definition,
    _meta: &Metadata,
    config: &Config,
) -> io::Result<()> {
    let kind = match def.category {
        Category::Types => "constructor",
        Category::Functions => "method",
    };
    writeln!(
        out,
        "\n{indent}/// [`{name}`](https://core.telegram.org/{kind}/{name})\n\
         {indent}///\n\
         {indent}/// Generated from:\n\
         {indent}/// ```tl\n\
         {indent}/// {def}\n\
         {indent}/// ```",
        name = def.full_name(),
    )?;

    if config.impl_debug {
        writeln!(out, "{indent}#[derive(Debug)]")?;
    }
    if config.impl_serde {
        writeln!(
            out,
            "{indent}#[derive(serde::Serialize, serde::Deserialize)]"
        )?;
    }
    writeln!(out, "{indent}#[derive(Clone, PartialEq)]")?;
    writeln!(
        out,
        "{indent}pub struct {}{} {{",
        n::def_type_name(def),
        generic_list(def, ""),
    )?;

    for param in &def.params {
        match &param.ty {
            ParameterType::Flags => {} // computed on-the-fly
            ParameterType::Normal { .. } => {
                writeln!(
                    out,
                    "{indent}    pub {}: {},",
                    n::param_attr_name(param),
                    n::param_qual_name(param),
                )?;
            }
        }
    }
    writeln!(out, "{indent}}}")
}

fn write_identifiable<W: Write>(out: &mut W, indent: &str, def: &Definition) -> io::Result<()> {
    let gl = generic_list(def, "");
    writeln!(
        out,
        "{indent}impl{gl} crate::Identifiable for {}{gl} {{\n\
         {indent}    const CONSTRUCTOR_ID: u32 = {:#010x};\n\
         {indent}}}",
        n::def_type_name(def),
        def.id,
    )
}

fn write_struct_serializable<W: Write>(
    out: &mut W,
    indent: &str,
    def: &Definition,
    meta: &Metadata,
) -> io::Result<()> {
    let gl_decl = generic_list(def, ": crate::Serializable");
    let gl_use = generic_list(def, "");

    writeln!(
        out,
        "{indent}impl{gl_decl} crate::Serializable for {}{gl_use} {{",
        n::def_type_name(def)
    )?;

    let underscore = if def.category == Category::Types && def.params.is_empty() {
        "_"
    } else {
        ""
    };
    writeln!(
        out,
        "{indent}    fn serialize(&self, {underscore}buf: &mut impl Extend<u8>) {{"
    )?;

    if def.category == Category::Functions {
        writeln!(out, "{indent}        use crate::Identifiable;")?;
        writeln!(out, "{indent}        Self::CONSTRUCTOR_ID.serialize(buf);")?;
    }

    for param in &def.params {
        write_param_serialization(out, indent, def, meta, param)?;
    }

    writeln!(out, "{indent}    }}")?;
    writeln!(out, "{indent}}}")
}

fn write_param_serialization<W: Write>(
    out: &mut W,
    indent: &str,
    def: &Definition,
    meta: &Metadata,
    param: &ferogram_tl_parser::tl::Parameter,
) -> io::Result<()> {
    use ParameterType::*;

    match &param.ty {
        Flags => {
            if meta.is_unused_flag(def, param) {
                writeln!(out, "{indent}        0u32.serialize(buf);")?;
                return Ok(());
            }
            // Compute the flags bitmask from optional params
            write!(out, "{indent}        (")?;
            let mut first = true;
            for other in &def.params {
                if let Normal {
                    flag: Some(fl), ty, ..
                } = &other.ty
                {
                    if fl.name != param.name {
                        continue;
                    }
                    if !first {
                        write!(out, " | ")?;
                    }
                    first = false;
                    if ty.name == "true" {
                        write!(
                            out,
                            "if self.{} {{ 1 << {} }} else {{ 0 }}",
                            n::param_attr_name(other),
                            fl.index
                        )?;
                    } else {
                        write!(
                            out,
                            "if self.{}.is_some() {{ 1 << {} }} else {{ 0 }}",
                            n::param_attr_name(other),
                            fl.index
                        )?;
                    }
                }
            }
            if first {
                write!(out, "0u32")?;
            }
            writeln!(out, ").serialize(buf);")?;
        }
        Normal { ty, flag } => {
            let attr = n::param_attr_name(param);
            if flag.is_some() {
                if ty.name == "true" {
                    // bool flag: nothing to serialize, it's in the flags word
                } else {
                    writeln!(
                        out,
                        "{indent}        if let Some(v) = &self.{attr} {{ v.serialize(buf); }}"
                    )?;
                }
            } else {
                writeln!(out, "{indent}        self.{attr}.serialize(buf);")?;
            }
        }
    }
    Ok(())
}

fn write_struct_deserializable<W: Write>(
    out: &mut W,
    indent: &str,
    def: &Definition,
) -> io::Result<()> {
    let gl_decl = generic_list(def, ": crate::Deserializable");
    let gl_use = generic_list(def, "");

    // Empty structs never read from `buf`. Name it `_buf` to suppress the
    // unused-variable warning in the generated output.
    let buf_name = if def.params.is_empty() { "_buf" } else { "buf" };

    writeln!(
        out,
        "{indent}impl{gl_decl} crate::Deserializable for {}{gl_use} {{",
        n::def_type_name(def)
    )?;
    writeln!(
        out,
        "{indent}    fn deserialize({buf_name}: crate::deserialize::Buffer) -> crate::deserialize::Result<Self> {{"
    )?;

    // Read flags first so optional params can check them
    let flag_params: Vec<_> = def
        .params
        .iter()
        .filter(|p| p.ty == ParameterType::Flags)
        .collect();

    for fp in &flag_params {
        writeln!(
            out,
            "{indent}        let _{} = u32::deserialize(buf)?;",
            n::param_attr_name(fp)
        )?;
    }

    // Now deserialize each non-flag param
    for param in &def.params {
        if param.ty == ParameterType::Flags {
            continue; // already done above
        }
        if let ParameterType::Normal { ty, flag } = &param.ty {
            let attr = n::param_attr_name(param);
            if let Some(fl) = flag {
                if ty.name == "true" {
                    writeln!(
                        out,
                        "{indent}        let {attr} = (_{} & (1 << {})) != 0;",
                        fl.name, fl.index
                    )?;
                } else {
                    writeln!(
                        out,
                        "{indent}        let {attr} = if (_{} & (1 << {})) != 0 {{ Some({}::deserialize(buf)?) }} else {{ None }};",
                        fl.name,
                        fl.index,
                        n::type_item_path(ty)
                    )?;
                }
            } else {
                writeln!(
                    out,
                    "{indent}        let {attr} = {}::deserialize(buf)?;",
                    n::type_item_path(ty)
                )?;
            }
        }
    }

    writeln!(out, "{indent}        Ok(Self {{")?;
    for param in &def.params {
        if param.ty != ParameterType::Flags {
            let attr = n::param_attr_name(param);
            writeln!(out, "{indent}            {attr},")?;
        }
    }
    writeln!(out, "{indent}        }})")?;
    writeln!(out, "{indent}    }}")?;
    writeln!(out, "{indent}}}")
}

fn write_remote_call<W: Write>(out: &mut W, indent: &str, def: &Definition) -> io::Result<()> {
    // Generic functions (e.g. invokeWithLayer<X>) need the type parameter on
    // the impl header and on the struct name, just like every other write_* helper.
    let gl_decl = generic_list(def, ": crate::Serializable + crate::Deserializable");
    let gl_use = generic_list(def, "");
    writeln!(
        out,
        "{indent}impl{gl_decl} crate::RemoteCall for {}{gl_use} {{",
        n::def_type_name(def)
    )?;
    writeln!(
        out,
        "{indent}    type Return = {};",
        n::type_qual_name(&def.ty)
    )?;
    writeln!(out, "{indent}}}")
}

// Enum generation

fn write_enums_mod<W: Write>(
    defs: &[Definition],
    config: &Config,
    meta: &Metadata,
    out: &mut W,
) -> io::Result<()> {
    writeln!(out, "// @generated: do not edit by hand")?;
    writeln!(out, "pub mod enums {{")?;

    let grouped = grouper::group_types_by_ns(defs);
    let mut keys: Vec<&Option<String>> = grouped.keys().collect();
    keys.sort();

    for key in keys {
        let types = &grouped[key];
        let indent = if let Some(ns) = key {
            writeln!(out, "    pub mod {ns} {{")?;
            "        ".to_owned()
        } else {
            "    ".to_owned()
        };

        for ty in types.iter().filter(|t| !is_builtin(&t.name)) {
            write_enum(out, &indent, ty, meta, config)?;
            write_enum_serializable(out, &indent, ty, meta)?;
            write_enum_deserializable(out, &indent, ty, meta)?;
            if config.impl_from_type {
                write_impl_from(out, &indent, ty, meta)?;
            }
            if config.impl_from_enum {
                write_impl_try_from(out, &indent, ty, meta)?;
            }
        }

        if key.is_some() {
            writeln!(out, "    }}")?;
        }
    }

    writeln!(out, "}}")
}

fn write_enum<W: Write>(
    out: &mut W,
    indent: &str,
    ty: &ferogram_tl_parser::tl::Type,
    meta: &Metadata,
    config: &Config,
) -> io::Result<()> {
    writeln!(
        out,
        "\n{indent}/// [`{name}`](https://core.telegram.org/type/{name})",
        name = n::type_name(ty)
    )?;
    if config.impl_debug {
        writeln!(out, "{indent}#[derive(Debug)]")?;
    }
    if config.impl_serde {
        writeln!(
            out,
            "{indent}#[derive(serde::Serialize, serde::Deserialize)]"
        )?;
    }
    writeln!(out, "{indent}#[derive(Clone, PartialEq)]")?;
    writeln!(out, "{indent}pub enum {} {{", n::type_name(ty))?;

    for def in meta.defs_for_type(ty) {
        let variant = n::def_variant_name(def);
        if def.params.is_empty() {
            writeln!(out, "{indent}    {variant},")?;
        } else if meta.is_recursive(def) {
            writeln!(
                out,
                "{indent}    {variant}(Box<{}>),",
                n::def_qual_name(def)
            )?;
        } else {
            writeln!(out, "{indent}    {variant}({}),", n::def_qual_name(def))?;
        }
    }

    writeln!(out, "{indent}}}")
}

fn write_enum_serializable<W: Write>(
    out: &mut W,
    indent: &str,
    ty: &ferogram_tl_parser::tl::Type,
    meta: &Metadata,
) -> io::Result<()> {
    writeln!(
        out,
        "{indent}impl crate::Serializable for {} {{",
        n::type_name(ty)
    )?;
    writeln!(
        out,
        "{indent}    fn serialize(&self, buf: &mut impl Extend<u8>) {{"
    )?;
    writeln!(out, "{indent}        use crate::Identifiable;")?;
    writeln!(out, "{indent}        match self {{")?;

    for def in meta.defs_for_type(ty) {
        let variant = n::def_variant_name(def);
        let bind = if def.params.is_empty() { "" } else { "(x)" };
        writeln!(out, "{indent}            Self::{variant}{bind} => {{")?;
        writeln!(
            out,
            "{indent}                {}::CONSTRUCTOR_ID.serialize(buf);",
            n::def_qual_name(def)
        )?;
        if !def.params.is_empty() {
            writeln!(out, "{indent}                x.serialize(buf);")?;
        }
        writeln!(out, "{indent}            }}")?;
    }

    writeln!(out, "{indent}        }}")?;
    writeln!(out, "{indent}    }}")?;
    writeln!(out, "{indent}}}")
}

fn write_enum_deserializable<W: Write>(
    out: &mut W,
    indent: &str,
    ty: &ferogram_tl_parser::tl::Type,
    meta: &Metadata,
) -> io::Result<()> {
    writeln!(
        out,
        "{indent}impl crate::Deserializable for {} {{",
        n::type_name(ty)
    )?;
    writeln!(
        out,
        "{indent}    fn deserialize(buf: crate::deserialize::Buffer) -> crate::deserialize::Result<Self> {{"
    )?;
    writeln!(out, "{indent}        use crate::Identifiable;")?;
    writeln!(out, "{indent}        let id = u32::deserialize(buf)?;")?;
    writeln!(out, "{indent}        Ok(match id {{")?;

    for def in meta.defs_for_type(ty) {
        let variant = n::def_variant_name(def);
        let qual = n::def_qual_name(def);
        if def.params.is_empty() {
            writeln!(
                out,
                "{indent}            {qual}::CONSTRUCTOR_ID => Self::{variant},"
            )?;
        } else if meta.is_recursive(def) {
            writeln!(
                out,
                "{indent}            {qual}::CONSTRUCTOR_ID => Self::{variant}(Box::new({qual}::deserialize(buf)?)),"
            )?;
        } else {
            writeln!(
                out,
                "{indent}            {qual}::CONSTRUCTOR_ID => Self::{variant}({qual}::deserialize(buf)?),"
            )?;
        }
    }

    writeln!(
        out,
        "{indent}            _ => return Err(crate::deserialize::Error::UnexpectedConstructor {{ id }}),"
    )?;
    writeln!(out, "{indent}        }})")?;
    writeln!(out, "{indent}    }}")?;
    writeln!(out, "{indent}}}")
}

fn write_impl_from<W: Write>(
    out: &mut W,
    indent: &str,
    ty: &ferogram_tl_parser::tl::Type,
    meta: &Metadata,
) -> io::Result<()> {
    for def in meta.defs_for_type(ty) {
        let enum_name = n::type_name(ty);
        let qual = n::def_qual_name(def);
        let variant = n::def_variant_name(def);

        writeln!(out, "{indent}impl From<{qual}> for {enum_name} {{")?;
        let underscore = if def.params.is_empty() { "_" } else { "" };
        writeln!(out, "{indent}    fn from({underscore}x: {qual}) -> Self {{")?;
        if def.params.is_empty() {
            writeln!(out, "{indent}        Self::{variant}")?;
        } else if meta.is_recursive(def) {
            writeln!(out, "{indent}        Self::{variant}(Box::new(x))")?;
        } else {
            writeln!(out, "{indent}        Self::{variant}(x)")?;
        }
        writeln!(out, "{indent}    }}")?;
        writeln!(out, "{indent}}}")?;
    }
    Ok(())
}

fn write_impl_try_from<W: Write>(
    out: &mut W,
    indent: &str,
    ty: &ferogram_tl_parser::tl::Type,
    meta: &Metadata,
) -> io::Result<()> {
    let enum_name = n::type_name(ty);
    for def in meta.defs_for_type(ty) {
        if def.params.is_empty() {
            continue;
        }
        let qual = n::def_qual_name(def);
        let variant = n::def_variant_name(def);

        writeln!(out, "{indent}impl TryFrom<{enum_name}> for {qual} {{")?;
        writeln!(out, "{indent}    type Error = {enum_name};")?;
        writeln!(out, "{indent}    #[allow(unreachable_patterns)]")?;
        writeln!(
            out,
            "{indent}    fn try_from(v: {enum_name}) -> Result<Self, Self::Error> {{"
        )?;
        writeln!(out, "{indent}        match v {{")?;
        if meta.is_recursive(def) {
            writeln!(
                out,
                "{indent}            {enum_name}::{variant}(x) => Ok(*x),"
            )?;
        } else {
            writeln!(
                out,
                "{indent}            {enum_name}::{variant}(x) => Ok(x),"
            )?;
        }
        writeln!(out, "{indent}            other => Err(other),")?;
        writeln!(out, "{indent}        }}")?;
        writeln!(out, "{indent}    }}")?;
        writeln!(out, "{indent}}}")?;
    }
    Ok(())
}
