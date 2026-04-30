# ferogram-tl-types

Auto-generated Rust types for all Telegram API Layer 224 constructors, functions, and enums.

[![Crates.io](https://img.shields.io/crates/v/ferogram-tl-types?color=fc8d62)](https://crates.io/crates/ferogram-tl-types)
[![Telegram](https://img.shields.io/badge/community-%40FerogramChat-2CA5E0?logo=telegram)](https://t.me/FerogramChat) [![Channel](https://img.shields.io/badge/channel-%40Ferogram-2CA5E0?logo=telegram)](https://t.me/Ferogram)
[![docs.rs](https://img.shields.io/badge/docs.rs-ferogram--tl--types-5865F2)](https://docs.rs/ferogram-tl-types)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](#license)
[![TL Layer](https://img.shields.io/badge/TL%20Layer-224-8b5cf6)](https://core.telegram.org/schema)

2,329 TL definitions generated at build time from Layer 224. All constructors, functions, and abstract types are represented as Rust structs and enums with binary TL serialization and deserialization.

---

## Installation

```toml
[dependencies]
ferogram-tl-types = "0.3.6"

# With MTProto low-level types (required by ferogram-mtproto):
ferogram-tl-types = { version = "0.3.6", features = ["tl-mtproto"] }
```

---

## Generated Structure

```rust
// TL constructors become structs
pub mod types {
    pub struct Message {
        pub id:      i32,
        pub peer_id: enums::Peer,
        pub message: String,
        // optional fields wrapped in Option<>
    }
}

// Abstract TL types become enums
pub mod enums {
    pub enum Message {
        Message(types::Message),
        Service(types::MessageService),
        Empty(types::MessageEmpty),
    }
}

// TL functions become structs implementing RemoteCall
pub mod functions {
    pub mod messages {
        pub struct SendMessage { /* fields */ }
        impl RemoteCall for SendMessage {
            type Return = enums::Updates;
        }
    }
}
```

---

## Feature Flags

| Feature | Default | Description |
|---|---|---|
| `tl-api` | yes | Telegram API schema (`api.tl`) |
| `tl-mtproto` | no | MTProto internal schema (`mtproto.tl`) |
| `impl-debug` | yes | `#[derive(Debug)]` on all types |
| `impl-from-type` | yes | `From<types::T> for enums::E` |
| `impl-from-enum` | yes | `TryFrom<enums::E> for types::T` |
| `deserializable-functions` | no | `Deserializable` on function types |
| `name-for-id` | no | `name_for_id(u32) -> Option<&'static str>` |
| `impl-serde` | no | `serde::Serialize` / `Deserialize` |

---

## Updating the TL Schema

```bash
cp new-api.tl ferogram-tl-types/tl/api.tl
cargo build
```

`ferogram-tl-gen` regenerates all types at compile time via `build.rs`. No manual code changes needed.

---

## Stack position

```
ferogram
└ ferogram-mtproto
  └ ferogram-tl-types  <-- here
    └ (build) ferogram-tl-gen
      └ (build) ferogram-tl-parser
```

---

## License

MIT or Apache-2.0, at your option. See [LICENSE-MIT](../LICENSE-MIT) and [LICENSE-APACHE](../LICENSE-APACHE).

**Ankit Chaubey** - [github.com/ankit-chaubey](https://github.com/ankit-chaubey)
