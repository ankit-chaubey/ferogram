# ⚡ ferogram

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="images/ferogram-banner-dark.png">
  <source media="(prefers-color-scheme: light)" srcset="images/ferogram-banner-light.png">
</picture>

<div class="hero-banner">
<h2>A modular, production-grade async Rust implementation of the Telegram MTProto protocol</h2>
<div class="hero-badges">

[![Crates.io](https://img.shields.io/crates/v/ferogram?color=7c6af7&label=ferogram&style=flat-square)](https://crates.io/crates/ferogram)
[![docs.rs](https://img.shields.io/docsrs/ferogram?style=flat-square&color=22c55e)](https://docs.rs/ferogram)
[![TL Layer](https://img.shields.io/badge/TL%20Layer-224-8b5cf6?style=flat-square)](https://core.telegram.org/schema)
[![License](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue?style=flat-square)](#license)
[![Rust](https://img.shields.io/badge/rust-2024_edition-f74c00?style=flat-square)](https://www.rust-lang.org/)

</div>
</div>

`ferogram` is a hand-written, bottom-up implementation of [Telegram MTProto](https://core.telegram.org/mtproto) in pure Rust. Every component: from the `.tl` schema parser, to AES-IGE encryption, to the Diffie-Hellman key exchange, to the typed async update stream: is owned and understood by this project.

**No black boxes. No magic. Just Rust, all the way down.**

---

## Why ferogram?

Most Telegram libraries are thin wrappers around generated code or ports from Python/JavaScript. `ferogram` is different: it was built from scratch to understand MTProto at the lowest level, then exposed through a straightforward high-level API.

<div class="feature-grid">
<div class="feature-card">
<div class="fc-icon">🦀</div>
<div class="fc-title">Pure Rust</div>
<div class="fc-desc">No FFI, no unsafe blocks. Fully async with Tokio. Works on Android (Termux), Linux, macOS, Windows.</div>
</div>
<div class="feature-card">
<div class="fc-icon">⚡</div>
<div class="fc-title">Full MTProto 2.0</div>
<div class="fc-desc">Complete DH handshake, AES-IGE encryption, salt tracking, DC migration: all handled automatically.</div>
</div>
<div class="feature-card">
<div class="fc-icon">🔐</div>
<div class="fc-title">User + Bot Auth</div>
<div class="fc-desc">Phone login with 2FA SRP, bot token login, session persistence across restarts.</div>
</div>
<div class="feature-card">
<div class="fc-icon">📡</div>
<div class="fc-title">Typed Update Stream</div>
<div class="fc-desc">NewMessage, MessageEdited, CallbackQuery, InlineQuery, ChatAction, UserStatus: all strongly typed.</div>
</div>
<div class="feature-card">
<div class="fc-icon">🔧</div>
<div class="fc-title">Raw API Escape Hatch</div>
<div class="fc-desc">Call any of 500+ Telegram API methods directly via <code>client.invoke()</code> with full type safety.</div>
</div>
<div class="feature-card">
<div class="fc-icon">🏗️</div>
<div class="fc-title">Auto-Generated Types</div>
<div class="fc-desc">All 2,329 Layer 224 constructors generated at build time from the official TL schema.</div>
</div>
</div>

---

## Crate overview

| Crate | Description | Typical user |
|---|---|---|
| **`ferogram`** | High-level async client: auth, send, receive, bots | ✅ You |
| `ferogram-tl-types` | All Layer 224 constructors, functions, enums | Raw API calls |
| `ferogram-mtproto` | MTProto session, DH, framing, transport | Library authors |
| `ferogram-crypto` | AES-IGE, RSA, SHA, auth key derivation | Internal |
| `ferogram-session` | Session persistence types and pluggable storage backends | Custom session storage |
| `ferogram-parsers` | Telegram HTML and Markdown entity parsers | Formatted text handling |
| `ferogram-tl-gen` | Build-time Rust code generator | Build tool |
| `ferogram-tl-parser` | `.tl` schema → AST parser | Build tool |

> **TIP:** Most users only ever import `ferogram`. The other crates are either used internally or for advanced raw API calls.

---

## Quick install

```toml
[dependencies]
ferogram = "0.3.6"
tokio        = { version = "1", features = ["full"] }
```

Then head to [Installation](./installation.md) for credentials setup, or jump straight to:

- [Quick Start: User Account](./quickstart-user.md): login, send a message, receive updates
- [Quick Start: Bot](./quickstart-bot.md): bot token login, commands, callbacks

---

## Release history

See [What's New](./whats-new.md) for a full version-by-version breakdown, or the [CHANGELOG](https://github.com/ankit-chaubey/ferogram/blob/main/CHANGELOG.md) for the raw diff summary.

---

## Author

Developed by [**Ankit Chaubey**](https://github.com/ankit-chaubey) out of curiosity to explore.

<div align="center">

[![Crates.io](https://img.shields.io/crates/v/ferogram?style=flat-square\&color=fc8d62\&label=ferogram)](https://crates.io/crates/ferogram)
[![docs.rs](https://img.shields.io/badge/docs.rs-ferogram-5865F2?style=flat-square)](https://docs.rs/ferogram)
[![License](https://img.shields.io/badge/license-MIT%20%7C%20Apache--2.0-blue?style=flat-square)](LICENSE-MIT)
[![TL Layer](https://img.shields.io/badge/TL%20Layer-224-8b5cf6?style=flat-square)](https://core.telegram.org/schema)
[![Telegram](https://img.shields.io/badge/chat-%40FerogramChat-2CA5E0?style=flat-square\&logo=telegram)](https://t.me/FerogramChat)
[![Channel](https://img.shields.io/badge/channel-%40ferogram-2CA5E0?style=flat-square\&logo=telegram)](https://t.me/Ferogram)

</div>

ferogram is developed as part of exploration, learning, and experimentation with the Telegram MTProto protocol.
Use it at your own risk. Its future and stability are not yet guaranteed.

---

## Terms of Service

Ensure your usage complies with [Telegram's Terms of Service](https://core.telegram.org/api/terms) and [API Terms of Service](https://core.telegram.org/api/terms). Misuse of the Telegram API, including spam, mass scraping, or automation of normal user accounts, may result in account limitations or permanent bans.
