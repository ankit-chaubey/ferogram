# Crate Architecture

`ferogram` is a workspace of focused, single-responsibility crates. Understanding the stack helps when you need to go below the high-level API.

## Dependency graph


```
Your App
 └ ferogram ← high-level Client, UpdateStream, InputMessage
 ├ ferogram-session ← session persistence, DC table, backends
 ├ ferogram-parsers ← Markdown and HTML entity parsing
 ├ ferogram-mtproto ← MTProto session, DH, message framing
 │ └ ferogram-crypto ← AES-IGE, RSA, SHA, factorize
 └ ferogram-tl-types ← all generated types + LAYER constant
 ├ ferogram-tl-gen (build-time code generator)
 └ ferogram-tl-parser (build-time TL schema parser)
```

---

## ferogram


**The high-level async Telegram client.** Import this in your application.

### What it provides
- `Client`: the main handle with all high-level methods
- `ClientBuilder`: fluent builder for connecting (`Client::builder()...connect()`)
- `Config`: connection configuration
- `Update` enum: typed update events
- `InputMessage`: fluent message builder
- `parsers::parse_markdown` / `parsers::parse_html`: text → entities
- `UpdateStream`: async iterator
- `Dialog`, `DialogIter`, `MessageIter`: dialog/history access
- `Participant`, `ParticipantStatus`: member info
- `Photo`, `Document`, `Sticker`, `Downloadable`: typed media wrappers
- `UploadedFile`, `DownloadIter`: upload/download
- `TypingGuard`: auto-cancels chat action on drop
- `SearchBuilder`, `GlobalSearchBuilder`: fluent search
- `InlineKeyboard`, `ReplyKeyboard`, `Button`: keyboard builders
- `SessionBackend` trait + `BinaryFileBackend`, `InMemoryBackend`, `StringSessionBackend`, `SqliteBackend`, `LibSqlBackend`
- `Socks5Config`: proxy configuration
- `TransportKind`: Abridged, Intermediate, Obfuscated
- Error types: `InvocationError`, `RpcError`, `SignInError`, `LoginToken`, `PasswordToken`
- Retry traits: `RetryPolicy`, `AutoSleep`, `NoRetries`, `RetryContext`

---

## ferogram-tl-types


**All generated Telegram API types.** Auto-regenerated at `cargo build` from `tl/api.tl`.

### What it provides
- `LAYER: i32`: the current layer number (224)
- `types::*`: 1,200+ concrete structs (`types::Message`, `types::User`, etc.)
- `enums::*`: 400+ boxed type enums (`enums::Message`, `enums::Peer`, etc.)
- `functions::*`: 500+ RPC function structs implementing `RemoteCall`
- `Serializable` / `Deserializable` traits
- `Cursor`: zero-copy deserializer
- `RemoteCall`: marker trait for RPC functions
- Optional: `name_for_id(u32) -> Option<&'static str>`

### Key type conventions

| Pattern | Meaning |
|---|---|
| `tl::types::Foo` | Concrete constructor: a struct |
| `tl::enums::Bar` | Boxed type: an enum wrapping one or more `types::*` |
| `tl::functions::ns::Method` | RPC function: implements `RemoteCall` |

Most Telegram API fields use `enums::*` types because the wire format is polymorphic.

---

## ferogram-mtproto


**The MTProto session layer.** Handles the low-level mechanics of talking to Telegram.

### What it provides
- `EncryptedSession`: manages auth key, salt, session ID, message IDs
- `authentication::*`: complete 3-step DH key exchange
- Message framing: serialization, padding, encryption, HMAC
- `msg_container` unpacking (batched responses)
- gzip decompression of `gzip_packed` responses
- Transport abstraction (abridged, intermediate, obfuscated)

### DH handshake steps


1. **PQ factorization**: `req_pq_multi` → server sends `resPQ`
2. **Server DH params**: `req_DH_params` with encrypted key → `server_DH_params_ok`
3. **Client DH finish**: `set_client_DH_params` → `dh_gen_ok`

After step 3, both sides hold the same auth key derived from the shared DH secret.

---

## ferogram-crypto


**Cryptographic primitives.** Pure Rust, `#![deny(unsafe_code)]`.

| Component | Algorithm | Usage |
|---|---|---|
| `aes` | AES-256-IGE | MTProto 2.0 message encryption/decryption |
| `auth_key` | SHA-256, XOR | Auth key derivation from DH material |
| `factorize` | Pollard's rho | PQ factorization in DH step 1 |
| RSA | PKCS#1 v1.5 | Encrypting PQ proof with Telegram's public keys |
| SHA-1 | SHA-1 | Used in auth key derivation |
| SHA-256 | SHA-256 | MTProto 2.0 MAC computation |
| `obfuscated` | AES-CTR | Transport-layer obfuscation init |
| PBKDF2 | PBKDF2-SHA512 | 2FA password derivation (via ferogram) |

