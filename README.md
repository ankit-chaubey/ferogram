<div align="center">

# ferogram

Async Rust library for Telegram's MTProto protocol.

[![Crates.io](https://img.shields.io/crates/v/ferogram?style=flat-square&color=fc8d62)](https://crates.io/crates/ferogram)
[![docs.rs](https://img.shields.io/badge/docs.rs-ferogram-5865F2?style=flat-square)](https://docs.rs/ferogram)
[![License](https://img.shields.io/badge/license-MIT%20%7C%20Apache--2.0-blue?style=flat-square)](LICENSE-MIT)
[![TL Layer](https://img.shields.io/badge/TL%20Layer-225-8b5cf6?style=flat-square)](https://core.telegram.org/schema)
[![Telegram Channel](https://img.shields.io/badge/channel-%40Ferogram-2CA5E0?style=flat-square&logo=telegram)](https://t.me/Ferogram)
[![Telegram Chat](https://img.shields.io/badge/chat-%40FerogramChat-2CA5E0?style=flat-square&logo=telegram)](https://t.me/FerogramChat)

Built by **[Ankit Chaubey](https://github.com/ankit-chaubey)**

</div>

I built ferogram because I kept hitting walls with other MTProto libraries. Things that should have been straightforward weren't, and I kept needing the library to behave slightly differently than it would let me. So I wrote my own.

It talks to Telegram directly over MTProto, no Bot API proxy in between. It works for both bots and user accounts from the same API and the same client builder. The major use cases are covered: messaging, media, inline keyboards, CDN downloads, FSM for multi-step conversations, FakeTLS and MTProxy for censored networks, and a raw `invoke()` escape hatch for anything the high-level API doesn't wrap yet.

If you want the Bot API instead, take a look at [ferobot](https://github.com/ankit-chaubey/ferobot).

The longer-term goal is to support multiple languages from the same Rust core. Python is already live as [ferogram-py](https://github.com/ankit-chaubey/ferogram-py) on PyPI, pre-built wheels, no Rust toolchain needed.

> [!NOTE]
> ferogram is still in active development. It covers major use cases and runs in production, but the API may still shift. Check [CHANGELOG](CHANGELOG.md) before upgrading.

## Installation

```toml
[dependencies]
ferogram = "0.6.0"
tokio    = { version = "1", features = ["full"] }
```

Get `api_id` and `api_hash` from [my.telegram.org](https://my.telegram.org). For optional feature flags (SQLite session, HTML parser, FSM derive macro) see the [`ferogram` crate README](ferogram/README.md#installation).

## Quick start: bot

```rust
use ferogram::{Client, update::Update};

const API_ID: i32 = 0; // from https://my.telegram.org
const API_HASH: &str = ""; // from https://my.telegram.org

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

## Quick start: user account

```rust
use ferogram::Client;

const API_ID: i32 = 0; // from https://my.telegram.org
const API_HASH: &str = ""; // from https://my.telegram.org

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let (client, _) = Client::quick_connect("my.session", API_ID, API_HASH).await?;

    client.send_message("me", "Hello from ferogram!").await?;
    Ok(())
}
```

## Dispatcher and filters

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

## FSM

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

## Raw API

If the high-level API doesn't cover what you need, `client.invoke()` takes any TL function directly:

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

## Session backends

By default the session is a binary file on disk. Switch to SQLite, LibSQL (Turso), or a base64 string for serverless setups. You can also bring your own by implementing `SessionBackend`.

```rust
let s = client.export_session_string().await?;
let (client, _) = Client::builder().session_string(s).connect().await?;
```

## Features

See **[FEATURES.md](FEATURES.md)** for the full list with method signatures. Runnable examples are in [`ferogram/examples/`](ferogram/examples/).

If something is missing, open a feature request or drop by [t.me/FerogramChat](https://t.me/FerogramChat). If the high-level API isn't enough, the raw API is always there.

## Language bindings

```bash
pip install ferogram
```

Python support is live via [ferogram-py](https://github.com/ankit-chaubey/ferogram-py). Pre-built wheels for major platforms, no Rust toolchain required. More language targets are planned.

## Secret chats

Secret chats (end-to-end encrypted) are fully implemented but not published to crates.io yet. The plan is to release once there is enough community demand for it.

## Crates

Most users only need `ferogram`. The rest of the workspace exists if you need a specific layer on its own: [`ferogram-session`](ferogram-session/) for session backends, [`ferogram-fsm`](ferogram-fsm/) for FSM state storage, [`ferogram-parsers`](ferogram-parsers/) for HTML and Markdown entity parsing, [`ferogram-derive`](ferogram-derive/) for the `#[derive(FsmState)]` proc macro, [`ferogram-mtsender`](ferogram-mtsender/) for the DC connection pool, [`ferogram-connect`](ferogram-connect/) for raw TCP and MTProto framing, [`ferogram-mtproto`](ferogram-mtproto/) for the MTProto session layer, [`ferogram-crypto`](ferogram-crypto/) for crypto primitives, and the TL toolchain ([`ferogram-tl-types`](ferogram-tl-types/), [`ferogram-tl-gen`](ferogram-tl-gen/), [`ferogram-tl-parser`](ferogram-tl-parser/)) if you need the generated types or want to run the code generator yourself.

See the [`ferogram` crate README](ferogram/README.md#crates) for the full table with descriptions.

## Testing

```bash
cargo test --workspace
cargo test --workspace --all-features
```

## Community

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
