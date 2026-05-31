# ferogram-session

Session persistence types and pluggable storage backends for ferogram.

[![Crates.io](https://img.shields.io/crates/v/ferogram-session?color=fc8d62)](https://crates.io/crates/ferogram-session)
[![Telegram](https://img.shields.io/badge/community-%40FerogramChat-2CA5E0?logo=telegram)](https://t.me/FerogramChat) [![Channel](https://img.shields.io/badge/channel-%40Ferogram-2CA5E0?logo=telegram)](https://t.me/Ferogram)
[![docs.rs](https://img.shields.io/badge/docs.rs-ferogram--session-5865F2)](https://docs.rs/ferogram-session)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](#license)

Session persistence for ferogram. `ferogram` re-exports everything from here, so existing code needs no changes. You only need to depend on this directly if you're building something that uses session storage without the full client.

For installation instructions see the [ferogram README](https://github.com/ankit-chaubey/ferogram).

## What it stores

- DC address table with per-DC auth keys, salts, and capability flags
- MTProto update counters: pts, qts, seq, date, and per-channel pts
- Peer access-hash cache for users, channels, and groups
- Min-user message contexts for `InputPeerUserFromMessage`

The binary format is versioned. `load()` handles all previous versions. `save()` always writes the current version. Saves are atomic: written to a `.tmp` file first, then renamed into place.

## String Sessions

Two formats are supported. Both are accepted by `Client::builder().session_string("...")` which auto-detects the format.

### Compact (V1/V2)

Exported by `client.export_session_string()`. Encodes dc_id, ip, port, user_id, and auth key only. Good for serverless or portable deployments.

```rust
let s = client.export_session_string().await?;
Client::builder().session_string(&s).connect().await?;
```

### Native (full state)

Exported by `client.export_native_session_string()`. Includes the full DC table, update counters (PTS, QTS, seq), and peer cache. Use when you need to resume update processing from exactly where you left off.

```rust
let s = client.export_native_session_string().await?;
Client::builder().session_string(&s).connect().await?;
```

## Backends

### BinaryFileBackend

Default. Saves the session as a binary file on disk.

```rust
use ferogram_session::BinaryFileBackend;
let backend = BinaryFileBackend::new("ferogram.session");
```

### InMemoryBackend

No persistence, lives only for the process lifetime. Good for tests or quick scripts.

```rust
use ferogram_session::InMemoryBackend;
let backend = InMemoryBackend::new();
```

### StringSessionBackend

Stores the session as a base64 string. Useful when you can't write to disk.

```rust
use ferogram_session::StringSessionBackend;
let backend = StringSessionBackend::new(std::env::var("SESSION").unwrap_or_default());
```

### SqliteBackend (feature: `sqlite-session`)

```rust
use ferogram_session::SqliteBackend;
let backend = SqliteBackend::open("sessions.db")?;
```

### LibSqlBackend (feature: `libsql-session`)

```rust
use ferogram_session::LibSqlBackend;
let backend = LibSqlBackend::open_local("sessions.db")?;
```

## Custom Backends

Implement `SessionBackend` to add your own storage:

```rust
use ferogram_session::{SessionBackend, PersistedSession};
use std::io;

struct RedisBackend { /* ... */ }

impl SessionBackend for RedisBackend {
    fn save(&self, session: &PersistedSession) -> io::Result<()> { todo!() }
    fn load(&self) -> io::Result<Option<PersistedSession>> { todo!() }
    fn delete(&self) -> io::Result<()> { todo!() }
    fn name(&self) -> &str { "redis" }
}
```

## Feature flags

| Flag | What it enables |
|---|---|
| `sqlite-session` | `SqliteBackend` via rusqlite |
| `libsql-session` | `LibSqlBackend` via libsql |
| `serde` | `Serialize`/`Deserialize` on session types |

## Stack position

```
ferogram
└ ferogram-session  <-- here
```

## License

MIT or Apache-2.0, at your option. See [LICENSE-MIT](../LICENSE-MIT) and [LICENSE-APACHE](../LICENSE-APACHE).

**Ankit Chaubey** - [github.com/ankit-chaubey](https://github.com/ankit-chaubey)
