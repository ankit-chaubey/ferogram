# ferogram

High-level async Telegram client for Rust.

[![Crates.io](https://img.shields.io/crates/v/ferogram?color=fc8d62)](https://crates.io/crates/ferogram)
[![docs.rs](https://img.shields.io/badge/docs.rs-ferogram-5865F2)](https://docs.rs/ferogram)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](#license)
[![TL Layer](https://img.shields.io/badge/TL%20Layer-224-8b5cf6)](https://core.telegram.org/schema)

---

## Installation

```toml
[dependencies]
ferogram = "0.4.8"
tokio        = { version = "1", features = ["full"] }
```

Get your `api_id` and `api_hash` from [my.telegram.org](https://my.telegram.org).

### Feature Flags

```toml
ferogram = { version = "0.4.8", features = ["sqlite-session"] }  # SQLite session
ferogram = { version = "0.4.8", features = ["libsql-session"] }  # libsql / Turso
ferogram = { version = "0.4.8", features = ["html"] }            # HTML parser
ferogram = { version = "0.4.8", features = ["html5ever"] }       # html5ever parser
```

`StringSessionBackend`, `InMemoryBackend`, and `BinaryFileBackend` are always available.

---

## Connecting

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

### ClientBuilder methods

| Method | Description |
|---|---|
| `.api_id(i32)` | Telegram API ID (required) |
| `.api_hash(str)` | Telegram API hash (required) |
| `.session(path)` | Binary file session at `path` |
| `.session_string(s)` | Portable base64 string session |
| `.in_memory()` | Non-persistent in-memory session |
| `.session_backend(Arc<dyn SessionBackend>)` | Custom backend |
| `.catch_up(bool)` | Replay missed updates on connect (default: false) |
| `.dc_addr(str)` | Override first DC address |
| `.socks5(Socks5Config)` | Route connections through SOCKS5 proxy |
| `.allow_ipv6(bool)` | Allow IPv6 DC addresses (default: false) |
| `.transport(TransportKind)` | MTProto transport (default: Abridged) |
| `.retry_policy(Arc<dyn RetryPolicy>)` | Override flood-wait retry policy |
| `.connect()` | Build and connect, returns `(Client, ShutdownToken)` |

---

## Authentication

```rust
// Bot
if !client.is_authorized().await? {
    client.bot_sign_in("1234567890:ABCdef...").await?;
    client.save_session().await?;
}

// User
use ferogram::SignInError;

if !client.is_authorized().await? {
    let token = client.request_login_code("+1234567890").await?;
    let code = read_line();

    match client.sign_in(&token, &code).await {
        Ok(name) => println!("Welcome, {name}!"),
        Err(SignInError::PasswordRequired(t)) => {
            client.check_password(*t, "my_2fa_password").await?;
        }
        Err(e) => return Err(e.into()),
    }
    client.save_session().await?;
}

// Sign out
client.sign_out().await?;
```

### String Sessions

```rust
// Export
let session_string = client.export_session_string().await?;

// Restore
let (client, _shutdown) = Client::builder()
    .api_id(12345)
    .api_hash("your_api_hash")
    .session_string(session_string)
    .connect()
    .await?;
```

---

## Update Stream

```rust
let mut stream = client.stream_updates();
while let Some(update) = stream.next().await {
    match update {
        Update::NewMessage(msg)    => { /* new message */ }
        Update::MessageEdited(msg) => { /* edited message */ }
        Update::MessageDeleted(d)  => { /* deleted messages */ }
        Update::CallbackQuery(cb)  => { /* inline button pressed */ }
        Update::InlineQuery(iq)    => { /* @bot inline mode */ }
        Update::UserTyping(a)      => { /* typing / uploading */ }
        Update::UserStatus(s)      => { /* online/offline status */ }
        Update::Raw(raw)           => { /* unmapped TL update */ }
        _ => {}  // Update is #[non_exhaustive]
    }
}
```

`IncomingMessage` accessors: `id()`, `text()`, `peer_id()`, `sender_id()`, `outgoing()`, `date()`, `edit_date()`, `mentioned()`, `silent()`, `pinned()`, `post()`, `raw`.

---

## Messaging

```rust
client.send_message("@username", "Hello!").await?;
client.send_message("me", "Note to self").await?;
client.send_to_self("Reminder").await?;

if let Some(peer) = msg.peer_id() {
    client.send_message_to_peer(peer.clone(), "Reply!").await?;
}

client.edit_message(peer.clone(), msg_id, "Updated text").await?;
client.forward_messages(from_peer.clone(), to_peer.clone(), &[id1, id2]).await?;
client.delete_messages(peer.clone(), &[id1, id2]).await?;
client.pin_message(peer.clone(), msg_id, true).await?;
client.unpin_message(peer.clone(), msg_id).await?;
client.unpin_all_messages(peer.clone()).await?;
```

### InputMessage Builder

```rust
use ferogram::{InputMessage, parsers::parse_markdown};
use ferogram::keyboard::{Button, InlineKeyboard};

let (text, entities) = parse_markdown("**Bold** and `code`");

let kb = InlineKeyboard::new()
    .row([
        Button::callback("Yes", b"confirm:yes"),
        Button::callback("No",  b"confirm:no"),
    ]);

client
    .send_message_to_peer_ex(
        peer.clone(),
        &InputMessage::text(text)
            .entities(entities)
            .reply_to(Some(msg_id))
            .silent(true)
            .keyboard(kb),
    )
    .await?;
```

| Method | Description |
|---|---|
| `InputMessage::text(str)` | Create with text |
| `.reply_to(Option<i32>)` | Reply to message ID |
| `.silent(bool)` | No notification sound |
| `.no_webpage(bool)` | Suppress link preview |
| `.schedule_date(Option<i32>)` | Schedule for Unix timestamp |
| `.entities(Vec<MessageEntity>)` | Formatted text entities |
| `.keyboard(impl Into<ReplyMarkup>)` | Inline or reply keyboard |

---

## Keyboards

```rust
use ferogram::keyboard::{Button, InlineKeyboard, ReplyKeyboard};

// Inline
let kb = InlineKeyboard::new()
    .row([Button::callback("Like", b"like"), Button::url("Docs", "https://docs.rs/ferogram")]);

// Reply
let kb = ReplyKeyboard::new()
    .row([Button::text("Photo"), Button::text("Cancel")])
    .resize().single_use();

// Answer callback
Update::CallbackQuery(cb) => {
    client.answer_callback_query(cb.query_id, Some("Done!"), false).await?;
}
```

Button types: `callback`, `url`, `url_auth`, `switch_inline`, `switch_elsewhere`, `webview`, `simple_webview`, `request_phone`, `request_geo`, `request_poll`, `request_quiz`, `game`, `buy`, `copy_text`, `text`.

---

## Media

```rust
// Upload
let uploaded = client.upload_file("photo.jpg", &bytes).await?;
let uploaded = client.upload_file_concurrent("video.mp4", &bytes).await?;
let uploaded = client.upload_stream("doc.pdf", tokio_reader).await?;

client.send_file(peer.clone(), uploaded, false).await?;
client.send_album(peer.clone(), vec![a, b]).await?;

// Download
let bytes = client.download_media(&msg_media).await?;
client.download_media_to_file(&msg_media, "output.jpg").await?;
```

---

## Text Formatting

```rust
use ferogram::parsers::{parse_markdown, generate_markdown};

let (text, entities) = parse_markdown("**Bold**, `code`, _italic_");
client.send_message_to_peer_ex(peer.clone(), &InputMessage::text(text).entities(entities)).await?;
```

With `html` feature:

```rust
use ferogram::parsers::{parse_html, generate_html};
let (text, entities) = parse_html("<b>Bold</b> and <code>mono</code>");
```

---

## Reactions

```rust
use ferogram::reactions::InputReactions;

client.send_reaction(peer.clone(), msg_id, InputReactions::emoticon("👍")).await?;
client.send_reaction(peer.clone(), msg_id, InputReactions::custom_emoji(doc_id)).await?;
client.send_reaction(peer.clone(), msg_id, InputReactions::remove()).await?;
```

---

## Typing Guard

`TypingGuard` starts and stops typing indicators automatically on drop:

```rust
let _typing = client.typing(peer.clone()).await?;
let _typing = client.uploading_document(peer.clone()).await?;
let _typing = client.recording_video(peer.clone()).await?;

use ferogram::TypingGuard;
let _guard = TypingGuard::start(client, peer.clone(), action).await?;
```

---

## Participants

```rust
use ferogram::participants::{AdminRightsBuilder, BannedRightsBuilder};

let members = client.get_participants(peer.clone(), 100).await?;

client.kick_participant(peer.clone(), user_id).await?;
client.ban_participant(peer.clone(), user_id, BannedRightsBuilder::full_ban()).await?;
client.promote_participant(peer.clone(), user_id,
    AdminRightsBuilder::new().delete_messages(true).rank("Moderator")).await?;
```

---

## Search

```rust
use ferogram_tl_types::enums::MessagesFilter;

let results = client
    .search(peer.clone(), "query")
    .filter(MessagesFilter::InputMessagesFilterPhotos)
    .limit(50)
    .fetch(&client)
    .await?;

let global = client.search_global_builder("rust").broadcasts_only(true).fetch(&client).await?;
```

---

## Dialogs

```rust
let dialogs = client.get_dialogs(50).await?;
let mut iter = client.iter_dialogs();
while let Some(dialog) = iter.next(&client).await? { /* ... */ }

let mut iter = client.iter_messages(peer.clone());
while let Some(msg) = iter.next(&client).await? { /* ... */ }

client.mark_as_read(peer.clone()).await?;
client.delete_dialog(peer.clone()).await?;
```

---

## Session Backends

```rust
Client::builder().session("bot.session")         // binary file
Client::builder().in_memory()                     // tests
Client::builder().session_string(env_var)         // serverless
Client::builder().session_backend(Arc::new(SqliteBackend::new("sessions.db")))
Client::builder().session_backend(Arc::new(LibSqlBackend::remote(url, token)))
```

---

## Transport and Networking

```rust
use ferogram::{TransportKind, Socks5Config};

Client::builder().transport(TransportKind::Obfuscated)           // DPI bypass
Client::builder().socks5(Socks5Config::new("127.0.0.1:1080"))
Client::builder().socks5(Socks5Config::with_auth("host:1080", "user", "pass"))

client.invoke_on_dc(&req, 2).await?;
client.signal_network_restored();
```

---

## Error Handling

```rust
use ferogram::{InvocationError, RpcError};

match client.send_message("@peer", "Hello").await {
    Ok(()) => {}
    Err(InvocationError::Rpc(RpcError { code, message, .. })) => {
        eprintln!("Telegram error {code}: {message}");
    }
    Err(InvocationError::Io(e)) => eprintln!("I/O error: {e}"),
    Err(e) => eprintln!("Other: {e}"),
}
```

`FLOOD_WAIT` is handled automatically by `AutoSleep`. To disable:

```rust
use ferogram::retry::NoRetries;
Client::builder().retry_policy(Arc::new(NoRetries))
```

---

## Raw API

```rust
use ferogram::tl;

let req = tl::functions::bots::SetBotCommands { /* ... */ };
client.invoke(&req).await?;
client.invoke_on_dc(&req, 2).await?;
```

---

## Shutdown

```rust
let (client, shutdown) = Client::connect(config).await?;
shutdown.cancel();    // graceful
client.disconnect();  // immediate
```

---

## Client Methods Reference

All `async` methods return `Result<T, InvocationError>` unless noted.

### Connection & Session

| Method | Signature | Description |
|---|---|---|
| `Client::builder()` | `ClientBuilder` | Fluent builder |
| `Client::connect()` | `async (Config) → (Client, ShutdownToken)` | Low-level connect |
| `client.is_authorized()` | `async → bool` | Check if logged in |
| `client.save_session()` | `async → ()` | Persist session |
| `client.export_session_string()` | `async → String` | Export base64 session |
| `client.disconnect()` | sync | Immediate disconnect |
| `client.signal_network_restored()` | sync | Skip reconnect backoff |

### Authentication

| Method | Signature | Description |
|---|---|---|
| `client.bot_sign_in(token)` | `async → String` | Bot token login |
| `client.request_login_code(phone)` | `async → LoginToken` | Start user login |
| `client.sign_in(token, code)` | `async → String` | Complete user login |
| `client.check_password(token, pw)` | `async → String` | Submit 2FA password |
| `client.sign_out()` | `async → bool` | Log out |
| `client.get_me()` | `async → tl::types::User` | Get own user |

### Updates

| Method | Signature | Description |
|---|---|---|
| `client.stream_updates()` | `sync → UpdateStream` | Typed async update stream |
| `stream.next()` | `async → Option<Update>` | Next typed update |
| `stream.next_raw()` | `async → Option<RawUpdate>` | Next raw TL update |

### Messaging

| Method | Signature | Description |
|---|---|---|
| `client.send_message(peer, text)` | `async → ()` | Send text |
| `client.send_message_to_peer(peer, text)` | `async → ()` | Send text to peer |
| `client.send_message_to_peer_ex(peer, msg)` | `async → ()` | Send `InputMessage` |
| `client.send_to_self(text)` | `async → ()` | Send to Saved Messages |
| `client.edit_message(peer, id, text)` | `async → ()` | Edit a message |
| `client.forward_messages(from, to, ids)` | `async → ()` | Forward messages |
| `client.delete_messages(peer, ids)` | `async → ()` | Delete messages |
| `client.get_messages(peer, limit)` | `async → Vec<Message>` | Fetch latest messages |
| `client.get_messages_by_id(peer, ids)` | `async → Vec<Option<Message>>` | Fetch by ID |
| `client.pin_message(peer, id, notify)` | `async → ()` | Pin a message |
| `client.unpin_message(peer, id)` | `async → ()` | Unpin a message |
| `client.unpin_all_messages(peer)` | `async → ()` | Unpin all |
| `client.get_pinned_message(peer)` | `async → Option<Message>` | Get pinned |
| `client.get_scheduled_messages(peer)` | `async → Vec<Message>` | List scheduled |
| `client.delete_scheduled_messages(peer, ids)` | `async → ()` | Cancel scheduled |

### Media

| Method | Signature | Description |
|---|---|---|
| `client.upload_file(name, bytes)` | `async → UploadedFile` | Sequential upload |
| `client.upload_file_concurrent(name, bytes)` | `async → UploadedFile` | Parallel upload |
| `client.upload_stream(name, reader)` | `async → UploadedFile` | Upload from `AsyncRead` |
| `client.send_file(peer, file, as_photo)` | `async → ()` | Send uploaded file |
| `client.send_album(peer, files)` | `async → ()` | Send as album |
| `client.download_media(loc)` | `async → Vec<u8>` | Download to memory |
| `client.download_media_to_file(loc, path)` | `async → ()` | Stream to file |
| `client.download_media_concurrent(loc)` | `async → Vec<u8>` | Parallel download |
| `client.download(item)` | `async → Vec<u8>` | Download `Downloadable` |

### Chat Actions

| Method | Signature | Description |
|---|---|---|
| `client.typing(peer)` | `async → TypingGuard` | RAII typing |
| `client.uploading_document(peer)` | `async → TypingGuard` | RAII upload indicator |
| `client.recording_video(peer)` | `async → TypingGuard` | RAII video indicator |
| `client.mark_as_read(peer)` | `async → ()` | Mark as read |
| `client.clear_mentions(peer)` | `async → ()` | Clear mention badges |
| `client.send_reaction(peer, msg_id, r)` | `async → ()` | Send reaction |

### Dialogs & Peers

| Method | Signature | Description |
|---|---|---|
| `client.get_dialogs(limit)` | `async → Vec<Dialog>` | Fetch dialogs |
| `client.iter_dialogs()` | `sync → DialogIter` | Lazy dialog iterator |
| `client.iter_messages(peer)` | `sync → MessageIter` | Lazy message iterator |
| `client.delete_dialog(peer)` | `async → ()` | Leave and delete |
| `client.join_chat(peer)` | `async → ()` | Join public chat |
| `client.accept_invite_link(link)` | `async → ()` | Accept invite |
| `client.resolve_peer(str)` | `async → Peer` | Resolve username/phone |
| `client.resolve_username(str)` | `async → Peer` | Resolve bare username |

### Search

| Method | Signature | Description |
|---|---|---|
| `client.search(peer, query)` | `sync → SearchBuilder` | In-chat search |
| `client.search_global_builder(query)` | `sync → GlobalSearchBuilder` | Global search |
| `client.search_messages(peer, q, limit)` | `async → Vec<IncomingMessage>` | Quick in-chat search |
| `client.search_global(q, limit)` | `async → Vec<IncomingMessage>` | Quick global search |

### Participants

| Method | Signature | Description |
|---|---|---|
| `client.get_participants(peer, limit)` | `async → Vec<Participant>` | Fetch members |
| `client.iter_participants(peer)` | `sync → impl Iterator` | Lazy iterator |
| `client.kick_participant(peer, user_id)` | `async → ()` | Kick |
| `client.ban_participant(peer, user_id, rights)` | `async → ()` | Ban/restrict |
| `client.promote_participant(peer, user_id, rights)` | `async → ()` | Promote admin |
| `client.get_permissions(peer, user_id)` | `async → ParticipantPermissions` | Get permissions |
| `client.get_profile_photos(user_id, offset, limit)` | `async → Vec<Photo>` | Fetch photos |

### Raw & Advanced

| Method | Signature | Description |
|---|---|---|
| `client.invoke(req)` | `async → R::Return` | Call any Layer 224 method |
| `client.invoke_on_dc(req, dc_id)` | `async → R::Return` | Call on specific DC |

---

## License

MIT or Apache-2.0, at your option. See [LICENSE-MIT](../LICENSE-MIT) and [LICENSE-APACHE](../LICENSE-APACHE).

**Ankit Chaubey** - [github.com/ankit-chaubey](https://github.com/ankit-chaubey) · [docs.rs/ferogram](https://docs.rs/ferogram)
