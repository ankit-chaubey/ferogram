<div align="center">

# ferogram

Async Rust library for Telegram's MTProto protocol.

[![Crates.io](https://img.shields.io/crates/v/ferogram?style=flat-square&color=fc8d62)](https://crates.io/crates/ferogram)
[![docs.rs](https://img.shields.io/badge/docs.rs-ferogram-5865F2?style=flat-square)](https://docs.rs/ferogram)
[![License](https://img.shields.io/badge/license-MIT%20%7C%20Apache--2.0-blue?style=flat-square)](LICENSE-MIT)
[![TL Layer](https://img.shields.io/badge/TL%20Layer-224-8b5cf6?style=flat-square)](https://core.telegram.org/schema)
[![Telegram Channel](https://img.shields.io/badge/channel-%40Ferogram-2CA5E0?style=flat-square&logo=telegram)](https://t.me/Ferogram)
[![Telegram Chat](https://img.shields.io/badge/chat-%40FerogramChat-2CA5E0?style=flat-square&logo=telegram)](https://t.me/FerogramChat)

Built by **[Ankit Chaubey](https://github.com/ankit-chaubey)**

</div>

> **Pre-production.** APIs may change between minor versions. Check [CHANGELOG](CHANGELOG.md) before upgrading.

---

## What it is

ferogram is an MTProto client library for Rust. It covers both user accounts and bots, talking to Telegram's servers directly over MTProto: no Bot API proxy in between.

Written from scratch in async Rust on Tokio, organized as a workspace of focused crates. Most users only touch the `ferogram` crate directly.

---

## Crates

| Crate | What it does |
|---|---|
| [`ferogram`](ferogram/) | High-level async client. Auth, messaging, media, dispatcher, FSM, middleware. |
| [`ferogram-session`](ferogram-session/) | Session persistence types and pluggable storage backends. |
| [`ferogram-parsers`](ferogram-parsers/) | Telegram Markdown and HTML entity parsers. |
| [`ferogram-tl-types`](ferogram-tl-types/) | Auto-generated TL types, functions, and enums for Layer 224. |
| [`ferogram-mtproto`](ferogram-mtproto/) | MTProto 2.0 session, DH key exchange, message framing, transports. |
| [`ferogram-crypto`](ferogram-crypto/) | AES-IGE, RSA, SHA, Diffie-Hellman, obfuscation, auth key derivation. |
| [`ferogram-tl-gen`](ferogram-tl-gen/) | Build-time code generator from TL AST to Rust source. |
| [`ferogram-tl-parser`](ferogram-tl-parser/) | Parses `.tl` schema text into a Definition AST. |

---

## Installation

```toml
[dependencies]
ferogram = "0.3"
tokio    = { version = "1", features = ["full"] }
```

Get `api_id` and `api_hash` from [my.telegram.org](https://my.telegram.org).

Optional features:

```toml
ferogram = { version = "0.3", features = [
    "sqlite-session",  # SQLite backend (rusqlite)
    "libsql-session",  # libSQL / Turso backend
    "html",            # HTML entity parser
    "html5ever",       # html5ever-based HTML parser
    "derive",          # FsmState derive macro
    "serde",           # serde support for session types
] }
```

---

## Quick start: bot

```rust
use ferogram::{Client, update::Update};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let (client, _shutdown) = Client::builder()
        .api_id(std::env::var("API_ID")?.parse()?)
        .api_hash(std::env::var("API_HASH")?)
        .session("bot.session")
        .connect()
        .await?;

    client.bot_sign_in(&std::env::var("BOT_TOKEN")?).await?;
    client.save_session().await?;

    let mut stream = client.stream_updates();
    while let Some(upd) = stream.next().await {
        if let Update::NewMessage(msg) = upd {
            if !msg.outgoing() {
                if let Some(text) = msg.text() {
                    msg.reply(text).await?;
                }
            }
        }
    }
    Ok(())
}
```

---

## Quick start: user account

```rust
use ferogram::{Client, SignInError};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let (client, _shutdown) = Client::builder()
        .api_id(12345)
        .api_hash("your_api_hash")
        .session("my.session")
        .connect()
        .await?;

    if !client.is_authorized().await? {
        let token = client.request_login_code("+1234567890").await?;
        let code  = read_line();

        match client.sign_in(&token, &code).await {
            Ok(name) => println!("Signed in as {name}"),
            Err(SignInError::PasswordRequired(t)) => {
                client.check_password(*t, "my_2fa_password").await?;
            }
            Err(e) => return Err(e.into()),
        }
        client.save_session().await?;
    }

    client.send_message("me", "Hello from ferogram!").await?;
    Ok(())
}
```

---

## Features

For the full feature list see **[FEATURES.md](FEATURES.md)**.

**Authentication**
- Phone + code login with optional 2FA (SRP)
- Bot token login
- Session export / import as a portable string

**Updates**
- Typed async update stream: `NewMessage`, `MessageEdited`, `MessageDeleted`, `CallbackQuery`, `InlineQuery`, `InlineSend`, `UserStatus`, `UserTyping`, `ParticipantUpdate`, `JoinRequest`, `MessageReaction`, `PollVote`, `BotStopped`, `ShippingQuery`, `PreCheckoutQuery`, `ChatBoost`, `Raw`

**Messaging**
- Send, edit, delete, forward, pin, unpin
- Reply-to, schedule, silent flag
- HTML and Markdown entity formatting
- Inline keyboards with callback data

**Media**
- Upload files with concurrent chunked transfer
- Download media and CDN files to disk
- Photo, document, audio, video, sticker

**Peers and dialogs**
- Automatic access-hash caching for users, chats, channels
- Paginated dialog and message history iterators
- Global and per-chat message search
- Mark as read, delete dialogs, clear mentions

**Bot helpers**
- `FLOOD_WAIT` auto-retry with configurable policy
- Dispatcher with composable filter combinators (`&`, `|`, `!`)
- Middleware system for pre-handler interception
- FSM (finite state machine) for multi-step conversations
- Inline keyboard builder
- Callback query and inline query answering

**Connection**
- Automatic DC migration
- Transport probing (races Abridged vs Obfuscated)
- SOCKS5 proxy
- DNS-over-HTTPS resolver with TTL cache
- Reconnect with session persistence

**Raw API**
- `client.invoke(&req)` for any TL function
- `client.invoke_on_dc(dc_id, &req)` for DC-specific calls
- Full Layer 224 coverage via `ferogram::tl`

---

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

Filters compose with `&`, `|`, `!`. Built-ins include `command`, `private`, `group`, `channel`, `text`, `media`, `regex`, `custom`, and more.

---

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

State storage is pluggable. Implement `StateStorage` for Redis, SQL, or anything else.

---

## Session backends

| Backend | Feature | Notes |
|---|---|---|
| `BinaryFileBackend` | default | Single file on disk. |
| `InMemoryBackend` | default | No persistence. Tests. |
| `StringSessionBackend` | default | Base64 string. Serverless / env-var. |
| `SqliteBackend` | `sqlite-session` | Multi-session local file. |
| `LibSqlBackend` | `libsql-session` | Turso / distributed libSQL. |
| Custom |: | Implement `SessionBackend`. |

```rust
let s = client.export_session_string().await?;
let (client, _) = Client::builder().session_string(s).connect().await?;
```

---

## Raw API

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

---

## Testing

```bash
cargo test --workspace
cargo test --workspace --all-features
```

Integration tests use `InMemoryBackend` and don't need real credentials.

---

## Community

- **Channel** (releases, announcements): [t.me/Ferogram](https://t.me/Ferogram)
- **Chat** (questions, discussion): [t.me/FerogramChat](https://t.me/FerogramChat)
- **Guide**: [ferogram.ankitchaubey.in](https://ferogram.ankitchaubey.in/)
- **API docs**: [docs.rs/ferogram](https://docs.rs/ferogram)
- **Crates.io**: [crates.io/crates/ferogram](https://crates.io/crates/ferogram)
- **GitHub**: [github.com/ankit-chaubey/ferogram](https://github.com/ankit-chaubey/ferogram)

---

## Author

Developed by [Ankit Chaubey](https://github.com/ankit-chaubey) while exploring the MTProto protocol.

Thanks to [Lonami](https://codeberg.org/Lonami/grammers) for grammers (early MTProto reference), and to [Telegram Desktop](https://github.com/telegramdesktop/tdesktop) and [TDLib](https://github.com/tdlib/td) for protocol behavior references.

This is still early-stage work. The API is not stable yet. Use at your own risk.

---

## Contributing

Read [CONTRIBUTING.md](CONTRIBUTING.md) before opening a PR. Run `cargo test --workspace` and `cargo clippy --workspace` first. Security issues: see [SECURITY.md](SECURITY.md).

---

## License

MIT OR Apache-2.0, at your option. See [LICENSE-MIT](LICENSE-MIT) and [LICENSE-APACHE](LICENSE-APACHE).

Usage must comply with [Telegram's API Terms of Service](https://core.telegram.org/api/terms). Automating user accounts for spam or mass scraping will get them banned.
