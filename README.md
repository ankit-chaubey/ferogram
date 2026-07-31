<div align="center">

# Ferogram

A Native & Elegant MTProto Framework for Rust

[![Crates.io](https://img.shields.io/crates/v/ferogram?style=flat-square&logo=rust&logoColor=white&color=F97316)](https://crates.io/crates/ferogram)
[![docs.rs](https://img.shields.io/badge/docs.rs-ferogram-5865F2?style=flat-square&logo=docs.rs&logoColor=white)](https://docs.rs/ferogram)
[![Dependencies](https://img.shields.io/badge/Dependencies-Up%20to%20date-14B8A6?style=flat-square)](https://deps.rs/repo/github/ankit-chaubey/ferogram)
[![License](https://img.shields.io/badge/License-MIT%20%7C%20Apache--2.0-64748B?style=flat-square)](LICENSE-MIT)
[![Telegram](https://img.shields.io/badge/Telegram-FerogramChat-06B6D4?style=flat-square&logo=telegram&logoColor=white)](https://t.me/FerogramChat)

Built by **[Ankit Chaubey](https://github.com/ankit-chaubey)**

</div>

## Overview
**Modern APIs.** **Native MTProto.** Ferogram helps you build **fast**, **powerful** Telegram applications for both bots and user accounts without compromising on flexibility.

From messaging and media to transfers, calls, MTProxy, and much more, Ferogram brings everything together in one elegant framework.

> Let's build something amazing. High-level APIs where you want them, raw invoke() where you need complete control.

---

## Getting started
All it takes is a single line in your `Cargo.toml`.
```toml
[dependencies]
ferogram = "0.6.5"
```

Development on GitHub moves faster than crates.io. If you need something from `main`, you can point directly to a specific commit:

```toml
ferogram = { git = "https://github.com/ankit-chaubey/ferogram", rev = "COMMIT_SHA" }
```

Otherwise, stable from crates.io is the safe default.

---

### Quick start

```rust
use ferogram::Client;

const API_ID: i32 = 0;
const API_HASH: &str = "";

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let (client, _) = Client::quick_connect("my.session", API_ID, API_HASH).await?;

    client.send_message("me", "Hello from ferogram!").await?;
    Ok(())
}
```

Building a bot instead? Same API, just a bot token from [@BotFather](https://t.me/BotFather). Full bot example lives in the [ferogram/examples](https://github.com/ankit-chaubey/ferogram/tree/main/ferogram/examples).

---

## Python support
Love Python as much as we do? Ferogram is also available for Python with a clean, user-friendly API while keeping the heavy lifting in Rust.

Powered by same high-performance Rust core, it takes care of networking, encryption, TL parsing, and session management. Explore the [ferogram python](https://github.com/ankit-chaubey/ferogram-py) source, learn about its [architecture](https://github.com/ankit-chaubey/ferogram-py/blob/main/assets/architecture.svg), or install the pre-built wheels for you platform from [PyPI](https://pypi.org/project/ferogram), no Rust toolchain needed.

All you need is:
```bash
pip install ferogram
```

---

## Core features

| Feature | Description |
|---|---|
| **TgCalls** | Voice and video calls for Telegram clients, wrapping Telegram's calling stack in a Rust API for joining voice chats and streaming media. See [tgcalls](https://crates.io/crates/tgcalls). |
| **Dispatcher and filters** | Composable filters (`&`, `\|`, `!`) for routing updates to handlers. |
| **FSM** | Finite state machine for multi-step bot conversations, with pluggable storage, configurable state strategies, and type-safe states. See [ferogram-fsm](https://github.com/ankit-chaubey/ferogram/tree/main/ferogram-fsm). |
| **Raw API** | `client.invoke()` takes any TL function directly, for when the high-level API doesn't cover something. |
| **Session backends** | Binary file by default. SQLite, LibSQL (Turso), and base64 string are also supported, or bring your own via [`SessionBackend`](https://github.com/ankit-chaubey/ferogram/tree/main/ferogram-session#custom-backends). |
| **MTProxy** | Built-in support for Classic, DD, and FakeTLS transport. |

<details>
<summary>Raw API example</summary>

```rust
use ferogram::tl;

let req = tl::functions::bots::SetBotCommands {
    scope: tl::enums::BotCommandScope::Default(tl::types::BotCommandScopeDefault {}),
    lang_code: "en".into(),
    commands: vec![tl::enums::BotCommand::BotCommand(tl::types::BotCommand {
        command: "start".into(),
        description: "Start the bot".into(),
    })],
};
client.invoke(&req).await?;
client.invoke_on_dc(2, &req).await?;
```

</details>

See the crate documentation for a complete overview of all supported features.

---

## What's covered

See **[FEATURES.md](FEATURES.md)** for the full feature list, or try the [runnable examples](https://github.com/ankit-chaubey/ferogram/tree/main/ferogram/examples).

If something is missing, open a feature request or drop a suggestion in [t.me/FerogramChat](https://t.me/FerogramChat).

---

## Community and links

Join the ferogram community! Questions, discussions, bugs report and feedback are always welcome.

- **Channel** (releases & announcements): [@Ferogram](https://t.me/Ferogram)
- **Chat** (questions & discussion): [@FerogramChat](https://t.me/FerogramChat)
- **Docs**: [docs.ferogram.dev](https://docs.ferogram.dev)

## Contributing

Please read the [Contributing Guide](https://github.com/ankit-chaubey/ferogram/blob/main/CONTRIBUTING.md) before opening a pull request.

## Acknowledgments

Big shoutout to [Lonami](https://codeberg.org/Lonami/grammers) for grammers. It was one of the most helpful references while building ferogram initially.

Protocol behavior references from [Telegram Desktop](https://github.com/telegramdesktop/tdesktop) and [TDLib](https://github.com/tdlib/td).

## License

This project is dual-licensed under:

- MIT License
- Apache License 2.0

You may choose either license.

You are free to use, modify, and distribute this software, including for commercial use, provided the original license and copyright notice are included.

See [`LICENSE-MIT`](https://github.com/ankit-chaubey/ferogram/blob/main/LICENSE-MIT) and [`LICENSE-APACHE`](https://github.com/ankit-chaubey/ferogram/blob/main/LICENSE-APACHE) for full details.

Usage must comply with [Telegram's API Terms of Service](https://core.telegram.org/api/terms).
