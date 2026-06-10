# ferogram

[![Crates.io](https://img.shields.io/crates/v/ferogram?color=fc8d62&label=ferogram&style=flat-square)](https://crates.io/crates/ferogram)
[![docs.rs](https://img.shields.io/docsrs/ferogram?style=flat-square&color=22c55e)](https://docs.rs/ferogram)
[![TL Layer](https://img.shields.io/badge/TL%20Layer-227-8b5cf6?style=flat-square)](https://core.telegram.org/schema)
[![License](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue?style=flat-square)](#license)

Hey, glad you're here.

ferogram is an async Rust library for Telegram's MTProto protocol, built by [Ankit Chaubey](https://github.com/ankit-chaubey). It talks to Telegram directly, no Bot API proxy in between, and it works for both bots and user accounts from the same API.

I built it because I kept hitting walls with other MTProto libraries. Things that should have been straightforward weren't, and I kept needing the library to behave slightly differently than it would let me. So I wrote my own. The goal was to cover the major use cases first, and it does: messaging, media, CDN downloads, inline keyboards, FSM for multi-step conversations, FakeTLS and MTProxy for censored networks, and a raw `invoke()` escape hatch for anything the high-level API doesn't wrap yet.

If you want the Bot API instead, take a look at [ferobot](https://github.com/ankit-chaubey/ferobot).

If something's missing for you, drop by [t.me/FerogramChat](https://t.me/FerogramChat). I genuinely like hearing what people are building with it.

## Quick install

```toml
[dependencies]
ferogram = "0.6.0"
tokio    = { version = "1", features = ["full"] }
```

Get `api_id` and `api_hash` from [my.telegram.org](https://my.telegram.org). That's all you need to get started.

## Where to go next

- [Installation](./installation.md) covers credentials, optional feature flags, and session backends.
- [Quick Start: Bot](./quickstart-bot.md) gets a bot running in about 20 lines.
- [Quick Start: User Account](./quickstart-user.md) covers phone login and sending your first message.
- [Crate Architecture](./crates.md) if you want to understand how the pieces fit together.

## What's under the hood

Everything in the workspace is written from scratch: the `.tl` schema parser, the TL code generator, AES-IGE crypto, the DH key exchange, MTProto framing, the session layer, and the high-level client on top. Each piece is its own crate so you can pull in just what you need. Most people never touch any of it directly, but it's all there if you do.

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
| [`ferogram-tl-types`](https://docs.rs/ferogram-tl-types) | Auto-generated TL types, functions, and enums for Layer 227. |
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

## Python

Python support is live via [ferogram-py](https://github.com/ankit-chaubey/ferogram-py). Pre-built wheels for major platforms, no Rust toolchain needed.

```bash
pip install ferogram
```

## Secret chats

Secret chats (end-to-end encrypted) are fully implemented but not published to crates.io yet. The plan is to release once there is enough community demand for it.

## Voice and video calls

Group audio calls are fully implemented, stable, and already in active production use by the author. Written in Rust from scratch, not a wrapper around anything.

Group video calls are implemented and stable for most scenarios, with some known codec edge cases still being ironed out.

Peer-to-peer calls are partially implemented and still in active development.

All of this lives in its own workspace crate and will be published separately when it comes out of the workspace. Python bindings via ferogram-py are also planned.

## Community

- **Channel** (releases, announcements): [t.me/Ferogram](https://t.me/Ferogram)
- **Chat** (questions, discussion): [t.me/FerogramChat](https://t.me/FerogramChat)
- **API docs**: [docs.rs/ferogram](https://docs.rs/ferogram)
- **GitHub**: [github.com/ankit-chaubey/ferogram](https://github.com/ankit-chaubey/ferogram)

## License

MIT OR Apache-2.0.

Usage must comply with [Telegram's API Terms of Service](https://core.telegram.org/api/terms).
