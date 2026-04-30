

# Installation

## Add to Cargo.toml

```toml
[dependencies]
ferogram = "0.3.6"
tokio        = { version = "1", features = ["full"] }
```

`ferogram` re-exports everything you need for both user clients and bots.

---

## Getting API credentials

Every Telegram API call requires an `api_id` (integer) and `api_hash` (hex string) from your registered app.

**Step-by-step:**

1. Go to **[https://my.telegram.org](https://my.telegram.org)** and log in with your phone number
2. Click **API development tools**
3. Fill in any app name, short name, platform (Desktop), and URL (can be blank)
4. Click **Create application**
5. Copy `App api_id` and `App api_hash`

> **SECURITY:** Never hardcode credentials in source code. Use environment variables or a secrets file that is in `.gitignore`.

```rust
// Good: from environment
let api_id:   i32    = std::env::var("TG_API_ID")?.parse()?;
let api_hash: String = std::env::var("TG_API_HASH")?;

// Bad: hardcoded in source
let api_id   = 12345;
let api_hash = "deadbeef..."; // ← never do this in a public repo
```

---

## Bot token (bots only)

For bots, additionally get a **bot token** from [@BotFather](https://t.me/BotFather):

1. Open Telegram → search `@BotFather` → `/start`
2. Send `/newbot`
3. Choose a display name (e.g. "My Awesome Bot")
4. Choose a username ending in `bot` (e.g. `my_awesome_bot`)
5. Copy the token: `1234567890:ABCdefGHIjklMNOpqrSTUvwxYZ`

---

## Optional features

### SQLite session storage

```toml
ferogram = { version = "0.3.6", features = ["sqlite-session"] }
```

Stores session data in a SQLite database instead of a binary file. More robust for long-running servers.

### LibSQL / Turso session storage: New in v0.2.0

```toml
ferogram = { version = "0.3.6", features = ["libsql-session"] }
```

Backed by [libsql](https://github.com/tursodatabase/libsql): supports local embedded databases and remote Turso cloud databases. Ideal for serverless or distributed deployments.

```rust
use ferogram::session_backend::LibSqlBackend;

// Local
let backend = LibSqlBackend::open_local("session.libsql").await?;

// Remote (Turso cloud)
let backend = LibSqlBackend::open_remote(
    "libsql://your-db.turso.io",
    "your-turso-auth-token",
).await?;
```

### String session (portable, no extra deps): New in v0.2.0

No feature flag needed. Encode a session as a base64 string and restore it anywhere:

```rust
// Export
let s = client.export_session_string().await?;

// Restore
let (client, _shutdown) = Client::builder()
    .api_id(api_id)
    .api_hash(api_hash)
    .session_string(s)
    .connect()
    .await?;
```

See [Session Backends](./authentication/session-backends.md) for the full guide.

### HTML entity parsing

```toml
# Built-in hand-rolled HTML parser (no extra deps)
ferogram = { version = "0.3.6", features = ["html"] }

# OR: spec-compliant html5ever tokenizer (overrides built-in)
ferogram = { version = "0.3.6", features = ["html5ever"] }
```

| Feature | Deps added | Notes |
|---|---|---|
| `html` | none | Fast, minimal, covers common Telegram HTML tags |
| `html5ever` | `html5ever` | Full spec-compliant tokenizer; use when parsing arbitrary HTML |

### Raw type system features (`ferogram-tl-types`)

If you use `ferogram-tl-types` directly for raw API access:

```toml
ferogram-tl-types = { version = "0.3.6", features = [
    "tl-api",          # Telegram API types (required)
    "tl-mtproto",      # Low-level MTProto types
    "impl-debug",      # Debug trait on all types (default ON)
    "impl-from-type",  # From<types::T> for enums::E (default ON)
    "impl-from-enum",  # TryFrom<enums::E> for types::T (default ON)
    "name-for-id",     # name_for_id(u32) -> Option<&'static str>
    "impl-serde",      # serde::Serialize / Deserialize
] }
```

---

## Verifying installation

```rust
use ferogram_tl_types::LAYER;

fn main() {
    println!("Using Telegram API Layer {}", LAYER);
    // → Using Telegram API Layer 224
}
```

---

## Platform notes

| Platform | Status | Notes |
|---|---|---|
| Linux x86_64 | ✅ Fully supported | |
| macOS (Apple Silicon + Intel) | ✅ Fully supported | |
| Windows | ✅ Supported | Use WSL2 for best experience |
| Android (Termux) | ✅ Works | Native ARM64 |
| iOS | ⚠️ Untested | No async runtime constraints known |
