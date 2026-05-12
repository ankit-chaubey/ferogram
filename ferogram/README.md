<div align="center">

# ferogram

Async Rust client for the Telegram MTProto API.

[![Crates.io](https://img.shields.io/crates/v/ferogram?style=flat-square&color=fc8d62)](https://crates.io/crates/ferogram)
[![docs.rs](https://img.shields.io/badge/docs.rs-ferogram-5865F2?style=flat-square)](https://docs.rs/ferogram)
[![License](https://img.shields.io/badge/license-MIT%20%7C%20Apache--2.0-blue?style=flat-square)](../LICENSE-MIT)
[![TL Layer](https://img.shields.io/badge/TL%20Layer-225-8b5cf6?style=flat-square)](https://core.telegram.org/schema)
[![Telegram Channel](https://img.shields.io/badge/channel-%40Ferogram-2CA5E0?style=flat-square&logo=telegram)](https://t.me/Ferogram)
[![Telegram Chat](https://img.shields.io/badge/chat-%40FerogramChat-2CA5E0?style=flat-square&logo=telegram)](https://t.me/FerogramChat)

Built by **[Ankit Chaubey](https://github.com/ankit-chaubey)**

</div>

> [!NOTE]
> ferogram is still in development but already covers major use cases for production. Check [CHANGELOG](../CHANGELOG.md) before upgrading.

## What it is

This is the high-level client crate in the ferogram workspace. It talks to Telegram directly over MTProto, no Bot API proxy in between. Works for user accounts and bots.

For the rest of the workspace (crypto, session, TL types, transport layer, etc.) see the [repository root](https://github.com/ankit-chaubey/ferogram) or the [crates table](#crates) below.

---

## Installation

```toml
[dependencies]
ferogram = "0.4.0"
tokio    = { version = "1", features = ["full"] }
```

Get `api_id` and `api_hash` from [my.telegram.org](https://my.telegram.org).

Optional feature flags:

```toml
ferogram = { version = "0.4.0", features = [
    "sqlite-session",  # SqliteBackend via rusqlite
    "libsql-session",  # LibSqlBackend via libsql-client (Turso)
    "html",            # parse_html / generate_html (built-in parser)
    "html5ever",       # parse_html via spec-compliant html5ever
    "derive",          # #[derive(FsmState)]
    "serde",           # serde support on session types
] }
```

---

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

---

## Connecting

```rust
use ferogram::Client;

let (client, _shutdown) = Client::builder()
    .api_id(12345)
    .api_hash("your_api_hash")
    .session("my.session")
    .catch_up(true)
    .connect()
    .await?;
```

Some builder options worth knowing:

- `.session(path)` / `.in_memory()` / `.session_string(s)` / `.session_backend(Arc<...>)` for session storage
- `.socks5("127.0.0.1:1080")` / `.proxy_link("t.me/proxy?...")` for proxies
- `.transport(TransportKind::Obfuscated)` or `.transport(TransportKind::FakeTls)` for DPI bypass
- `.probe_transport(true)` to race transports and use the first that connects
- `.resilient_connect(true)` to fall back through DoH and Telegram's special-config if TCP fails
- `.catch_up(true)` to replay missed updates after a reconnect
- `.retry_policy(...)` / `.restart_policy(...)` to customize retry and reconnect behavior

Full list and usage at [docs.rs/ferogram](https://docs.rs/ferogram).

---

## Dispatcher and filters

```rust
use ferogram::filters::{Dispatcher, command, private, text_contains, group, media};

let mut dp = Dispatcher::new();

dp.on_message(command("start"), |msg| async move {
    msg.reply("Hello!").await.ok();
});

dp.on_message(private() & text_contains("help"), |msg| async move {
    msg.reply("Type /start to begin.").await.ok();
});

dp.on_message(group() & media(), |msg| async move {
    // handle media in groups
});

while let Some(upd) = stream.next().await {
    dp.dispatch(upd).await;
}
```

Filters compose with `&`, `|`, `!`. Built-ins include `command`, `private`, `group`, `channel`, `text`, `text_contains`, `media`, `photo`, `document`, `forwarded`, `reply`, `from_user`, `album`, `custom`, and more.

---

## Middleware

```rust
dp.middleware(|upd, next| async move {
    tracing::info!("incoming update");
    let result = next.run(upd).await;
    tracing::info!("handler done");
    result
});
```

Runs in registration order. Call `next.run(upd)` to pass control forward, or return early to stop the chain.

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

`MemoryStorage` is built in. To persist state, implement `StateStorage` for Redis, a database, or anything else. State keys scope per-user, per-chat, or per-user-in-chat via `StateKeyStrategy`. See [`ferogram-fsm`](../ferogram-fsm/) for details.

---

## Session backends

```rust
use ferogram_session::{SqliteBackend, LibSqlBackend};
use std::sync::Arc;

Client::builder().session("bot.session")                                     // binary file (default)
Client::builder().in_memory()                                                // no persistence
Client::builder().session_string(env::var("SESSION")?)                       // base64 string
Client::builder().session_backend(Arc::new(SqliteBackend::open("s.db")?))   // sqlite
Client::builder().session_backend(Arc::new(LibSqlBackend::remote(url, token).await?))  // turso
```

Custom: implement `SessionBackend` from [`ferogram-session`](../ferogram-session/). The base64 string backend is handy for serverless or containers where you can't write to disk.

---

## Transport and proxy

```rust
use ferogram::TransportKind;

Client::builder().transport(TransportKind::Abridged)    // default
Client::builder().transport(TransportKind::Obfuscated)  // DPI bypass, plain MTProxy secrets
Client::builder().transport(TransportKind::FakeTls)     // TLS camouflage, 0xee secrets

// MTProxy from a t.me link
Client::builder().proxy_link("https://t.me/proxy?server=HOST&port=PORT&secret=SECRET")

// SOCKS5
Client::builder().socks5("127.0.0.1:1080")

// Race transports, use first to connect
Client::builder().probe_transport(true)

// Fall back through DoH + Telegram special-config if TCP fails
Client::builder().resilient_connect(true)
```

See [`ferogram-connect`](../ferogram-connect/) for the framing layer underneath.

---

## Raw API

If the high-level API doesn't cover something yet, `client.invoke()` takes any Layer 225 TL function directly:

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
client.invoke_on_dc(2, &req).await?;  // target a specific DC
```

See [`ferogram-tl-types`](../ferogram-tl-types/) for all 2,329 generated types.

---

## Error handling

```rust
use ferogram::{InvocationError, RpcError};

match client.send_message("@peer", "Hi").await {
    Ok(()) => {}
    Err(InvocationError::Rpc(RpcError { code, message, .. })) => {
        eprintln!("Telegram error {code}: {message}");
    }
    Err(InvocationError::Io(e)) => eprintln!("I/O: {e}"),
    Err(e) => eprintln!("{e}"),
}
```

`FLOOD_WAIT` is handled automatically. To disable it:

```rust
use ferogram::retry::NoRetries;
Client::builder().retry_policy(Arc::new(NoRetries))
```

---

## Shutdown

```rust
let (client, shutdown) = Client::builder()...connect().await?;

shutdown.cancel();   // graceful
client.disconnect(); // immediate
```

---

## Features

Most of what you'd expect is already covered. Check **[FEATURES.md](../FEATURES.md)** for the complete list with all method signatures. A few things worth pointing out:

- [x] **Bot + user account** in the same API, same client builder
- [x] **Dispatcher with composable filters** (`&`, `|`, `!`) and middleware pipeline
- [x] **FSM with pluggable storage** for multi-step conversations
- [x] **FakeTLS and Obfuscated2 transport** for censored regions, full MTProxy support
- [x] **Resilient connect** - DoH resolver + Telegram special-config fallback when TCP is blocked
- [x] **Transport probing** - races multiple transports, uses whichever connects first
- [x] **Update gap recovery** - PTS/QTS tracking, fetches missed updates via `getDifference` on reconnect
- [x] **QR code login** for user accounts
- [x] **Concurrent upload and download** with per-part retry and CDN redirect handling
- [x] **Turso/LibSQL session backend** for serverless and distributed setups
- [x] **Forum topics** (supergroups with topics enabled) - create, edit, delete
- [x] **Inline button click from code** (`msg.click_button`, `msg.click_button_where`)
- [x] **Scheduled messages** - send, list, delete, send immediately
- [x] **Paid reactions** and custom emoji reactions
- [x] **Python bindings** via [ferogram-py](https://github.com/ankit-chaubey/ferogram-py), pre-built wheels, no Rust toolchain needed
- [ ] Secret chats (end-to-end encrypted) - not yet

---

## Crates

Most people only need `ferogram`. But each crate is independently publishable if you need just one layer.

| Crate | What it does |
|---|---|
| [`ferogram`](.) | High-level client. Auth, messaging, media, dispatcher, FSM, middleware. |
| [`ferogram-session`](../ferogram-session/) | Session types and pluggable storage backends (file, memory, SQLite, LibSQL, base64). |
| [`ferogram-fsm`](../ferogram-fsm/) | FSM state storage and context. `StateStorage` trait, `MemoryStorage`, `StateContext`. |
| [`ferogram-parsers`](../ferogram-parsers/) | Telegram Markdown and HTML entity parsers. |
| [`ferogram-derive`](../ferogram-derive/) | `#[derive(FsmState)]` proc macro. |
| [`ferogram-mtsender`](../ferogram-mtsender/) | DC connection pool and retry policy. `AutoSleep`, `NoRetries`, `CircuitBreaker`. |
| [`ferogram-connect`](../ferogram-connect/) | Raw TCP, MTProto framing, obfuscation, SOCKS5, MTProxy, gzip. |
| [`ferogram-mtproto`](../ferogram-mtproto/) | MTProto 2.0 session, DH key exchange, message framing, PFS key binding. |
| [`ferogram-crypto`](../ferogram-crypto/) | AES-IGE, RSA, SHA, Diffie-Hellman, PQ factorization, auth key derivation. |
| [`ferogram-tl-types`](../ferogram-tl-types/) | Auto-generated TL types, functions, and enums for Layer 225. |
| [`ferogram-tl-gen`](../ferogram-tl-gen/) | Build-time code generator from TL AST to Rust source. |
| [`ferogram-tl-parser`](../ferogram-tl-parser/) | Parses `.tl` schema text into a Definition AST. |

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

---

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

