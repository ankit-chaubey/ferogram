# Session Persistence

A session stores your auth key, DC address, and peer access-hash cache. Without it, you'd need to log in on every run.

## Binary file (default)

```rust
use ferogram::{Client, Config};

let (client, _shutdown) = Client::builder()
        .api_id(12345)
        .api_hash("abc123")
        .session("my.session")
        .connect().await?.await?;
```

After login, save to disk:

```rust
client.save_session().await?;
```

The file is created at `session_path` and reloaded automatically on the next `Client::connect`. **Keep it in `.gitignore`: it grants full API access to your account.**

---

## In-memory (ephemeral)

Nothing written to disk. Useful for tests or short-lived scripts:

```rust
use ferogram::session_backend::InMemoryBackend;

let (client, _shutdown) = Client::builder()
    .session(InMemoryBackend::new())
    .api_id(12345)
    .api_hash("abc123")
    .connect()
    .await?;
```

Login is required on every run since nothing persists.

---

## SQLite (robust, long-running servers)

```toml
ferogram = { version = "0.1.1", features = ["sqlite-session"] }
```

```rust
let (client, _shutdown) = Client::connect(Config {
    ..Default::default()
}).await?;
```

SQLite is more resilient against crash-corruption than the binary format. Ideal for production bots.

---

## String session: New in v0.1.1

Encode the entire session as a portable base64 string. Store it in an env var, a DB column, or CI secrets:

```rust
// Export (after login)
let s = client.export_session_string().await?;
// → "AQAAAAEDAADtE1lMHBT7...=="

// Restore
let (client, _shutdown) = Client::with_string_session(
    &s, api_id, api_hash,
).await?;

// Or via builder
use ferogram::session_backend::StringSessionBackend;
let (client, _shutdown) = Client::builder()
    .session(StringSessionBackend::new(&s))
    .api_id(api_id)
    .api_hash(api_hash)
    .connect()
    .await?;
```

See [Session Backends](./session-backends.md) for the full guide including LibSQL (Turso) backend.

---

## What's stored in a session

| Field | Description |
|---|---|
| Auth key | 2048-bit DH-derived key for encryption |
| Auth key ID | Hash of the key, used as identifier |
| DC ID | Which Telegram data center to connect to |
| DC address | The IP:port of the DC |
| Server salt | Updated regularly by Telegram |
| Sequence numbers | For message ordering |
| Peer cache | User/channel access hashes (speeds up API calls) |

---

## Security

> **SECURITY:** A stolen session file gives full API access to your account. Protect it like a password.

- Add to `.gitignore`: `*.session`, `*.session.db`
- Set restrictive permissions: `chmod 600 my.session`
- Never log or print session file contents
- If compromised: revoke from **Telegram → Settings → Devices → Terminate session**

---

## Multi-session / multi-account

Each `Client::connect` loads one session. For multiple accounts, use multiple files:

```rust
let (client_a, _) = Client::connect(Config {
    api_id, api_hash: api_hash.clone(), ..Default::default()
}).await?;

let (client_b, _) = Client::connect(Config {
    api_id, api_hash: api_hash.clone(), ..Default::default()
}).await?;
```
