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

//! Build script: parse `.tl` schema files and generate Rust source code.
//!
//! Adding support for a new layer is as simple as dropping a new `.tl` file
//! into `tl/` and bumping the `LAYER` constant: the rest is automatic.

use std::env;
use std::fs;
use std::io;
use std::path::PathBuf;

use ferogram_tl_gen::{Config, Outputs, generate};
use ferogram_tl_parser::{parse_tl_file, tl::Definition};

fn main() -> io::Result<()> {
    let out_dir = env::var("OUT_DIR").expect("OUT_DIR not set");

    // Collect schema files
    let mut all_defs: Vec<Definition> = Vec::new();
    let mut layer: i32 = 0;

    let schemas: &[(&str, bool, bool)] = &[
        // (path,         feature:tl-api, feature:tl-mtproto)
        ("tl/api.tl", true, false),
        ("tl/mtproto.tl", false, true),
    ];

    for (path, api_feature, mtproto_feature) in schemas {
        let enabled = (*api_feature && cfg!(feature = "tl-api"))
            || (*mtproto_feature && cfg!(feature = "tl-mtproto"));
        if !enabled {
            continue;
        }

        let content =
            fs::read_to_string(path).unwrap_or_else(|e| panic!("Cannot read {path}: {e}"));

        // Cargo rebuild trigger
        println!("cargo:rerun-if-changed={path}");

        // Extract `// LAYER N` from anywhere in the file
        for line in content.lines() {
            if let Some(rest) = line.strip_prefix("// LAYER ")
                && let Ok(n) = rest.trim().parse::<i32>()
            {
                layer = layer.max(n);
                break;
            }
        }

        for result in parse_tl_file(&content) {
            match result {
                Ok(def) => all_defs.push(def),
                Err(e) => eprintln!("cargo:warning=TL parse error in {path}: {e}"),
            }
        }
    }

    // Build config from features
    let config = Config {
        gen_name_for_id: cfg!(feature = "name-for-id"),
        deserializable_functions: cfg!(feature = "deserializable-functions"),
        impl_debug: cfg!(feature = "impl-debug"),
        impl_from_type: cfg!(feature = "impl-from-type"),
        impl_from_enum: cfg!(feature = "impl-from-enum"),
        impl_serde: cfg!(feature = "impl-serde"),
    };

    // Generate code
    let mut outputs = Outputs::from_dir(&out_dir)?;
    generate(&all_defs, &config, &mut outputs)?;
    outputs.flush()?;

    // Patch the LAYER constant into generated_common.rs
    let common_path = PathBuf::from(&out_dir).join("generated_common.rs");
    let common = fs::read_to_string(&common_path)?;
    let patched = common.replace(
        "pub const LAYER: i32 = 0; // update via build.rs",
        &format!("pub const LAYER: i32 = {layer};"),
    );
    fs::write(&common_path, patched)?;

    Ok(())
}
