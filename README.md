<div align="center">

# ferogram

Async Rust library for Telegram's MTProto protocol.

[![Crates.io](https://img.shields.io/crates/v/ferogram?style=flat-square&color=fc8d62)](https://crates.io/crates/ferogram)
[![docs.rs](https://img.shields.io/badge/docs.rs-ferogram-5865F2?style=flat-square)](https://docs.rs/ferogram)
[![License](https://img.shields.io/badge/license-MIT%20%7C%20Apache--2.0-blue?style=flat-square)](LICENSE-MIT)
[![TL Layer](https://img.shields.io/badge/TL%20Layer-228-8b5cf6?style=flat-square)](https://core.telegram.org/schema)
[![Telegram Channel](https://img.shields.io/badge/channel-%40Ferogram-2CA5E0?style=flat-square&logo=telegram)](https://t.me/Ferogram)
[![Telegram Chat](https://img.shields.io/badge/chat-%40FerogramChat-2CA5E0?style=flat-square&logo=telegram)](https://t.me/FerogramChat)

Built by **[Ankit Chaubey](https://github.com/ankit-chaubey)**

</div>

I built ferogram because I kept hitting walls with other MTProto libraries. Things that should have been straightforward weren't, and I kept needing the library to behave slightly differently than it would let me. So I wrote my own.


It talks to Telegram directly over MTProto, no Bot API proxy in between. It works for both bots and user accounts from the same API and the same client builder. 

The major use cases are covered: messaging, media, inline keyboards, CDN downloads, FSM for multi-step conversations, FakeTLS and MTProxy for censored networks, and a raw `invoke()` escape hatch for anything the high-level API doesn't wrap yet.

---

If you want the Bot API instead, take a look at [ferobot](https://github.com/ankit-chaubey/ferobot).

The longer-term goal is to support [multiple languages](https://github.com/ankit-chaubey/ferogram/blob/main/FEATURES.md#multi-language-bindings) from the same Rust core. Python is already live as [ferogram-py](https://github.com/ankit-chaubey/ferogram-py) on PyPI, [pre-built wheels](https://pypi.org/project/ferogram), no Rust toolchain needed.

> [!NOTE]
> ferogram is still in active development. It covers major use cases and runs in production, but the API may still shift. Check [CHANGELOG](CHANGELOG.md) before upgrading.

---

## Getting started

```toml
[dependencies]
ferogram = "0.6.4"
tokio    = { version = "1", features = ["full"] }
```

Get `api_id` and `api_hash` from [my.telegram.org](https://my.telegram.org). For optional feature flags (SQLite session, HTML parser, FSM derive macro) see the [`ferogram` crate README](ferogram/README.md#installation).

---

Development on GitHub moves faster than crates.io. Releases are pushed to [crates.io](https://crates.io/crates/ferogram) when there's a patch or a proper release, so there may be fixes and features on `main` that aren't published yet. If you need something from `main`, you can point directly to a specific commit:

```toml
ferogram = { git = "https://github.com/ankit-chaubey/ferogram", rev = "COMMIT_SHA" }
```

Otherwise, stable from crates.io is the safe default.

---

### Quick start: bot

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

### Quick start: user account

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

---

## Core features

### Dispatcher and filters

```rust
use ferogram::filters::{Dispatcher, command, private, text_contains};

let mut dp = Dispatcher::new();

dp.on_message(command("start"), |msg| async move {
    msg.reply("Hello!").await.ok();
});

dp.on_message(private() & text_contains("help"), |msg| async move {
    msg.reply("Type /start to begin.").await.ok();
});

while let Some(upd) = stream.next().await {
    dp.dispatch(upd).await;
}
```

Filters compose with `&`, `|`, `!`. Built-ins cover `command`, `private`, `group`, `channel`, `text`, `media`, `forwarded`, `reply`, `album`, `custom`, and more.

### FSM

```rust
use ferogram::{FsmState, fsm::MemoryStorage};
use std::sync::Arc;

#[derive(FsmState, Clone, Debug, PartialEq)]
enum Form { Name, Age }

dp.with_state_storage(Arc::new(MemoryStorage::new()));

dp.on_message_fsm(text(), Form::Name, |msg, state| async move {
    state.set_data("name", msg.text().unwrap()).await.ok();
    state.transition(Form::Age).await.ok();
    msg.reply("How old are you?").await.ok();
});
```

Storage is swappable. Implement `StateStorage` to use Redis, a database, or anything else.

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

```rust
let s = client.export_session_string().await?;
let (client, _) = Client::builder().session_string(s).connect().await?;
```

---

## What's covered

See **[FEATURES.md](FEATURES.md)** for the full list with method signatures. Runnable examples are in [`ferogram/examples/`](ferogram/examples/).

If something is missing, open a feature request or drop by [t.me/FerogramChat](https://t.me/FerogramChat). If the high-level API isn't enough, the raw API is always there.

---

**Secret chats** (end-to-end encrypted) are fully implemented but not published to crates.io yet. The plan is to release once there is enough community demand for it.

**Voice and video calls** : group audio is fully implemented, stable, and already in production use. Group video is implemented with some codec edge cases still being ironed out. P2P is partially implemented and in active development. All of this will be published as separate crates when it comes out of the workspace.

---

## Testing

```bash
cargo test --workspace
cargo test --workspace --all-features
```

---

## Community and links

- **Channel** (releases, announcements): [t.me/Ferogram](https://t.me/Ferogram)
- **Chat** (questions, discussion): [t.me/FerogramChat](https://t.me/FerogramChat)
- **Guide**: [ferogram.ankitchaubey.in](https://ferogram.ankitchaubey.in/)
- **API docs**: [docs.rs/ferogram](https://docs.rs/ferogram)
- **Crates.io**: [crates.io/crates/ferogram](https://crates.io/crates/ferogram)
- **GitHub**: [github.com/ankit-chaubey/ferogram](https://github.com/ankit-chaubey/ferogram)

## Contributing

Read [CONTRIBUTING.md](CONTRIBUTING.md) before opening a PR. Run `cargo fmt --all`, `cargo test --workspace`, and `cargo clippy --workspace` first. Security issues: see [SECURITY.md](SECURITY.md).

## Acknowledgments

Big shoutout to [Lonami](https://codeberg.org/Lonami/grammers) for grammers. It was one of the most helpful references while building ferogram, and grammers and Telethon are two of my all-time favorites. Love those projects.

Protocol behavior references from [Telegram Desktop](https://github.com/telegramdesktop/tdesktop) and [TDLib](https://github.com/tdlib/td).

## License

MIT OR Apache-2.0. See [LICENSE-MIT](LICENSE-MIT) and [LICENSE-APACHE](LICENSE-APACHE).

Usage must comply with [Telegram's API Terms of Service](https://core.telegram.org/api/terms).
