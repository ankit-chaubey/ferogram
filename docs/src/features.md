# Feature Flags

## ferogram

| Feature | Default | Description |
|---|---|---|
| `sqlite-session` | ❌ | SQLite-backed session storage via `rusqlite` |
| `libsql-session` | ❌ | libsql / Turso session storage: local or remote (**Added in v0.3.0**) |
| `parsers` | ❌ | HTML and Markdown entity parsers (`parse_html`, `generate_html`, `parse_markdown`) |
| `html` | ❌ | Alias of `parsers` |
| `html5ever` | ❌ | Spec-compliant html5ever tokenizer: overrides the built-in HTML parser |
| `serde` | ❌ | `serde::Serialize` / `Deserialize` on `Config` and public structs |
| `derive` | ❌ | `#[derive(FsmState)]` proc-macro for FSM state enums |
| `fsm` | ❌ | FSM dispatcher helpers (`dp.on_message_fsm`, `FsmState`) |
| `experimental` | ❌ | Experimental transfer APIs, such as resumable transfers |
| `libsql-remote-session` | ❌ | `LibSqlBackend` over remote Turso and embedded replicas (`open_remote`, `open_replica`) |
| `resilient-connect` | ❌ | DNS-over-HTTPS and special-config fallback for censored networks |
| `socks5` | ❌ | SOCKS5 proxy support for outgoing connections |
| `metrics` | ❌ | RPC and connection counters, histograms, gauges |
| `e2e` | ❌ | Secret chats (end-to-end encryption). Re-exports `ferogram-e2e` as `ferogram::e2e` |
| `parser` | ❌ | Re-exports `ferogram-tl-parser` for custom TL schema tooling |
| `codegen` | ❌ | Re-exports `ferogram-tl-gen` for custom code generation tooling |

The following are **always available** without any feature flag:
- `InlineKeyboard`, `ReplyKeyboard`, `Button`: keyboard builders
- `InputReactions`: reaction builder
- `TypingGuard`: RAII typing indicator
- `SearchBuilder`, `GlobalSearchBuilder`: fluent search
- `PeerRef`: flexible peer argument
- `User`, `Group`, `Channel`, `Chat`: typed wrappers
- `Socks5Config`: SOCKS5 proxy config
- `BannedRightsBuilder`, `AdminRightsBuilder`, `ParticipantPermissions`
- `StringSessionBackend`, `InMemoryBackend`, `BinaryFileBackend`
- `ClientBuilder`: fluent connection builder

```toml
# SQLite session only
ferogram = { version = "0.6.7", features = ["sqlite-session"] }

# LibSQL / Turso session (new in 0.2.0)
ferogram = { version = "0.6.7", features = ["libsql-session"] }

# HTML parsing (minimal, no extra deps)
ferogram = { version = "0.6.7", features = ["html"] }

# HTML parsing (spec-compliant, adds html5ever dep)
ferogram = { version = "0.6.7", features = ["html5ever"] }

# Secret chats (end-to-end encryption)
ferogram = { version = "0.6.7", features = ["e2e"] }

# Multiple features at once
ferogram = { version = "0.6.7", features = ["sqlite-session", "html"] }
```

---

## ferogram-tl-types

| Feature | Default | Description |
|---|---|---|
| `tl-api` | ✅ | Telegram API schema (constructors, functions, enums) |
| `tl-mtproto` | ❌ | Low-level MTProto transport types |
| `impl-debug` | ✅ | `#[derive(Debug)]` on all generated types |
| `impl-from-type` | ✅ | `From<types::T> for enums::E` conversions |
| `impl-from-enum` | ✅ | `TryFrom<enums::E> for types::T` conversions |
| `deserializable-functions` | ❌ | `Deserializable` for function types (server-side use) |
| `name-for-id` | ❌ | `name_for_id(id: u32) -> Option<&'static str>` |
| `impl-serde` | ❌ | `serde::Serialize` + `serde::Deserialize` on all types |

### Example: enable serde

```toml
ferogram-tl-types = { version = "0.6.7", features = ["tl-api", "impl-serde"] }
```

```rust
let json = serde_json::to_string(&some_tl_type)?;
```

### Example: name_for_id (debugging)

```toml
ferogram-tl-types = { version = "0.6.7", features = ["tl-api", "name-for-id"] }
```

```rust
use ferogram_tl_types::name_for_id;

if let Some(name) = name_for_id(0x74ae4240) {
    println!("Constructor: {name}"); // → "updates"
}
```

### Example: minimal (no Debug, no conversions)

```toml
ferogram-tl-types = { version = "0.6.7", default-features = false, features = ["tl-api"] }
```

Reduces compile time when you don't need convenience traits.

---

## String session: no feature flag needed

`StringSessionBackend` and `export_session_string()` are available in the default build: no feature flag required:

```toml
ferogram = "0.6.7"   # already includes StringSessionBackend
```

```rust
let s = client.export_session_string().await?;
let (client, _) = Client::with_string_session(&s, api_id, api_hash).await?;
```

---

## docs.rs build matrix

When building docs on docs.rs, all feature flags are enabled:

```toml
[package.metadata.docs.rs]
features = ["sqlite-session", "libsql-session", "serde", "html", "html5ever", "parser", "codegen"]
rustdoc-args = ["--cfg", "docsrs"]
```