---

## ferogram-tl-parser


**TL schema parser.** Converts `.tl` text into structured `Definition` values.

### Parsed AST types
- `Definition`: a single TL line (constructor or function)
- `Category`: `Type` or `Function`
- `Parameter`: a named field with type
- `ParameterType`: flags, conditionals, generic, basic
- `Flag`: `flags.N?type` conditional fields

Used exclusively by `build.rs` in `ferogram-tl-types`. You never import it directly.

---

## ferogram-tl-gen


**Rust code generator.** Takes the parsed AST and emits valid Rust source files.

### Output files (written to `$OUT_DIR`)
| File | Contents |
|---|---|
| `generated_common.rs` | `pub const LAYER: i32 = N;` + optional `name_for_id` |
| `generated_types.rs` | `pub mod types { … }`: all constructor structs |
| `generated_enums.rs` | `pub mod enums { … }`: all boxed type enums |
| `generated_functions.rs` | `pub mod functions { … }`: all RPC function structs |

Each type automatically gets:
- `impl Serializable`: binary TL encoding
- `impl Deserializable`: binary TL decoding
- `impl Identifiable`: `const CONSTRUCTOR_ID: u32`
- Optional: `impl Debug`, `impl From`, `impl TryFrom`, `impl Serialize/Deserialize`

---

## ferogram-session


**Session persistence types and pluggable storage backends.**

### What it provides
- `PersistedSession`: versioned binary format holding the full DC table, update state, and peer cache
- `SessionBackend` trait: implement to add custom storage (Redis, Postgres, etc.)
- `BinaryFileBackend`: stores session as a binary file on disk (default)
- `InMemoryBackend`: in-memory only, useful for testing or short-lived bots
- `StringSessionBackend`: base64 string, useful for environment-variable sessions
- `SqliteBackend` (feature: `sqlite-session`): SQLite-backed persistent sessions
- `LibSqlBackend` (feature: `libsql-session`): libSQL-backed persistent sessions
- `DcEntry` / `DcFlags`: per-DC auth key, salt, and capability flag storage
- `UpdatesStateSnap`: pts, qts, seq, date, and per-channel pts counters
- `CachedPeer` / `CachedMinPeer`: peer access-hash cache for users, channels, groups

### Feature flags

| Flag | What it enables |
|---|---|
| `sqlite-session` | `SqliteBackend` via rusqlite |
| `libsql-session` | `LibSqlBackend` via libsql |
| `serde` | `Serialize`/`Deserialize` on session types |

### Stack position

```
ferogram
└ ferogram-session
```

---

## ferogram-parsers


**Telegram HTML and Markdown entity parsers.**

### What it provides
- `parse_markdown(src)` → `(String, Vec<MessageEntity>)`: Telegram-flavoured Markdown to plain text + entity list
- `generate_markdown(text, entities)` → `String`: entity list back to Markdown
- `parse_html(src)` → `(String, Vec<MessageEntity>)`: Telegram HTML to plain text + entity list
- `generate_html(text, entities)` → `String`: entity list back to HTML

Used by `ferogram` for `InputMessage::markdown()` and `InputMessage::html()`, and available standalone for any crate that works with Telegram formatted text.

### Supported Markdown syntax

| Syntax | Entity |
|---|---|
| `**bold**` or `*bold*` | Bold |
| `__italic__` or `_italic_` | Italic |
| `~~strike~~` | Strikethrough |
| `\|\|spoiler\|\|` | Spoiler |
| `` `code` `` | Code |
| ` ```lang\npre\n``` ` | Pre (code block) |
| `[text](url)` | TextUrl |
| `[text](tg://user?id=123)` | MentionName |
| `![text](tg://emoji?id=123)` | CustomEmoji |

### Supported HTML tags

`<b>`, `<strong>`, `<i>`, `<em>`, `<u>`, `<s>`, `<del>`, `<code>`, `<pre>`, `<tg-spoiler>`, `<a href="url">`, `<tg-emoji emoji-id="id">`

### Feature flags

| Flag | What it enables |
|---|---|
| `html5ever` | Replaces `parse_html` with a spec-compliant html5ever tokenizer |

### Stack position

```
ferogram
└ ferogram-parsers
  └ ferogram-tl-types (tl-api feature)
```
