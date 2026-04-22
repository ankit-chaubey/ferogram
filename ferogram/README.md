<div align="center">

# ferogram

Async Rust client for the Telegram MTProto API.

[![Crates.io](https://img.shields.io/crates/v/ferogram?color=fc8d62)](https://crates.io/crates/ferogram)
[![docs.rs](https://img.shields.io/badge/docs.rs-ferogram-5865F2)](https://docs.rs/ferogram)
[![License](https://img.shields.io/badge/license-MIT%20%7C%20Apache--2.0-blue)](LICENSE-MIT)
[![TL Layer](https://img.shields.io/badge/TL%20Layer-224-8b5cf6)](https://core.telegram.org/schema)
[![Telegram Channel](https://img.shields.io/badge/channel-%40Ferogram-2CA5E0?logo=telegram)](https://t.me/Ferogram)
[![Telegram Chat](https://img.shields.io/badge/chat-%40FerogramChat-2CA5E0?logo=telegram)](https://t.me/FerogramChat)

Built by **[Ankit Chaubey](https://github.com/ankit-chaubey)**

</div>

> **Pre-production.** APIs may change between minor versions. Check [CHANGELOG](../CHANGELOG.md) before upgrading.

---

## What it is

`ferogram` is the high-level client crate in the ferogram workspace. It talks to Telegram directly over MTProto, no Bot API HTTP proxy. Works for both user accounts and bots.

For the full workspace (crypto, session, TL types, etc.) see the [repository root](https://github.com/ankit-chaubey/ferogram).

---

## Installation

```toml
[dependencies]
ferogram = "0.3"
tokio    = { version = "1", features = ["full"] }
```

Get `api_id` and `api_hash` from [my.telegram.org](https://my.telegram.org).

Optional feature flags:

```toml
ferogram = { version = "0.3", features = [
    "sqlite-session",  # SqliteBackend via rusqlite
    "libsql-session",  # LibSqlBackend via libsql-client (Turso)
    "html",            # parse_html / generate_html (built-in parser)
    "html5ever",       # parse_html via spec-compliant html5ever
    "derive",          # #[derive(FsmState)]
    "serde",           # serde support for session types
] }
```

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

### `ClientBuilder` methods

| Method | Description |
|---|---|
| `.api_id(i32)` | Telegram API ID (required) |
| `.api_hash(str)` | Telegram API hash (required) |
| `.session(path)` | Binary file session at `path` |
| `.session_string(s)` | Portable base64 string session |
| `.in_memory()` | Non-persistent in-memory session |
| `.session_backend(Arc<dyn SessionBackend>)` | Custom backend |
| `.catch_up(bool)` | Replay missed updates on reconnect (default: false) |
| `.dc_addr(str)` | Override first DC address |
| `.socks5(Socks5Config)` | Route through SOCKS5 proxy |
| `.proxy_link(str)` | MTProxy `t.me/proxy?...` link |
| `.allow_ipv6(bool)` | Allow IPv6 DC addresses (default: false) |
| `.transport(TransportKind)` | MTProto transport (Abridged, Obfuscated, FakeTls) |
| `.probe_transport(bool)` | Race transports, use the first to connect (default: false) |
| `.resilient_connect(bool)` | Fall back through DoH + Telegram special-config on TCP failure |
| `.retry_policy(Arc<dyn RetryPolicy>)` | Override flood-wait retry policy |
| `.restart_policy(Arc<dyn ConnectionRestartPolicy>)` | Override reconnect policy |
| `.device_model(str)` | Device model string sent in InitConnection |
| `.system_version(str)` | System version string |
| `.app_version(str)` | App version string |
| `.experimental_features(ExperimentalFeatures)` | Opt-in to non-default behaviors |
| `.connect()` | Build and connect, returns `(Client, ShutdownToken)` |

---

## Authentication

```rust
// Bot login
if !client.is_authorized().await? {
    client.bot_sign_in("1234567890:ABCdef...").await?;
    client.save_session().await?;
}

// User login
use ferogram::SignInError;

if !client.is_authorized().await? {
    let token = client.request_login_code("+1234567890").await?;
    let code  = read_line(); // read from stdin

    match client.sign_in(&token, &code).await {
        Ok(name) => println!("Signed in as {name}"),
        Err(SignInError::PasswordRequired(t)) => {
            client.check_password(*t, "my_2fa_password").await?;
        }
        Err(e) => return Err(e.into()),
    }
    client.save_session().await?;
}

// QR code login (for clients where keyboard input is not available)
let (token_bytes, expires_in) = client.export_login_token().await?;
// Display as QR code, then poll:
if let Some(name) = client.check_qr_login(token_bytes).await? {
    println!("Signed in as {name}");
}

// Sign out
client.sign_out().await?;
```

### Session export / restore

```rust
let s = client.export_session_string().await?;

let (client, _) = Client::builder()
    .api_id(12345)
    .api_hash("your_api_hash")
    .session_string(s)
    .connect()
    .await?;
```

---

## Update stream

```rust
use ferogram::update::Update;

let mut stream = client.stream_updates();
while let Some(upd) = stream.next().await {
    match upd {
        Update::NewMessage(msg)            => { /* new or forwarded message */ }
        Update::MessageEdited(msg)         => { /* message was edited */ }
        Update::MessageDeleted(d)          => { /* one or more messages deleted */ }
        Update::CallbackQuery(cb)          => { /* inline button pressed */ }
        Update::InlineQuery(iq)            => { /* @bot inline mode query */ }
        Update::InlineSend(s)              => { /* user chose an inline result */ }
        Update::UserStatus(s)              => { /* user online/offline change */ }
        Update::UserTyping(a)              => { /* typing / uploading / recording */ }
        Update::ParticipantUpdate(p)       => { /* member joined/left/promoted/banned */ }
        Update::JoinRequest(j)             => { /* user requested to join via link */ }
        Update::MessageReaction(r)         => { /* reaction added or removed */ }
        Update::PollVote(v)                => { /* user voted in a poll */ }
        Update::BotStopped(b)              => { /* user blocked or restarted bot */ }
        Update::ShippingQuery(s)           => { /* shipping address submitted */ }
        Update::PreCheckoutQuery(p)        => { /* payment confirmation screen */ }
        Update::ChatBoost(b)               => { /* channel boosted via bot */ }
        Update::Raw(raw)                   => { /* unmapped TL update */ }
        _ => {}
    }
}
```

`IncomingMessage` accessors: `id()`, `text()`, `text_html()`, `text_markdown()`, `peer_id()`, `sender_id()`, `sender()`, `outgoing()`, `date()`, `edit_date()`, `mentioned()`, `silent()`, `pinned()`, `post()`, `has_media()`, `is_photo()`, `is_document()`, `download_media_with()`, `command()`, `reply()`, `raw`.

---

## Dispatcher and filters

```rust
use ferogram::filters::{Dispatcher, command, private, group, text, text_contains, regex, media};

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

Built-in filter functions: `all`, `none`, `private`, `group`, `channel`, `text`, `text_contains`, `command`, `media`, `photo`, `document`, `audio`, `video`, `sticker`, `regex`, `custom`, `from_user`, `outgoing`, `incoming`.

Filters compose with `&` (both), `|` (either), `!` (not).

---

## Middleware

```rust
use ferogram::filters::Dispatcher;

let mut dp = Dispatcher::new();

dp.middleware(|upd, next| async move {
    tracing::info!("incoming update");
    let result = next.run(upd).await;
    tracing::info!("handler done");
    result
});
```

Middleware runs in registration order before any handler. Calling `next.run(upd)` passes control down the chain. Returning early skips all remaining middleware and the handler.

---

## FSM

```rust
use ferogram::{FsmState, fsm::MemoryStorage};
use ferogram::filters::{Dispatcher, command, text};
use std::sync::Arc;

#[derive(FsmState, Clone, Debug, PartialEq)]
enum OrderState {
    WaitingProduct,
    WaitingQuantity,
}

let mut dp = Dispatcher::new();
dp.with_state_storage(Arc::new(MemoryStorage::new()));

dp.on_message(command("order"), |msg| async move {
    msg.reply("Which product?").await.ok();
});

dp.on_message_fsm(text(), OrderState::WaitingProduct, |msg, state| async move {
    state.set_data("product", msg.text().unwrap()).await.ok();
    state.transition(OrderState::WaitingQuantity).await.ok();
    msg.reply("How many?").await.ok();
});

dp.on_message_fsm(text(), OrderState::WaitingQuantity, |msg, state| async move {
    let product = state.get_data::<String>("product").await.unwrap_or_default();
    let qty = msg.text().unwrap_or("1");
    msg.reply(&format!("Order: {qty}x {product}")).await.ok();
    state.finish().await.ok();
});
```

State storage: `MemoryStorage` is built in. Implement `StateStorage` for Redis, SQL, or any other backend. State keys can be scoped per-user, per-chat, or per-user-in-chat via `StateKeyStrategy`.

---

## Messaging

```rust
client.send_message("@username", "Hello!").await?;
client.send_message("me", "Saved note").await?;
client.send_to_self("Reminder").await?;

client.send_message_to_peer(peer.clone(), "Text").await?;

client.edit_message(peer.clone(), msg_id, "Updated text").await?;
client.forward_messages(from_peer.clone(), to_peer.clone(), &[id1, id2]).await?;
client.delete_messages(peer.clone(), &[id1, id2]).await?;
client.pin_message(peer.clone(), msg_id, true).await?;
client.unpin_message(peer.clone(), msg_id).await?;
client.unpin_all_messages(peer.clone()).await?;

// Get reply-to message
let replied = client.get_reply_to_message(&msg).await?;
```

### `InputMessage` builder

```rust
use ferogram::InputMessage;
use ferogram::keyboard::{Button, InlineKeyboard};

let kb = InlineKeyboard::new()
    .row([
        Button::callback("Yes", b"confirm:yes"),
        Button::callback("No",  b"confirm:no"),
    ])
    .row([Button::url("Docs", "https://docs.rs/ferogram")]);

client.send_message_to_peer_ex(
    peer.clone(),
    &InputMessage::html("<b>Bold</b> and <code>mono</code>")
        .reply_to(Some(msg_id))
        .silent(true)
        .no_webpage(true)
        .keyboard(kb),
).await?;
```

| Method | Description |
|---|---|
| `InputMessage::text(str)` | Plain text |
| `InputMessage::markdown(str)` | Parse Telegram Markdown |
| `InputMessage::html(str)` | Parse HTML |
| `.reply_to(Option<i32>)` | Reply to message ID |
| `.silent(bool)` | No notification sound |
| `.background(bool)` | Background message |
| `.no_webpage(bool)` | Suppress link preview |
| `.invert_media(bool)` | Show media above caption |
| `.schedule_date(Option<i32>)` | Schedule for Unix timestamp |
| `.schedule_once_online()` | Send when peer comes online |
| `.entities(Vec<MessageEntity>)` | Pre-computed entities |
| `.keyboard(impl Into<ReplyMarkup>)` | Inline or reply keyboard |
| `.copy_media(InputMedia)` | Attach media |
| `.clear_media()` | Remove attached media |

### Scheduled messages

```rust
let scheduled = client.get_scheduled_messages(peer.clone()).await?;
client.delete_scheduled_messages(peer.clone(), &[id1]).await?;
client.send_scheduled_now(peer.clone(), msg_id).await?;
```

---

## Keyboards

```rust
use ferogram::keyboard::{Button, InlineKeyboard, ReplyKeyboard};

// Inline keyboard
let kb = InlineKeyboard::new()
    .row([
        Button::callback("Click me", b"data"),
        Button::url("Open", "https://example.com"),
        Button::switch_inline("Search", "query"),
        Button::webview("App", "https://myapp.example.com"),
    ]);

// Reply keyboard
let kb = ReplyKeyboard::new()
    .row([Button::text("Option A"), Button::text("Option B")])
    .resize()
    .single_use()
    .placeholder("Choose...");

// Answer callback query
Update::CallbackQuery(cb) => {
    cb.answer().text("Done!").show_alert(false).send(&client).await?;
    // or shorthand:
    client.answer_callback_query(cb.query_id, Some("Done!"), false).await?;
}

// Answer inline query
Update::InlineQuery(iq) => {
    client.answer_inline_query(iq.query_id, results, cache_time, is_personal, next_offset).await?;
}
```

Full button types: `callback`, `url`, `url_auth`, `switch_inline`, `switch_inline_chosen_peer`, `switch_inline_current_chat`, `webview`, `simple_webview`, `request_phone`, `request_geo`, `request_poll`, `request_quiz`, `game`, `buy`, `copy_text`, `text`.

---

## Media

```rust
use std::sync::Arc;

// Upload from bytes
let file = client.upload_file("photo.jpg", &bytes, "image/jpeg").await?;
let file = client.upload_file_concurrent(Arc::new(bytes), "video.mp4", "video/mp4").await?;

// Upload from AsyncRead stream
let file = client.upload_stream("doc.pdf", reader, file_size).await?;

// Upload arbitrary InputMedia (stickers, thumbnails, etc.)
let media = client.upload_media(peer.clone(), input_media).await?;

// Send
client.send_file(peer.clone(), &file, false).await?;          // false = as document
client.send_file(peer.clone(), &file, true).await?;           // true = as photo
client.send_album(peer.clone(), vec![file_a, file_b]).await?; // grouped

// Download to memory
let bytes = client.download_media(&msg.raw.media).await?;
let bytes = client.download_media_concurrent(&msg.raw.media).await?;

// Download to file
client.download_media_to_file(&msg.raw.media, "output.jpg").await?;
client.download_media_to_file_on_dc(&msg.raw.media, "output.jpg", dc_id).await?;

// Implement Downloadable on your own types for download()
let bytes = client.download(&photo).await?;

// CDN downloads (large media)
let iter = client.iter_download(file_location);
```

---

## Text formatting

```rust
// Telegram-flavoured Markdown
use ferogram::parsers::{parse_markdown, generate_markdown};

let (text, entities) = parse_markdown("**Bold**, `code`, _italic_, ||spoiler||");

// HTML (requires "html" feature)
use ferogram::parsers::{parse_html, generate_html};

let (text, entities) = parse_html("<b>Bold</b>, <code>mono</code>, <tg-spoiler>hidden</tg-spoiler>");

// InputMessage has shorthand for both:
InputMessage::markdown("**bold**")
InputMessage::html("<b>bold</b>")
```

---

## Reactions

```rust
use ferogram::reactions::InputReactions;

client.send_reaction(peer.clone(), msg_id, InputReactions::emoticon("👍")).await?;
client.send_reaction(peer.clone(), msg_id, InputReactions::custom_emoji(doc_id)).await?;
client.send_reaction(peer.clone(), msg_id, InputReactions::remove()).await?;
client.send_paid_reaction(peer.clone(), msg_id, count).await?;
client.read_reactions(peer.clone()).await?;
client.clear_recent_reactions().await?;

let reactions = client.get_message_reactions(peer.clone(), msg_id).await?;
let list = client.get_reaction_list(peer.clone(), msg_id, reaction, offset, limit).await?;
```

---

## Typing indicators

`TypingGuard` sends the action on construction and cancels it on drop:

```rust
let _typing   = client.typing(peer.clone()).await?;
let _uploading = client.uploading_document(peer.clone()).await?;
let _recording = client.recording_video(peer.clone()).await?;

use ferogram::TypingGuard;
let _guard = TypingGuard::start(&client, peer.clone(), action).await?;
```

---

## Dialogs

```rust
let dialogs = client.get_dialogs(50).await?;

// Lazy paginated iterator
let mut iter = client.iter_dialogs();
while let Some(dialog) = iter.next(&client).await? {
    println!("{}", dialog.title());
}

// Pinned, archive
let pinned = client.get_pinned_dialogs(folder_id).await?;
client.pin_dialog(peer.clone()).await?;
client.unpin_dialog(peer.clone()).await?;
client.archive_chat(peer.clone()).await?;
client.unarchive_chat(peer.clone()).await?;
client.mark_dialog_unread(peer.clone(), true).await?;

// Drafts
client.save_draft(peer.clone(), "draft text", reply_to).await?;
client.get_all_drafts().await?;
client.clear_all_drafts().await?;
```

---

## Message history

```rust
let msgs = client.get_messages(peer.clone(), 20).await?;
let msgs = client.get_messages_by_id(peer.clone(), &[id1, id2]).await?;
let pinned = client.get_pinned_message(peer.clone()).await?;

// Lazy paginated iterator (reverse chronological)
let mut iter = client.iter_messages(peer.clone());
while let Some(msg) = iter.next(&client).await? { /* ... */ }

client.mark_as_read(peer.clone()).await?;
client.clear_mentions(peer.clone()).await?;
client.delete_dialog(peer.clone()).await?;

// Get media group (album)
let group = client.get_media_group(peer.clone(), msg_id).await?;

// Message read participants (premium feature)
let readers = client.get_message_read_participants(peer.clone(), msg_id).await?;

// Export message link
let link = client.export_message_link(peer.clone(), msg_id, grouped).await?;
```

---

## Search

```rust
use ferogram_tl_types::enums::MessagesFilter;

// In-chat search with builder
let results = client
    .search(peer.clone(), "query")
    .filter(MessagesFilter::InputMessagesFilterPhotos)
    .from_user(user_peer)
    .limit(50)
    .fetch(&client)
    .await?;

// Global search with builder
let results = client
    .search_global_builder("rust telegram")
    .broadcasts_only(true)
    .limit(20)
    .fetch(&client)
    .await?;

// Quick helpers
let results = client.search_messages(peer.clone(), "query", 20).await?;
let results = client.search_global("query", 20).await?;
```

---

## Participants

```rust
use ferogram::participants::{AdminRightsBuilder, BannedRightsBuilder};

let members = client.get_participants(peer.clone(), 100).await?;

client.kick_participant(peer.clone(), user_peer.clone()).await?;

client.ban_participant(
    peer.clone(),
    user_peer.clone(),
    BannedRightsBuilder::full_ban(),
).await?;

client.ban_participant(
    peer.clone(),
    user_peer.clone(),
    BannedRightsBuilder::new()
        .send_messages(false)
        .send_media(false)
        .until_date(unix_ts),
).await?;

client.promote_participant(
    peer.clone(),
    user_peer.clone(),
    AdminRightsBuilder::new()
        .delete_messages(true)
        .ban_users(true)
        .pin_messages(true)
        .rank("Moderator"),
).await?;

let photos = client.get_profile_photos(user_id, 0, 10).await?;
```

---

## Chat management

```rust
// Create
let chat = client.create_group("Group Name", &[user_id1, user_id2]).await?;
let channel = client.create_channel("Channel Name", "About this channel", false).await?;

// Edit
client.edit_chat_title(peer.clone(), "New Title").await?;
client.edit_chat_about(peer.clone(), "New description").await?;
client.edit_chat_photo(peer.clone(), uploaded_file).await?;
client.edit_chat_default_banned_rights(peer.clone(), rights).await?;

// Membership
client.invite_users(peer.clone(), &[user_peer1, user_peer2]).await?;
client.leave_chat(peer.clone()).await?;
client.delete_channel(peer.clone()).await?;
client.delete_chat(chat_id).await?;
client.migrate_chat(chat_id).await?; // basic group -> supergroup

// Info
let full = client.get_chat_full(peer.clone()).await?;
let admins = client.get_chat_administrators(peer.clone()).await?;
let online = client.get_online_count(peer.clone()).await?;
let common = client.get_common_chats(user_peer.clone()).await?;
let count = client.count_channels().await?;

// Invite links
let link = client.export_invite_link(peer.clone()).await?;
let links = client.get_invite_links(peer.clone(), revoked, admin_id).await?;
client.revoke_invite_link(peer.clone(), link).await?;
client.edit_invite_link(peer.clone(), link, expire_date, usage_limit, request_needed).await?;
client.delete_invite_link(peer.clone(), link).await?;
client.delete_revoked_invite_links(peer.clone(), admin_id).await?;
let members = client.get_invite_link_members(peer.clone(), invite, offset, limit).await?;
let admins_invites = client.get_admins_with_invites(peer.clone()).await?;

// Misc
client.toggle_no_forwards(peer.clone(), true).await?;
client.set_chat_theme(peer.clone(), "🌊").await?;
client.set_chat_reactions(peer.clone(), reactions).await?;
let send_as = client.get_send_as_peers(peer.clone()).await?;
client.set_default_send_as(peer.clone(), send_as_peer).await?;
client.transfer_chat_ownership(peer.clone(), user_peer.clone(), password).await?;
let linked = client.get_linked_channel(peer.clone()).await?;

// Forum topics (supergroups with topics enabled)
let topics = client.get_forum_topics(peer.clone(), limit).await?;
client.create_forum_topic(peer.clone(), "Topic Name", icon_color, icon_emoji).await?;
client.edit_forum_topic(peer.clone(), topic_id, title, icon_emoji).await?;
client.delete_forum_topic_history(peer.clone(), topic_id).await?;
client.toggle_forum(peer.clone(), true).await?;
```

---

## Contacts

```rust
let contacts = client.get_contacts().await?;
client.add_contact(user_peer.clone(), "First", "Last", phone, share_phone).await?;
client.delete_contacts(&[user_id1, user_id2]).await?;
client.import_contacts(contacts_vec).await?;
let results = client.search_contacts("query", limit).await?;
client.block_user(user_peer.clone()).await?;
client.unblock_user(user_peer.clone()).await?;
```

---

## Profile

```rust
client.update_profile(first_name, last_name, about).await?;
client.update_username(username).await?;
client.update_status(false).await?; // false = online, true = offline
client.set_profile_photo(uploaded_file).await?;
client.delete_profile_photos(&[photo_id]).await?;
client.set_emoji_status(emoji_doc_id, expires).await?;
let full = client.get_user_full(user_peer.clone()).await?;

// Active sessions
let sessions = client.get_authorizations().await?;
client.terminate_session(session_hash).await?;
```

---

## Polls

```rust
client.send_poll(
    peer.clone(),
    "Question?",
    &["Option A", "Option B", "Option C"],
    is_anonymous,
    is_quiz,
    correct_answer_idx,
    solution,
).await?;

let results = client.get_poll_results(peer.clone(), msg_id).await?;
let votes = client.get_poll_votes(peer.clone(), msg_id, option, offset, limit).await?;
client.send_vote(peer.clone(), msg_id, &[chosen_option]).await?;
```

---

## Bot-specific

```rust
// Set commands
client.set_bot_commands(scope, lang_code, commands).await?;
client.delete_bot_commands(scope, lang_code).await?;

// Bot info
client.set_bot_info(bot_peer, lang_code, name, about, description).await?;
let info = client.get_bot_info(bot_peer, lang_code).await?;

// Inline message editing (from inline send)
client.edit_inline_message(inline_msg_id, text, entities, reply_markup).await?;

// Games
client.start_bot(bot_peer.clone(), peer.clone(), start_param).await?;
client.set_game_score(peer.clone(), msg_id, user_peer.clone(), score, force, no_edit).await?;
let scores = client.get_game_high_scores(peer.clone(), msg_id, user_peer.clone()).await?;

// Payments
client.send_invoice(peer.clone(), invoice_params).await?;
client.answer_shipping_query(query_id, ok, shipping_options, error).await?;
client.answer_precheckout_query(query_id, ok, error).await?;

// Reply to join request
// Update::JoinRequest(j) => approve or decline via j.approve(&client)
```

---

## Discussions and replies

```rust
let thread = client.get_replies(peer.clone(), msg_id, offset_id, limit).await?;
let discussion = client.get_discussion_message(channel_peer.clone(), msg_id).await?;
client.read_discussion(channel_peer.clone(), msg_id, read_max_id).await?;
```

---

## Translation and transcription

```rust
let translated = client.translate_messages(peer.clone(), &[msg_id], "en").await?;
let transcript = client.transcribe_audio(peer.clone(), msg_id).await?;
client.toggle_peer_translations(peer.clone(), disabled).await?;
```

---

## Stickers

```rust
let set = client.get_sticker_set("StickerSetShortName").await?;
client.install_sticker_set("StickerSetShortName", false).await?;
client.uninstall_sticker_set("StickerSetShortName").await?;
let all = client.get_all_stickers(hash).await?;
let docs = client.get_custom_emoji_documents(&[doc_id]).await?;
```

---

## Privacy and notifications

```rust
use ferogram_tl_types::enums::InputPrivacyKey;

let rules = client.get_privacy(InputPrivacyKey::StatusTimestamp).await?;
client.set_privacy(InputPrivacyKey::StatusTimestamp, rules).await?;

let settings = client.get_notify_settings(peer.clone()).await?;
client.update_notify_settings(peer.clone(), settings).await?;
```

---

## Admin log

```rust
let events = client.get_admin_log(
    peer.clone(),
    query,
    events_filter,
    admins,
    max_id,
    min_id,
    limit,
).await?;
```

---

## Channel and megagroup stats

```rust
let broadcast_stats = client.get_broadcast_stats(peer.clone(), dark).await?;
let megagroup_stats = client.get_megagroup_stats(peer.clone(), dark).await?;
```

---

## Peer resolution

```rust
// From username, phone, or t.me link
let peer = client.resolve_peer("@username").await?;
let peer = client.resolve_peer("+12345678901").await?;
let peer = client.resolve_username("username").await?;
let input_peer = client.resolve_to_input_peer(peer_ref).await?;
let hash = Client::parse_invite_hash("https://t.me/+abc123")?;
```

---

## Transport and proxy

```rust
use ferogram::{TransportKind, Socks5Config};

// Choose transport
Client::builder().transport(TransportKind::Abridged)    // default
Client::builder().transport(TransportKind::Obfuscated)  // DPI bypass
Client::builder().transport(TransportKind::FakeTls)     // TLS camouflage

// MTProxy (parses t.me/proxy or t.me/+proxy links)
Client::builder().proxy_link("https://t.me/proxy?server=HOST&port=PORT&secret=SECRET")

// SOCKS5
Client::builder().socks5(Socks5Config::new("127.0.0.1:1080"))
Client::builder().socks5(Socks5Config::with_auth("host:1080", "user", "pass"))

// Race transports, use first to connect
Client::builder().probe_transport(true)

// Fall back through DNS-over-HTTPS and Telegram special-config if TCP fails
Client::builder().resilient_connect(true)
```

---

## Session backends

```rust
use ferogram_session::{SqliteBackend, LibSqlBackend};
use std::sync::Arc;

Client::builder().session("bot.session")                                    // binary file
Client::builder().in_memory()                                               // no persistence
Client::builder().session_string(env::var("SESSION")?)                      // base64 string
Client::builder().session_backend(Arc::new(SqliteBackend::open("s.db")?))  // sqlite
Client::builder().session_backend(Arc::new(LibSqlBackend::remote(url, token).await?))  // turso
```

Custom: implement `SessionBackend` from `ferogram_session`.

---

## Error handling

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

`FLOOD_WAIT` is handled automatically. To disable:

```rust
use ferogram::retry::NoRetries;
Client::builder().retry_policy(Arc::new(NoRetries))
```

---

## Raw API

Every Layer 224 TL method is accessible directly:

```rust
use ferogram::tl;

let req = tl::functions::bots::SetBotCommands {
    scope: tl::enums::BotCommandScope::Default(tl::types::BotCommandScopeDefault {}),
    lang_code: "en".into(),
    commands: vec![tl::enums::BotCommand::BotCommand(tl::types::BotCommand {
        command: "start".into(),
        description: "Start the bot".into(),
    })],
};
client.invoke(&req).await?;
client.invoke_on_dc(2, &req).await?;
```

---

## Shutdown

```rust
let (client, shutdown) = Client::builder()...connect().await?;

shutdown.cancel();   // graceful shutdown
client.disconnect(); // immediate disconnect
```

---

## Full Feature List

For everything ferogram supports (auth, media, FSM, polls, admin rights, privacy, games, raw API, and more) see **[FEATURES.md](../FEATURES.md)**.

---

## Community

- **Channel** (announcements, releases): [t.me/Ferogram](https://t.me/Ferogram)
- **Chat** (questions, support): [t.me/FerogramChat](https://t.me/FerogramChat)
- **API docs**: [docs.rs/ferogram](https://docs.rs/ferogram)
- **Guide**: [ferogram.ankitchaubey.in](https://ferogram.ankitchaubey.in/)
- **GitHub**: [github.com/ankit-chaubey/ferogram](https://github.com/ankit-chaubey/ferogram)

---

## Author

Developed by [Ankit Chaubey](https://github.com/ankit-chaubey).

---

## License

MIT OR Apache-2.0. See [LICENSE-MIT](../LICENSE-MIT) and [LICENSE-APACHE](../LICENSE-APACHE).

Usage must comply with [Telegram's API Terms of Service](https://core.telegram.org/api/terms).
