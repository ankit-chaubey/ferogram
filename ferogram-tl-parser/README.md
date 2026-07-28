# ferogram-tl-parser

Parser for Telegram's TL (Type Language) schema files.

[![Crates.io](https://img.shields.io/crates/v/ferogram-tl-parser?style=flat-square&logo=rust&logoColor=white&color=F97316)](https://crates.io/crates/ferogram-tl-parser)
[![Telegram Channel](https://img.shields.io/badge/Channel-Ferogram-06B6D4?style=flat-square&logo=telegram&logoColor=white)](https://t.me/Ferogram) [![Telegram Chat](https://img.shields.io/badge/Chat-FerogramChat-06B6D4?style=flat-square&logo=telegram&logoColor=white)](https://t.me/FerogramChat)
[![docs.rs](https://img.shields.io/badge/docs.rs-ferogram--tl--parser-5865F2?style=flat-square&logo=docs.rs&logoColor=white)](https://docs.rs/ferogram-tl-parser)
[![License](https://img.shields.io/badge/License-MIT%20%7C%20Apache--2.0-64748B?style=flat-square)](#license)

Reads `.tl` schema files and produces a structured AST. Used by `ferogram-tl-gen` as a build-dependency. Most users won't need to depend on this directly.

For installation instructions see the [ferogram README](https://github.com/ankit-chaubey/ferogram).

---

## AST Types

```rust
pub struct Definition {
    pub name:     String,
    pub id:       Option<u32>,    // CRC32, may be omitted
    pub params:   Vec<Parameter>,
    pub ty:       Type,
    pub category: Category,       // Type or Function
}

pub enum ParameterType {
    Flags,
    Normal { ty: Type, flag: Option<Flag> },
    Repeated { params: Vec<Parameter> },
}

pub enum Category { Type, Function }
```

---

## Usage

```rust
use ferogram_tl_parser::{parse_tl_file, TlIterator, tl::Category};

// Collect all definitions
let schema = std::fs::read_to_string("api.tl").unwrap();
let definitions = parse_tl_file(&schema).unwrap();

// Streaming iterator (lower memory)
for def in TlIterator::new(&schema) {
    match def.category {
        Category::Type     => { /* constructor */ }
        Category::Function => { /* RPC function */ }
    }
}
```

Parse errors return `ParseError` with the failing line. Malformed tokens stop the iterator rather than silently skipping.

---

## Stack position

```
ferogram-tl-types
└ ferogram-tl-gen
  └ ferogram-tl-parser  <-- here
```

---

## License

This project is licensed under either the MIT License or Apache License 2.0, at your option. See [`LICENSE-MIT`](https://github.com/ankit-chaubey/ferogram/blob/main/LICENSE-MIT) and [`LICENSE-APACHE`](https://github.com/ankit-chaubey/ferogram/blob/main/LICENSE-APACHE) for details.

**Author:** Ankit Chaubey ([@ankit-chaubey](https://github.com/ankit-chaubey))
