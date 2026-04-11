# Session Backends

ferogram ships five session backends out of the box. They all implement the `SessionBackend` trait and are **hot-swappable**: switch by changing one line.

---

## Built-in backends

| Backend | Feature flag | Best for |
|---|---|---|
| `BinaryFileBackend` | *(default, no flag)* | Single-process bots, local scripts |
| `InMemoryBackend` | *(default, no flag)* | Tests, ephemeral tasks |
| `StringSessionBackend` | *(default, no flag)* | Serverless, env-var storage, CI bots |
| `SqliteBackend` | `sqlite-session` | Multi-session local apps |
| `LibSqlBackend` | `libsql-session` | Distributed / Turso-backed storage |

---

## `BinaryFileBackend` (default)

Saves the session as a binary file on disk. No feature flag needed.

```rust
use ferogram::Client;

let (client, _shutdown) = Client::builder()
    .api_id(12345)
    .api_hash("your_api_hash")
    .session("my.session")  // BinaryFileBackend at this path
    .connect()
    .await?;
```

Or construct directly:

```rust
use ferogram::session_backend::BinaryFileBackend;
use std::sync::Arc;

let backend = Arc::new(BinaryFileBackend::new("bot.session"));
```

---

## `InMemoryBackend`

Non-persistent: lost on process exit. Ideal for tests.

```rust
let (client, _shutdown) = Client::builder()
    .api_id(12345)
    .api_hash("your_api_hash")
    .in_memory()
    .connect()
    .await?;
```

Or construct directly:

```rust
use ferogram::session_backend::InMemoryBackend;
use std::sync::Arc;

let backend = Arc::new(InMemoryBackend::new());
```

---

## `StringSessionBackend`: portable auth

Encodes the entire session (auth key + DC + peer cache) as a single base64 string. Store it in an environment variable, a database column, or a secret manager.

### Export

```rust
let session_string = client.export_session_string().await?;
println!("{session_string}"); // store this somewhere safe
```

### Restore

```rust
// Via builder
let session = std::env::var("TG_SESSION").unwrap_or_default();

let (client, _shutdown) = Client::builder()
    .api_id(12345)
    .api_hash("your_api_hash")
    .session_string(session)
    .connect()
    .await?;
```

```rust
// Via Config shorthand
let (client, _shutdown) = Client::connect(Config::with_string_session(session_string))
    .await?;
```

```rust
// Construct backend directly
use ferogram::session_backend::StringSessionBackend;
use std::sync::Arc;

let backend = Arc::new(StringSessionBackend::new(session_string));
```

Pass an **empty string** to start a fresh session with no stored data.

---

## `SqliteBackend`: local database

Requires feature flag:

```toml
ferogram = { version = "0.1.1", features = ["sqlite-session"] }
```

```rust
use ferogram::SqliteBackend;
use std::sync::Arc;

let (client, _shutdown) = Client::builder()
    .api_id(12345)
    .api_hash("your_api_hash")
    .session_backend(Arc::new(SqliteBackend::new("sessions.db")))
    .connect()
    .await?;
```

The file is created if it doesn't exist.

---

## `LibSqlBackend`: libsql / Turso

Requires feature flag:

```toml
ferogram = { version = "0.1.1", features = ["libsql-session"] }
```

```rust
use ferogram::LibSqlBackend;
use std::sync::Arc;

// Local embedded database
let backend = LibSqlBackend::new("local.db");

// Remote Turso database
let backend = LibSqlBackend::remote(
    "libsql://your-db.turso.io",
    "your-turso-auth-token",
);

let (client, _shutdown) = Client::builder()
    .api_id(12345)
    .api_hash("your_api_hash")
    .session_backend(Arc::new(backend))
    .connect()
    .await?;
```

---

## Custom backend

Implement `SessionBackend` to use any storage: Redis, Postgres, S3, or anything else:

```rust
use ferogram::session_backend::{SessionBackend, PersistedSession, DcEntry, UpdateStateChange};
use std::io;

struct MyBackend {
    // your fields (e.g. a DB pool)
}

impl SessionBackend for MyBackend {
    fn save(&self, session: &PersistedSession) -> io::Result<()> {
        // Serialize and store session
        let bytes = serde_json::to_vec(session)
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
        // e.g. write to Redis / Postgres
        Ok(())
    }

    fn load(&self) -> io::Result<Option<PersistedSession>> {
        // Load and deserialize
        Ok(None)
    }

    fn delete(&self) -> io::Result<()> {
        Ok(())
    }

    fn name(&self) -> &str {
        "my-custom-backend"
    }

    // Optional: override granular methods for better performance
    // Default impls call load() → mutate → save()

    fn update_dc(&self, entry: &DcEntry) -> io::Result<()> {
        // UPDATE single DC row (e.g. SQL UPDATE)
        todo!()
    }

    fn set_home_dc(&self, dc_id: i32) -> io::Result<()> {
        // UPDATE home_dc column only
        todo!()
    }
}

// Use it
let (client, _shutdown) = Client::builder()
    .api_id(12345)
    .api_hash("your_api_hash")
    .session_backend(Arc::new(MyBackend { /* ... */ }))
    .connect()
    .await?;
```

### Granular `SessionBackend` methods

High-performance backends can override these to avoid full load/save round-trips:

| Method | When called | Default behaviour |
|---|---|---|
| `save(session)` | Any state change | Required |
| `load()` | On connect, and by default impls | Required |
| `delete()` | Session wipe | Required |
| `name()` | Logging/debug | Required |
| `update_dc(entry)` | After DH handshake on a new DC | `load → mutate → save` |
| `set_home_dc(dc_id)` | After a MIGRATE redirect | `load → mutate → save` |
| `apply_update_state(change)` | After update-sequence change | `load → mutate → save` |

---

## Using `ClientBuilder` to attach a backend

```rust
use std::sync::Arc;
use ferogram::{Client, session_backend::SessionBackend};

async fn connect_with_backend(
    backend: impl SessionBackend + 'static,
    api_id: i32,
    api_hash: &str,
) -> anyhow::Result<()> {
    let (client, _shutdown) = Client::builder()
        .api_id(api_id)
        .api_hash(api_hash)
        .session_backend(Arc::new(backend))
        .connect()
        .await?;

    // ...
    Ok(())
}
```

---

## Additional backend methods

### `StringSessionBackend::current()`

Reads the current serialised session string at any time:

```rust
use ferogram::session_backend::StringSessionBackend;

let backend = StringSessionBackend::new("");
// ... after connecting and authenticating ...
let s = backend.current(); // base64 session string
```

### `BinaryFileBackend::path()`

```rust
use ferogram::session_backend::BinaryFileBackend;

let backend = BinaryFileBackend::new("bot.session");
println!("Saving to: {}", backend.path().display());
```

### `InMemoryBackend::snapshot()`

Take a point-in-time snapshot of the in-memory session (useful in tests):

```rust
use ferogram::session_backend::InMemoryBackend;

let backend = InMemoryBackend::new();
// ... after auth ...
if let Some(session) = backend.snapshot() {
    // inspect or serialize session data
}
```
