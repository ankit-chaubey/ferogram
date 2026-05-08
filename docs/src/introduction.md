# ferogram

[![Crates.io](https://img.shields.io/crates/v/ferogram?color=fc8d62&label=ferogram&style=flat-square)](https://crates.io/crates/ferogram)
[![docs.rs](https://img.shields.io/docsrs/ferogram?style=flat-square&color=22c55e)](https://docs.rs/ferogram)
[![TL Layer](https://img.shields.io/badge/TL%20Layer-225-8b5cf6?style=flat-square)](https://core.telegram.org/schema)
[![License](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue?style=flat-square)](#license)

ferogram is an async Rust library for Telegram's MTProto protocol. It works for user accounts and bots, and talks to Telegram directly over MTProto with no Bot API HTTP proxy in the middle.

Everything in the workspace is written from scratch: the `.tl` schema parser, the TL code generator, AES-IGE crypto, the DH key exchange, MTProto framing, the session layer, and the high-level client on top. Each piece is its own crate so you can pull in just what you need.

## Why it exists

I was already using other MTProto libraries and kept running into cases where I needed things to work a bit differently than they allowed. So I wrote my own.

The goal was to cover the major use cases first, and it does. If something's missing for you, drop by [t.me/FerogramChat](https://t.me/FerogramChat). I genuinely like hearing what people are building with it.

## Crates

Most people only need `ferogram`. But each crate is independently published if you need a specific layer on its own.

| Crate | What it does |
|---|---|
| [`ferogram`](https://docs.rs/ferogram) | High-level client. Auth, messaging, media, dispatcher, FSM, middleware. |
| [`ferogram-session`](https://docs.rs/ferogram-session) | Session types and pluggable storage backends (file, memory, SQLite, LibSQL, base64). |
| [`ferogram-fsm`](https://docs.rs/ferogram-fsm) | FSM state storage and context. `StateStorage` trait, `MemoryStorage`, `StateContext`. |
| [`ferogram-parsers`](https://docs.rs/ferogram-parsers) | Telegram Markdown and HTML entity parsers. |
| [`ferogram-derive`](https://docs.rs/ferogram-derive) | `#[derive(FsmState)]` proc macro. |
| [`ferogram-mtsender`](https://docs.rs/ferogram-mtsender) | DC connection pool and retry policy. `AutoSleep`, `NoRetries`, `CircuitBreaker`. |
| [`ferogram-connect`](https://docs.rs/ferogram-connect) | Raw TCP, MTProto framing, obfuscation, SOCKS5, MTProxy, gzip. |
| [`ferogram-mtproto`](https://docs.rs/ferogram-mtproto) | MTProto 2.0 session, DH key exchange, message framing, PFS key binding. |
| [`ferogram-crypto`](https://docs.rs/ferogram-crypto) | AES-IGE, RSA, SHA, Diffie-Hellman, PQ factorization, auth key derivation. |
| [`ferogram-tl-types`](https://docs.rs/ferogram-tl-types) | Auto-generated TL types, functions, and enums for Layer 225. |
| [`ferogram-tl-gen`](https://docs.rs/ferogram-tl-gen) | Build-time code generator from TL AST to Rust source. |
| [`ferogram-tl-parser`](https://docs.rs/ferogram-tl-parser) | Parses `.tl` schema text into a Definition AST. |

The rough dependency chain:

```
ferogram
└ ferogram-mtsender
  └ ferogram-connect
    ├ ferogram-mtproto
    │ ├ ferogram-tl-types
    │ │ └ (build) ferogram-tl-gen
    │ │   └ (build) ferogram-tl-parser
    │ └ ferogram-crypto
    └ ferogram-crypto
```

## Quick install

```toml
[dependencies]
ferogram = "0.3.8"
tokio    = { version = "1", features = ["full"] }
```

Get `api_id` and `api_hash` from [my.telegram.org](https://my.telegram.org). That's all you need to get started.

## Where to go next

- [Installation](./installation.md) covers credentials, optional feature flags, and session backends.
- [Quick Start: Bot](./quickstart-bot.md) gets a bot running in about 20 lines.
- [Quick Start: User Account](./quickstart-user.md) covers phone login and sending your first message.
- [Crate Architecture](./crates.md) if you want to understand how the pieces fit together.

## Python

Python support is live via [ferogram-py](https://github.com/ankit-chaubey/ferogram-py). Pre-built wheels, no Rust toolchain needed.

```bash
pip install ferogram
```

## Community

- **Channel** (releases, announcements): [t.me/Ferogram](https://t.me/Ferogram)
- **Chat** (questions, discussion): [t.me/FerogramChat](https://t.me/FerogramChat)
- **API docs**: [docs.rs/ferogram](https://docs.rs/ferogram)
- **GitHub**: [github.com/ankit-chaubey/ferogram](https://github.com/ankit-chaubey/ferogram)

## License

MIT OR Apache-2.0.

Usage must comply with [Telegram's API Terms of Service](https://core.telegram.org/api/terms).
