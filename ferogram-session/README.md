# ferogram-session

Session persistence types and pluggable storage backends for ferogram.

[![Crates.io](https://img.shields.io/crates/v/ferogram-session?style=flat-square&logo=rust&logoColor=white&color=F97316)](https://crates.io/crates/ferogram-session)
[![docs.rs](https://img.shields.io/badge/docs.rs-ferogram--session-5865F2?style=flat-square&logo=docs.rs&logoColor=white)](https://docs.rs/ferogram-session)
[![License](https://img.shields.io/badge/License-MIT%20%7C%20Apache--2.0-64748B?style=flat-square)](#license)
[![Telegram Channel](https://img.shields.io/badge/Channel-Ferogram-06B6D4?style=flat-square&logo=telegram&logoColor=white)](https://t.me/Ferogram)
[![Telegram Chat](https://img.shields.io/badge/Chat-FerogramChat-06B6D4?style=flat-square&logo=telegram&logoColor=white)](https://t.me/FerogramChat)

Session persistence for ferogram. `ferogram` re-exports everything from here, so existing code needs no changes. You only need to depend on this directly if you're building something that uses session storage without the full client.

For installation instructions see the [ferogram README](https://github.com/ankit-chaubey/ferogram).

## What it stores

- DC address table with per-DC auth keys, salts, and capability flags (`DcFlags`: IPv6, media-only, TCP-obfuscated-only, CDN, static). Both IPv4 and IPv6 entries can be kept for the same DC, and `dc_for(dc_id, prefer_ipv6)` picks the right one.
- MTProto update counters: pts, qts, seq, date, and per-channel pts
- Peer access-hash cache for users, channels, groups, and Communities, tracked separately from channels via `is_community` so a Community is never mistaken for a supergroup on the wire
- Min-user message contexts for `InputPeerUserFromMessage`

The binary format is versioned. `load()` handles all previous versions. `save()` always writes the current version. Saves are atomic: written to a `.tmp` file first, then renamed into place.

Call `PersistedSession::stats()` for a breakdown (peer counts by kind, DCs with negotiated keys, approximate serialized size). Useful for spotting a bloated `min_peers` cache before it needs pruning.

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
let backend = SqliteBackend::in_memory()?; // tests, no disk
```

### LibSqlBackend (feature: `libsql-session`)

Local file or in-process in-memory, no remote server needed:

```rust
use ferogram_session::LibSqlBackend;
let backend = LibSqlBackend::open_local("sessions.db")?;
let backend = LibSqlBackend::in_memory()?; // tests, no disk
```

Remote Turso and embedded replicas need the `libsql-remote-session` feature on top of `libsql-session`:

```rust
use ferogram_session::LibSqlBackend;

// Talk to a remote Turso database directly.
let backend = LibSqlBackend::open_remote(url, auth_token)?;

// Local file that stays synced with a remote Turso database.
// Reads hit the local replica; writes sync to the remote.
let backend = LibSqlBackend::open_replica("sessions.db", url, auth_token)?;
```

`sqlite-session` and `libsql-remote-session` can't be enabled together, both pull in a sqlite3 C source and the build fails at link time. Pick one.

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
| `libsql-session` | `LibSqlBackend`, local file or in-memory, via libsql |
| `libsql-remote-session` | Remote Turso and embedded replicas on `LibSqlBackend` (implies `libsql-session`). Not compatible with `sqlite-session`. |
| `serde` | `Serialize`/`Deserialize` on session types |

## Stack position

```
ferogram
└ ferogram-session  <-- here
```

## License

This project is licensed under either the MIT License or Apache License 2.0, at your option. See [`LICENSE-MIT`](https://github.com/ankit-chaubey/ferogram/blob/main/LICENSE-MIT) and [`LICENSE-APACHE`](https://github.com/ankit-chaubey/ferogram/blob/main/LICENSE-APACHE) for details.

**Author:** Ankit Chaubey ([@ankit-chaubey](https://github.com/ankit-chaubey))
