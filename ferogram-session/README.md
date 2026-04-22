# ferogram-session

Session persistence types and pluggable storage backends for ferogram.

[![Crates.io](https://img.shields.io/crates/v/ferogram-session?color=fc8d62)](https://crates.io/crates/ferogram-session)
[![Telegram](https://img.shields.io/badge/community-%40FerogramChat-2CA5E0?logo=telegram)](https://t.me/FerogramChat) [![Channel](https://img.shields.io/badge/channel-%40Ferogram-2CA5E0?logo=telegram)](https://t.me/Ferogram)
[![docs.rs](https://img.shields.io/badge/docs.rs-ferogram--session-5865F2)](https://docs.rs/ferogram-session)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](#license)

Session persistence for ferogram. Extracted in v0.3.0 so that session storage can be used, extended, or replaced without pulling in the full client.

`ferogram` re-exports everything from here  existing code needs no changes.

---

## Installation

```toml
[dependencies]
ferogram-session = "0.3"
```

---

## What it stores

- DC address table with per-DC auth keys, salts, and capability flags
- MTProto update counters: pts, qts, seq, date, and per-channel pts
- Peer access-hash cache for users, channels, and groups
- Min-user message contexts for `InputPeerUserFromMessage`

The binary format is versioned. `load()` handles all previous versions without error. `save()` always writes the current version. Saves are atomic: written to a `.tmp` file first, then renamed into place.

---

## Session Types

### PersistedSession

The main serializable struct. Holds the full DC table, update state, and peer cache.

```rust
use ferogram_session::PersistedSession;
use std::path::Path;

let session = PersistedSession::load(Path::new("my.session"))?;
session.save(Path::new("my.session"))?;

// String (base64) round-trip
let s = session.to_string();
let session2 = PersistedSession::from_string(&s)?;
```

### DcEntry and DcFlags

```rust
use ferogram_session::{DcEntry, DcFlags};

let entry = DcEntry::from_parts(2, "149.154.167.51", 443, DcFlags::NONE);
let ipv6  = DcEntry::from_parts(2, "2001:b28:f23d:f001::a", 443, DcFlags::IPV6);
```

---

## Backends

### BinaryFileBackend

Stores the session as a binary file on disk. Default backend used by `ferogram`.

```rust
use ferogram_session::BinaryFileBackend;

let backend = BinaryFileBackend::new("ferogram.session");
```

### InMemoryBackend

Stores the session in memory only. Useful for testing or short-lived bots.

```rust
use ferogram_session::InMemoryBackend;

let backend = InMemoryBackend::new();
```

### StringSessionBackend

Stores the session as a base64 string. Useful for deployments where file I/O is unavailable (e.g. environment variable sessions).

```rust
use ferogram_session::StringSessionBackend;

let backend = StringSessionBackend::new(std::env::var("SESSION").unwrap_or_default());
```

### SqliteBackend (feature: `sqlite-session`)

```toml
ferogram-session = { version = "0.3", features = ["sqlite-session"] }
```

```rust
use ferogram_session::SqliteBackend;

let backend = SqliteBackend::open("sessions.db")?;
```

### LibSqlBackend (feature: `libsql-session`)

```toml
ferogram-session = { version = "0.3", features = ["libsql-session"] }
```

```rust
use ferogram_session::LibSqlBackend;

let backend = LibSqlBackend::open_local("sessions.db")?;
```

---

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

---

## Feature flags

| Flag | What it enables |
|---|---|
| `sqlite-session` | `SqliteBackend` via rusqlite |
| `libsql-session` | `LibSqlBackend` via libsql |
| `serde` | `Serialize`/`Deserialize` on session types |

---

## Stack position

```
ferogram
└ ferogram-session  <-- here
```

---

## License

MIT or Apache-2.0, at your option. See [LICENSE-MIT](../LICENSE-MIT) and [LICENSE-APACHE](../LICENSE-APACHE).

**Ankit Chaubey** - [github.com/ankit-chaubey](https://github.com/ankit-chaubey)
