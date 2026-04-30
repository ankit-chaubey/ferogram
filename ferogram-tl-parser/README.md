# ferogram-tl-parser

Parser for Telegram's TL (Type Language) schema files.

[![Crates.io](https://img.shields.io/crates/v/ferogram-tl-parser?color=fc8d62)](https://crates.io/crates/ferogram-tl-parser)
[![Telegram](https://img.shields.io/badge/community-%40FerogramChat-2CA5E0?logo=telegram)](https://t.me/FerogramChat) [![Channel](https://img.shields.io/badge/channel-%40Ferogram-2CA5E0?logo=telegram)](https://t.me/Ferogram)
[![docs.rs](https://img.shields.io/badge/docs.rs-ferogram--tl--parser-5865F2)](https://docs.rs/ferogram-tl-parser)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](#license)

Reads `.tl` schema files and produces a structured AST. Used by `ferogram-tl-gen` as a build-dependency  most users don't need to depend on this crate directly.

---

## Installation

```toml
[dependencies]
ferogram-tl-parser = "0.3.6"
```

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

MIT or Apache-2.0, at your option. See [LICENSE-MIT](../LICENSE-MIT) and [LICENSE-APACHE](../LICENSE-APACHE).

**Ankit Chaubey** - [github.com/ankit-chaubey](https://github.com/ankit-chaubey)
