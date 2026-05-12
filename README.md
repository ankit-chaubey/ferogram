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

> [!NOTE]
> ferogram is still in development but already covers major use cases for production. Check [CHANGELOG](CHANGELOG.md) before upgrading.

## What it is

ferogram is an MTProto client library for Rust. It works for both user accounts and bots, and talks to Telegram directly over MTProto with no Bot API HTTP proxy in between.

The goal is to eventually support multiple languages from the same Rust core, so you can write your bot in whatever language you prefer. Python is already live as a working example of that via [ferogram-py](https://github.com/ankit-chaubey/ferogram-py) on PyPI.

## Installation

```toml
[dependencies]
ferogram = "0.4.0"
tokio    = { version = "1", features = ["full"] }
```

Get `api_id` and `api_hash` from [my.telegram.org](https://my.telegram.org). For optional features see the [`ferogram` crate](ferogram/README.md#installation).

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

## Features

Most common use cases are already covered. See **[FEATURES.md](FEATURES.md)** for the full list. Working examples are in [`ferogram/examples/`](ferogram/examples/).

If something's missing, open a feature request or send a PR. Just make sure to read the [contributing guidelines](https://github.com/ankit-chaubey/ferogram#contributing) before you do.

If the high-level API doesn't cover what you need, you can always fall through to the [raw API](#raw-api) with `client.invoke()`.

## Raw API

If you need to call something that isn't wrapped yet, `client.invoke()` takes any TL function directly:

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

Filters compose with `&`, `|`, `!`. Built-ins include `command`, `private`, `group`, `channel`, `text`, `media`, `forwarded`, `reply`, `album`, `custom`, and more.

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

## Session backends

Session is stored as a binary file by default. Switch to SQLite or libSQL with a feature flag, or use a base64 string for serverless setups where you can't write to disk. You can also bring your own backend by implementing `SessionBackend`. See [`ferogram-session`](https://github.com/ankit-chaubey/ferogram#session-backends) for full details.

```rust
let s = client.export_session_string().await?;
let (client, _) = Client::builder().session_string(s).connect().await?;
```

## Language bindings

Python support is live via [ferogram-py](https://github.com/ankit-chaubey/ferogram-py). Install it with pip and you're good to go, no Rust toolchain required, wheels are pre-built for major platforms.

```bash
pip install ferogram
```

More language targets are planned.

## Crates

Most users only need the `ferogram` crate. If you need something lower-level like just the MTProto layer, crypto primitives, or the TL type generator on its own, see the [workspace crates overview](ferogram/README.md#crates).

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

Read [CONTRIBUTING.md](CONTRIBUTING.md) before opening a PR. Run `cargo fmt --all`, `cargo test --workspace` and `cargo clippy --workspace` first. Security issues: see [SECURITY.md](SECURITY.md).

## Author

[Ankit Chaubey](https://github.com/ankit-chaubey)

I built ferogram because I was already using other MTProto libraries but kept running into cases where I needed things to work a bit differently than they allowed. So I wrote my own.

It covers the major use cases and that was the primary goal. If something's missing for you, feel free to drop by [t.me/FerogramChat](https://t.me/FerogramChat) and say hi. I genuinely like hearing what people are building with it. Just keeping it real though, every new feature is more to maintain, so I'm a bit selective. But I still love to hear from you.

If ferogram has been useful, a star or fork means a lot. And if you want to contribute, even better.

## Acknowledgments

Big shoutout to [Lonami](https://codeberg.org/Lonami/grammers) for grammers. It was genuinely one of the most helpful references while building ferogram, and honestly grammers and Telethon are two of my all-time favorites that I've been using for years. Love those projects.

Protocol behavior references from [Telegram Desktop](https://github.com/telegramdesktop/tdesktop) and [TDLib](https://github.com/tdlib/td).

## License

MIT OR Apache-2.0. See [LICENSE-MIT](LICENSE-MIT) and [LICENSE-APACHE](LICENSE-APACHE).

Usage must comply with [Telegram's API Terms of Service](https://core.telegram.org/api/terms).
