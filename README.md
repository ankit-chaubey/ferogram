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
**Modern APIs. Native MTProto.** Ferogram helps you build fast, powerful Telegram applications for both bots and user accounts without compromising on flexibility.

From messaging and media to transfers, calls, MTProxy, and much more, Ferogram brings everything together in one elegant framework.

> Let's build something amazing. High-level APIs where you want them, raw invoke() where you need complete control.


---

## Getting started
All it takes is a single line in your `Cargo.toml`.
```toml
[dependencies]
ferogram = "0.6.4"
```

Development on GitHub moves faster than crates.io. Releases are pushed to [crates.io](https://crates.io/crates/ferogram) when there's a patch or a proper release, so there may be fixes and features on `main` or `dev` that aren't published yet. If you need something from `main`, you can point directly to a specific commit:

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
<details>
<summary>Starting as bot</summary>

### Quick start: bot
Building a bot is just as simple as building a user client. All you need is a bot token from [@BotFather](https://t.me/BotFather), and you're ready to go.
```rust
use ferogram::{Client, update::Update};

const API_ID: i32 = 0;
const API_HASH: &str = "";

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let (client, _) = Client::quick_connect("bot.session", API_ID, API_HASH).await?;

    let mut stream = client.stream_updates();
    while let Some(upd) = stream.next().await {
        if let Update::NewMessage(msg) = upd {
            if !msg.outgoing() {
                msg.reply(msg.text().unwrap_or_default()).await.ok();
            }
        }
    }
    Ok(())
}
```

</details>

---

## Python support
Love Python as much as we do? Ferogram is also available for Python with a clean, user-friendly API while keeping the heavy lifting in Rust.

Powered by a high-performance Rust core, it takes care of networking, encryption, TL parsing, and session management. [ferogram-py](https://github.com/ankit-chaubey/ferogram-py), Also [pre-built wheels](https://pypi.org/project/ferogram) on PyPI, no Rust toolchain needed.

---

## Core features

### Dispatcher and filters
Ferogram includes a powerful dispatcher with composable filters (&, |, !), a flexible FSM with pluggable state storage, session backends, media transfer utilities, and much more.

For detailed usage examples and API documentation, check the README files and documentation of the dedicated crates in this workspace.

### Raw API

When the high-level API doesn't cover something, `client.invoke()` takes any TL function directly:

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

### Session backends

By default the session is a binary file on disk. Switch to SQLite, LibSQL (Turso), or a base64 string for serverless setups. You can also bring your own by implementing `SessionBackend`.

---

## What's covered

See **[This](FEATURES.md)** for the quick list with method signatures. Runnable examples are in [`ferogram/examples/`](ferogram/examples/).

If something is missing, open a feature request or drop by [t.me/FerogramChat](https://t.me/FerogramChat). If the high-level API isn't enough, the raw API is always there.

---

### **Secret chats** 
Secret Chats (end-to-end encrypted) are fully implemented but not published to crates.io yet. The plan is to release once there is enough community demand for it.

### **Voice and video calls**
Group audio, video and P2P calling are now fully implemented. To get started, check out the [tgcalls](https://crates.io/crates/tgcalls) crate and its examples in [tgcalls](https://github.com/ankit-chaubey/tgcalls) repository. It provides seamless integration between Ferogram and the official [ntgcalls](https://crates.io/crates/ntgcalls) Rust bindings for building Telegram voice and video calling applications.

---

## Community and links

- **Channel** (releases, announcements): [t.me/Ferogram](https://t.me/Ferogram)
- **Chat** (questions, discussion): [t.me/FerogramChat](https://t.me/FerogramChat)
- **API docs**: [docs.rs/ferogram](https://docs.rs/ferogram)

## Contributing

Read [contribution guide](CONTRIBUTING.md) before opening a PR and as well Security issues: see [security.md](SECURITY.md).

## Acknowledgments

Big shoutout to [Lonami](https://codeberg.org/Lonami/grammers) for grammers. It was one of the most helpful references while building ferogram initially.

Protocol behavior references from [Telegram Desktop](https://github.com/telegramdesktop/tdesktop) and [TDLib](https://github.com/tdlib/td).

## License

MIT OR Apache-2.0. See [LICENSE-MIT](LICENSE-MIT) and [LICENSE-APACHE](LICENSE-APACHE).

Usage must comply with [Telegram's API Terms of Service](https://core.telegram.org/api/terms).
