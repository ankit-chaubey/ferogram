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

This is the main client crate. It talks to Telegram directly over MTProto, no Bot API proxy in between. It handles auth for both bots and user accounts, and gives you a dispatcher with composable filters, FSM for multi-step conversations, CDN downloads, middleware, and a raw `invoke()` escape hatch for anything not wrapped yet.

If you want the Bot API instead, take a look at [ferobot](https://github.com/ankit-chaubey/ferobot).

If you're starting fresh, this is the only crate you need. Everything else in the workspace exists to support it and can be pulled in separately if you need a specific layer on its own.

> [!NOTE]
> ferogram is still in active development. It covers major use cases and runs in production, but the API may still shift. Check [CHANGELOG](../CHANGELOG.md) before upgrading.

## Installation

```toml
[dependencies]
ferogram = "0.6.0"
tokio    = { version = "1", features = ["full"] }
```

Get `api_id` and `api_hash` from [my.telegram.org](https://my.telegram.org).

Optional feature flags:

```toml
ferogram = { version = "0.6.0", features = [
    "sqlite-session",  # SqliteBackend via rusqlite
    "libsql-session",  # LibSqlBackend via libsql-client (Turso)
    "html",            # parse_html / generate_html (built-in parser)
    "html5ever",       # parse_html via spec-compliant html5ever
    "derive",          # #[derive(FsmState)]
    "serde",           # serde support on session types
] }
```

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

## Examples

19 runnable examples covering everything from sending a message to your Saved Messages to a full FSM order bot.

```
cargo run --example hello_self
cargo run --example echo_bot
cargo run --example showcase_bot
# ... and 16 more
```

See **[examples/README.md](examples/README.md)** for the full list with descriptions and notes on when to use `quick_connect` vs `Client::builder()`.

## Connecting

`quick_connect` is the fast path. For anything more specific, use the builder:

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

A few builder options worth knowing:

- `.session(path)` / `.in_memory()` / `.session_string(s)` / `.session_backend(Arc<...>)` for session storage
- `.socks5("127.0.0.1:1080")` / `.proxy_link("t.me/proxy?...")` for proxies
- `.transport(TransportKind::Obfuscated)` or `.transport(TransportKind::FakeTls)` for DPI bypass
- `.probe_transport(true)` to race transports and use whichever connects first
- `.resilient_connect(true)` to fall back through DoH and Telegram's special-config if TCP is blocked
- `.catch_up(true)` to replay missed updates after a reconnect
- `.retry_policy(...)` / `.restart_policy(...)` for custom retry and reconnect behavior

Full list at [docs.rs/ferogram](https://docs.rs/ferogram).

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

Filters compose with `&`, `|`, `!`. Built-ins cover `command`, `private`, `group`, `channel`, `text`, `text_contains`, `media`, `photo`, `document`, `forwarded`, `reply`, `from_user`, `album`, `custom`, and more.

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

`MemoryStorage` is built in. To persist state across restarts, implement `StateStorage` for Redis, a database, or anything else. State keys scope per-user, per-chat, or per-user-in-chat via `StateKeyStrategy`. See [`ferogram-fsm`](../ferogram-fsm/) for details.

## Session backends

```rust
Client::builder().session("bot.session")                                              // binary file (default)
Client::builder().in_memory()                                                         // no persistence
Client::builder().session_string(env::var("SESSION")?)                               // base64 string
Client::builder().session_backend(Arc::new(SqliteBackend::open("s.db")?))            // sqlite
Client::builder().session_backend(Arc::new(LibSqlBackend::remote(url, token).await?)) // turso
```

The base64 string backend is useful for serverless or containers where writing to disk isn't an option. To bring your own, implement `SessionBackend` from [`ferogram-session`](../ferogram-session/).

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

// Fall back through DoH + Telegram special-config if TCP is blocked
Client::builder().resilient_connect(true)
```

See [`ferogram-connect`](../ferogram-connect/) for the framing layer underneath.

## Raw API

When the high-level API isn't enough, `client.invoke()` takes any Layer 225 TL function directly. It's the escape hatch, not the normal path, but it's always there:

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

See [`ferogram-tl-types`](../ferogram-tl-types/) for all generated types and functions.

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

## Shutdown

```rust
let (client, shutdown) = Client::builder()...connect().await?;

shutdown.cancel();   // graceful
client.disconnect(); // immediate
```

## Features

See **[FEATURES.md](../FEATURES.md)** for the full list with method signatures. A few things worth calling out explicitly:

- Bot and user account in the same API, same client builder
- Dispatcher with composable filters and middleware pipeline
- FSM with pluggable storage for multi-step conversations
- FakeTLS and Obfuscated2 transport for censored regions, full MTProxy support
- Resilient connect: DoH resolver + Telegram special-config fallback when TCP is blocked
- Transport probing: races multiple transports, uses whichever connects first
- Update gap recovery: PTS/QTS tracking, fetches missed updates via `getDifference` on reconnect
- QR code login for user accounts
- Concurrent upload and download with per-part retry and CDN redirect handling
- Turso/LibSQL session backend for serverless and distributed setups
- Forum topics (supergroups with topics enabled): create, edit, delete
- Inline button click from code (`msg.click_button`, `msg.click_button_where`)
- Scheduled messages: send, list, delete, send immediately
- Paid reactions and custom emoji reactions
- Python bindings via [ferogram-py](https://github.com/ankit-chaubey/ferogram-py), pre-built wheels, no Rust toolchain needed

Secret chats (end-to-end encrypted) are fully implemented but not published to crates.io yet. The plan is to release once there is enough community demand for it.

## Voice and video calls

Group audio calls are fully implemented, stable, and already in active production use. Written in Rust from scratch, not a wrapper around anything, which keeps it lightweight and efficient.

Group video calls are implemented and stable for most scenarios, with some known codec edge cases still being ironed out.

Peer-to-peer calls are partially implemented and still in active development.

All of this lives in its own workspace crate and will be published separately when it comes out of the workspace. Python bindings via ferogram-py are also planned for it.

## Crates

Most people only need this crate. But each crate in the workspace is independently publishable if you need just one layer.

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

The rough dependency chain (build-critical path only):

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

Read [CONTRIBUTING.md](../CONTRIBUTING.md) before opening a PR. Run `cargo fmt --all`, `cargo test --workspace`, and `cargo clippy --workspace` first. Security issues: see [SECURITY.md](../SECURITY.md).

## Acknowledgments

Big shoutout to [Lonami](https://codeberg.org/Lonami/grammers) for grammers. It was one of the most helpful references while building ferogram, and grammers and Telethon are two of my all-time favorites. Love those projects.

Protocol behavior references from [Telegram Desktop](https://github.com/telegramdesktop/tdesktop) and [TDLib](https://github.com/tdlib/td).

## License

MIT OR Apache-2.0. See [LICENSE-MIT](../LICENSE-MIT) and [LICENSE-APACHE](../LICENSE-APACHE).

Usage must comply with [Telegram's API Terms of Service](https://core.telegram.org/api/terms).
