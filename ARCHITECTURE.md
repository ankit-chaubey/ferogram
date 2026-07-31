# Architecture

Ferogram is a Cargo workspace. Most users only need the [`ferogram`](https://crates.io/crates/ferogram) crate;
everything else exists to support it and can be pulled in on its own if you need a
specific layer.

- [Crates](#crates)
- [Dependency chain](#dependency-chain)
- [Message box](#message-box)
- [Feature flags](#feature-flags)
- [Dispatcher and filters](#dispatcher-and-filters)
- [Middleware](#middleware)
- [FSM](#fsm)
- [Session backends](#session-backends)
- [Transport and proxy](#transport-and-proxy)
- [Error handling and shutdown](#error-handling-and-shutdown)
- [Python bindings](#python-bindings)
- [Building and testing](#building-and-testing)

## Crates

| Crate | What it does |
|---|---|
| [`ferogram`](ferogram/) | High-level client. Auth, messaging, media, dispatcher, FSM, middleware. |
| [`ferogram-msgbox`](ferogram-msgbox/) | Update-gap tracking. pts/qts/seq bookkeeping and gap detection for the `Updates` stream. |
| [`ferogram-session`](ferogram-session/) | Session types and pluggable storage backends (file, memory, SQLite, LibSQL, base64). |
| [`ferogram-fsm`](ferogram-fsm/) | FSM state storage and context. `StateStorage` trait, `MemoryStorage`, `StateContext`. |
| [`ferogram-parsers`](ferogram-parsers/) | Telegram Markdown and HTML entity parsers. |
| [`ferogram-derive`](ferogram-derive/) | `#[derive(FsmState)]` proc macro. |
| [`ferogram-mtsender`](ferogram-mtsender/) | DC connection pool and retry policy. `AutoSleep`, `NoRetries`, `CircuitBreaker`. |
| [`ferogram-connect`](ferogram-connect/) | Raw TCP, MTProto framing, obfuscation, SOCKS5, MTProxy, gzip. |
| [`ferogram-mtproto`](ferogram-mtproto/) | MTProto 2.0 session, DH key exchange, message framing, PFS key binding. |
| [`ferogram-crypto`](ferogram-crypto/) | AES-IGE, RSA, SHA, Diffie-Hellman, PQ factorization, auth key derivation. |
| [`ferogram-tl-types`](ferogram-tl-types/) | Auto-generated TL types, functions, and enums (tracks `tl::LAYER`). |
| [`ferogram-tl-gen`](ferogram-tl-gen/) | Build-time code generator from TL AST to Rust source. |
| [`ferogram-tl-parser`](ferogram-tl-parser/) | Parses `.tl` schema text into a Definition AST. |

Each crate is independently published and versioned together with the rest of the
workspace. Depend on one directly if you're building something narrower than a full
client; `ferogram` re-exports the ones most people end up needing anyway, so day to
day code rarely has to reach past it.

## Dependency chain

Build-critical path only:

```
ferogram
├ ferogram-msgbox              <-- used directly, not layered under mtsender/connect
│ └ ferogram-tl-types
└ ferogram-mtsender
  └ ferogram-connect
    ├ ferogram-mtproto
    │ ├ ferogram-tl-types
    │ │ └ (build) ferogram-tl-gen
    │ │   └ (build) ferogram-tl-parser
    │ └ ferogram-crypto
    └ ferogram-crypto
```

`ferogram-tl-types` sits at the bottom of the runtime graph but is regenerated at
build time from the TL schema, so a layer bump only touches the generated output,
not hand-written code further up the chain.

## Message box

Telegram's `Updates` stream is lossy over the wire. Pushed updates can drop or
arrive out of order, but every update carries a `pts`/`qts`/`seq` counter, so a
client can tell exactly when it missed something and ask for a diff to fill the
hole. [`ferogram-msgbox`](ferogram-msgbox/) is the state machine that does that
bookkeeping, re-exported as `ferogram::message_box`.

It tracks the global `pts`/`qts`/`seq`/`date` counters plus a separate `pts` per
channel, detects gaps by checking whether an incoming `pts_count` chains onto the
last known `pts`, and buffers out-of-order updates for a short window before
requesting a diff. It also decides when a periodic catch-up `getDifference` is due
even with no known gap, and classifies your own outgoing RPC responses so updates
piggybacked on them (like `sendMessage` returning the new message) fold into the
same pts sequence.

It's a pure state machine: no async, no networking, no RPC calls. Feed it updates
through `process_updates()` and it hands back either the update batch or a `Gap`,
which tells the caller which RPC to run and how to feed the result back:

```rust
use ferogram_msgbox::{MessageBoxes, UpdatesLike};

let mut mbox = MessageBoxes::new();

match mbox.process_updates(UpdatesLike::Updates(Box::new(incoming))) {
    Ok((updates, users, chats)) => {
        // dispatch each Vec's items as usual
    }
    Err(_gap) => {
        if let Some(req) = mbox.get_difference() {
            let diff = client.invoke(&req).await?;
            let (updates, users, chats) = mbox.apply_difference(diff);
            // dispatch updates/users/chats
        }
    }
}

// Called periodically, even with no known gap. Handles the 15-minute
// no-updates safety net and any pending per-channel diffs.
let deadline = mbox.check_deadlines();
```

On reconnect, feed `UpdatesLike::ConnectionClosed` in. Anything sent while the
socket was down, or before you were listening, is exactly the kind of gap this
is built to catch.

## Feature flags

```toml
ferogram = { version = "0.6.5", features = [
    "sqlite-session",         # SqliteBackend via rusqlite
    "libsql-session",         # LibSqlBackend, local file or in-memory, via libsql
    "libsql-remote-session",  # LibSqlBackend remote Turso + embedded replicas
    "html",                   # parse_html / generate_html (built-in parser)
    "html5ever",              # parse_html via spec-compliant html5ever
    "derive",                 # #[derive(FsmState)]
    "serde",                  # serde support on session types
    "fsm",                    # dp.on_message_fsm and the FSM dispatcher helpers
    "resilient-connect",      # DoH + special-config fallback for censored networks
    "socks5",                 # SOCKS5 proxy support for outgoing connections
    "metrics",                # RPC/connection counters, histograms, gauges
] }
```

`open_remote` and `open_replica` (session backends) need `libsql-remote-session` on top of
`libsql-session`, and `libsql-session` can't be combined with `sqlite-session` (both link a
sqlite3 C source). Everything above is off by default; the default build is just login,
raw RPC, and updates.

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

Filters compose with `&`, `|`, `!`. Built-ins cover `command`, `private`, `group`, `channel`, `text`, `text_contains`, `media`, `photo`, `document`, `forwarded`, `reply`, `from_user`, `album`, `custom`, and more. Callback queries and inline queries route through the same dispatcher via `on_callback_query` / `on_inline_query` / `on_inline_send`.

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

`MemoryStorage` is built in. To persist state across restarts, implement `StateStorage` for Redis, a database, or anything else. State keys scope per-user, per-chat, or per-user-in-chat via `StateKeyStrategy`. See [`ferogram-fsm`](ferogram-fsm/) for details.

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

The base64 string backend is useful for serverless or containers where writing to disk isn't an option. To bring your own backend, implement `SessionBackend` from [`ferogram-session`](ferogram-session/).

Under the hood, a session stores the DC address table with per-DC auth keys, salts, and capability flags, the MTProto update counters (pts, qts, seq, date, and per-channel pts), and a peer access-hash cache for users, channels, groups, and communities. The binary format is versioned: `load()` understands all previous versions, `save()` always writes the current one, and saves are atomic (written to a `.tmp` file first, then renamed into place).

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

[`ferogram-connect`](ferogram-connect/) is the framing layer underneath. It sits between a raw `TcpStream` and decrypted MTProto messages, and handles Abridged, Intermediate, Padded Intermediate, and Full transport framing, Obfuscated2 AES-256-CTR for bypassing DPI, FakeTLS for `0xee` MTProxy secrets, SOCKS5 with optional auth, and keepalive pings.

## Error handling and shutdown

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

Retry behavior itself lives in [`ferogram-mtsender`](ferogram-mtsender/): a `DcPool` holds one `DcConnection` per DC, created on demand, and the retry policy trait ships with `AutoSleep` (jittered backoff, auto-sleeps on `FLOOD_WAIT`/`SLOWMODE_WAIT`), `NoRetries`, and `CircuitBreaker` built in.

Shutdown:

```rust
let (client, shutdown) = Client::builder()...connect().await?;

shutdown.cancel();   // graceful: drains in-flight work, then disconnects
client.disconnect(); // immediate: drops the connection now
```

## Python bindings

[ferogram-py](https://github.com/ankit-chaubey/ferogram-py) wraps the Rust core in a Python package with the same design goal as the rest of the workspace: keep the compiled surface small and put everything else where it's easy to change.

![architecture](https://github.com/ankit-chaubey/ferogram-py/blob/main/assets/architecture.svg)

The compiled extension (`_ferogram.so`) is built with PyO3 and maturin, and stays deliberately thin. Networking, encryption, session storage, and MTProto internals all live in Rust, calling straight into the same `ferogram` crate this workspace produces. Everything you touch day to day, the client, handlers, and filters, is plain Python and can change without a recompile.

Tracing crosses the boundary too. Rust-side events are bridged into Python's logging through `pyo3-log`, with colored output and per-module caching so repeated log calls don't pay for a fresh lookup every time.

<details>
<summary>Prebuilt wheels for your platform</summary>

Wheels ship prebuilt for Linux (x86_64, aarch64), macOS (x86_64, arm64), Windows (x86_64), and Android/Termux (aarch64, x86_64). `pip install ferogram` grabs the right one on its own, no Rust toolchain required.

</details>

```bash
pip install ferogram
```

```python
from ferogram import Client

client = Client("my.session", api_id=API_ID, api_hash=API_HASH)

@client.on_message()
async def handler(msg):
    await msg.reply("Hello from ferogram!")

client.run()
```

## Building and testing

See [CONTRIBUTING.md](CONTRIBUTING.md) for build, test, and lint commands, plus the full development workflow and PR checklist.
