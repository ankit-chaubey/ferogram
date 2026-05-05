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

use ferogram_tl_parser::{parse_tl_file, tl::Category};

#[test]
fn parses_simple_type() {
    let src = "boolFalse#bc799737 = Bool;";
    let defs: Vec<_> = parse_tl_file(src).collect::<Result<_, _>>().unwrap();
    assert_eq!(defs.len(), 1);
    assert_eq!(defs[0].name, "boolFalse");
    assert_eq!(defs[0].id, 0xbc799737);
    assert_eq!(defs[0].ty.name, "Bool");
}

#[test]
fn parses_function_category() {
    let src = "
---functions---
help.getConfig#c4f9186b = Config;
";
    let defs: Vec<_> = parse_tl_file(src).collect::<Result<_, _>>().unwrap();
    assert_eq!(defs.len(), 1);
    assert_eq!(defs[0].category, Category::Functions);
    assert_eq!(defs[0].name, "getConfig");
    assert_eq!(defs[0].namespace, vec!["help"]);
}

#[test]
fn parses_flagged_parameter() {
    let src = "user#3ff6ecb0 flags:# id:long username:flags.0?string = User;";
    let defs: Vec<_> = parse_tl_file(src).collect::<Result<_, _>>().unwrap();
    assert_eq!(defs.len(), 1);
    assert_eq!(defs[0].params.len(), 3); // flags, id, username
}

#[test]
fn skips_blank_lines_and_comments() {
    let src = "
// this is a comment
boolTrue#997275b5 = Bool;
// another comment

boolFalse#bc799737 = Bool;
";
    let defs: Vec<_> = parse_tl_file(src).collect::<Result<_, _>>().unwrap();
    assert_eq!(defs.len(), 2);
}

#[test]
fn crc32_derived_id() {
    // boolFalse#bc799737: omit #id, parser must derive same value via CRC32
    let src = "boolFalse = Bool;";
    let defs: Vec<_> = parse_tl_file(src).collect::<Result<_, _>>().unwrap();
    assert_eq!(defs[0].id, 0xbc799737);
}
