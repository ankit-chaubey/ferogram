<div align="center">

# ferogram

A Native & Elegant MTProto Framework for Rust

[![Crates.io](https://img.shields.io/crates/v/ferogram?style=flat-square&logo=rust&logoColor=white&color=F97316)](https://crates.io/crates/ferogram)
[![docs.rs](https://img.shields.io/badge/docs.rs-ferogram-5865F2?style=flat-square&logo=docs.rs&logoColor=white)](https://docs.rs/ferogram)
[![License](https://img.shields.io/badge/License-MIT%20%7C%20Apache--2.0-64748B?style=flat-square)](../LICENSE-MIT)
[![TL Layer](https://img.shields.io/badge/TL%20Layer-228-8B5CF6?style=flat-square)](https://core.telegram.org/schema)
[![Telegram Channel](https://img.shields.io/badge/Channel-Ferogram-06B6D4?style=flat-square&logo=telegram&logoColor=white)](https://t.me/Ferogram)
[![Telegram Chat](https://img.shields.io/badge/Chat-FerogramChat-06B6D4?style=flat-square&logo=telegram&logoColor=white)](https://t.me/FerogramChat)

Built by **[Ankit Chaubey](https://github.com/ankit-chaubey)**

</div>

This is the main client crate. It talks to Telegram directly over MTProto and handles auth for both bots and user accounts from the same client builder. You get a dispatcher with composable filters, FSM for multi-step conversations, CDN downloads, middleware, MTProxy support and a raw `invoke()` escape hatch for anything not wrapped yet.

If you're starting fresh, this is the only crate you need. Everything else in the workspace exists to support it and can be pulled in separately if you need a specific layer on its own.

<details>
<summary><b>Contents</b></summary>

- [Installation](#installation)
- [Quick start: bot](#quick-start-bot)
- [Quick start: user account](#quick-start-user-account)
- [Examples](#examples)
- [Connecting](#connecting)
- [Session backends](#session-backends)
- [Transport and proxy](#transport-and-proxy)
- [Raw API](#raw-api)
- [Error handling](#error-handling)
- [Shutdown](#shutdown)
- [What's covered](#whats-covered)
- [Voice and video calls](#voice-and-video-calls)
- [Crates](#crates)
- [Community](#community)
- [License](#license)

</details>

## Installation

```toml
[dependencies]
ferogram = "0.6.5"
tokio    = { version = "1.53", features = ["full"] }
```

## Quick start: bot

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

## Quick start: user account

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

## Examples

19 runnable examples covering everything from sending a message to a full FSM order bot.

See **[examples](examples/README.md)** for the full list with descriptions and notes on when to use `quick_connect` vs `Client::builder()`.

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

`.catch_up(true)` replays missed updates after a reconnect, and `.retry_policy(...)` / `.restart_policy(...)` let you customize retry and reconnect behavior.

## Session backends

```rust
Client::builder().session("bot.session")                                    // binary file (default)
Client::builder().in_memory()                                               // no persistence
Client::builder().session_string(env::var("SESSION")?)                     // base64 string
Client::builder().session_backend(Arc::new(SqliteBackend::open("s.db")?))  // sqlite
Client::builder().session_backend(Arc::new(LibSqlBackend::open_local("s.db")?))          // libsql, local file
Client::builder().session_backend(Arc::new(LibSqlBackend::open_remote(url, token)?))     // turso, remote only
Client::builder().session_backend(Arc::new(LibSqlBackend::open_replica("s.db", url, token)?)) // turso, local + synced
```

The base64 string backend is useful for serverless or containers where writing to disk isn't an option. `open_remote` and `open_replica` need the `libsql-remote-session` feature on top of `libsql-session`, and can't be combined with `sqlite-session` (both link a sqlite3 C source). To bring your own backend, implement `SessionBackend` from [`ferogram-session`](../ferogram-session/).

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

When the high-level API isn't enough, `client.invoke()` takes any TL function directly (current layer exposed as `tl::LAYER`). It's the escape hatch, not the normal path, but it's always there:

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

## What's covered

- **Rich Messaging**: text, media, albums, polls, dice, games, reactions, scheduled messages
- **HTML & Markdown**: full parse and generate support for both formats
- **Inline & Reply Keyboards**: buttons, callbacks, inline mode
- **CDN**: transparent CDN download handling, no extra calls needed
- **Proxy Support**: SOCKS5 with optional auth
- **MTProxy**: Classic, DD, and FakeTLS transports, via link or manual config
- **Transport Probing**: races transports, connects via whichever is fastest
- **Concurrent Transfers**: parallel uploads/downloads with pause, resume, cancel, and progress tracking
- **Resumable Transfers**: checkpointed uploads/downloads that survive crashes
- **Session Backends**: file, in-memory, string, SQLite, LibSQL
- **Router & Dispatcher**: composable filters (`&`, `|`, `!`) for expressive handlers
- **FSM**: type-safe finite state machine for multi-step conversations
- **Middleware**: rate limiting, tracing, panic recovery
- **TgCalls**: group calls, P2P calls, conference calls, screen share/presentation, audio and video
- **Raw API**: full TL coverage via `client.invoke()`
- **Python Bindings**: native performance with a clean Python API

...and more features like this throughout the codebase!

See **[features docs](../FEATURES.md)** for the full list with method signatures. If something is missing, open a feature request or suggest in [@FerogramChat](https://t.me/FerogramChat).

**Secret chats** (end-to-end encrypted) are fully implemented but not published to crates.io yet. The plan is to release once there is enough community demand for it.

## Voice and video calls

Group audio, video, and P2P calling are now fully implemented. To get started, check out the [tgcalls](https://crates.io/crates/tgcalls) crate and its examples in the [tgcalls repository](https://github.com/ankit-chaubey/tgcalls). It provides seamless integration between ferogram and the official [ntgcalls](https://crates.io/crates/ntgcalls) Rust bindings for building Telegram voice and video calling applications.

## Crates

Most people only need this crate. But each crate in the workspace is independently publishable if you need just one layer - see **[ARCHITECTURE](../ARCHITECTURE.md)** for the full breakdown and dependency graph.

## Community

- **Channel** (releases, announcements): [@Ferogram](https://t.me/Ferogram)
- **Chat** (questions, discussion): [@FerogramChat](https://t.me/FerogramChat)
- **Docs**: (docs & guidance):
[docs.ferogram.dev](https://docs.ferogram.dev)
- **Official Website**: (Projects & crates):
[ferogram.dev](https://ferogram.dev)
- **GitHub**: [github.com/ankit-chaubey/ferogram](https://github.com/ankit-chaubey/ferogram)

## License

This project is licensed under either the MIT License or Apache License 2.0, at your option. See [`LICENSE-MIT`](https://github.com/ankit-chaubey/ferogram/blob/main/LICENSE-MIT) and [`LICENSE-APACHE`](https://github.com/ankit-chaubey/ferogram/blob/main/LICENSE-APACHE) for details.

**Author:** Ankit Chaubey ([@ankit-chaubey](https://github.com/ankit-chaubey))
