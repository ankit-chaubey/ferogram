# Release History

ferogram is an asynchronous Rust framework for Telegram's MTProto protocol, built to make working with Telegram easier for both bot and user accounts.

Here you'll find an overview of what's new in each release, from new features and improvements to bug fixes and breaking changes. For a more detailed breakdown, check out the "CHANGELOG" (https://github.com/ankit-chaubey/ferogram/blob/main/CHANGELOG.md).

---

## v0.6.7

Released 2026-11-10. TL layer 233, Telegram secret chats, and a round of fixes for downloads, QR login and updates.

All crates except `ferogram-e2e` are released as 0.6.7. `ferogram-e2e` is versioned separately and ships as 0.1.0.

### Secret chats

The new `e2e` feature adds end-to-end encrypted chats as `ferogram::e2e`, backed by the new `ferogram-e2e` crate. It covers key exchange, message and file encryption, encrypted media upload and download, re-keying, self-destruct timers and service messages. Chats are stored in `<session_name>.e2e` next to the session file, with atomic, checksummed writes and mode `0600` on Unix. The crate does not depend on `ferogram`, you can drive it from any client through the `E2eTransport` trait.

```rust
use ferogram::e2e::{E2eClient, E2eClientExt, E2eEvent};

let e2e = E2eClient::for_session(client.clone(), "mybot.session")?;
let chat_id = e2e.request_secret_chat("@someone").await?;

let mut stream = client.stream_updates();
while let Some(update) = stream.next().await {
    if let Ok(Some(E2eEvent::ChatReady { chat_id })) = e2e.handle_client_update(&update).await {
        e2e.send_text(chat_id, "hello", 0).await?;
    }
}
```

See the `secret_chat` example and the Secret Chats page in the docs.

### Message links

`Client::get_message_by_link(link)` resolves `t.me/<username>/<id>`, `t.me/c/<channel_id>/<id>` and `t.me/c/<channel_id>/<topic_id>/<id>`. With a topic ID, the message must be the topic root or belong to that topic. `IncomingMessage::reply_to_top_message_id()` returns the topic root or thread start from the reply header.

```rust
if let Some(msg) = client.get_message_by_link("https://t.me/c/1234567890/42").await? {
    println!("{}", msg.text().unwrap_or_default());
}
```

### TL layer 233

The layer moved from 229 to 233 over the 0.6.7 betas and the final release.

- **230:** `wallet.*` and `toncenter.*` methods, TON Connect sessions, onramp methods, `payments.getCurrencyRates`, `bots.addUsername` and `bots.removeUsername`.
- **232:** share themes, no-paid-messages tokens, star gift deposit, withdraw and display methods, `quote_rich` on reply headers, `in_news` and `pinned_msg_id` on `channel`.
- **233:** a widget API (`inputMediaWidget`, `messageMediaWidget`, bot widget queries and the `messages.*Widget*` functions), audio file transcription (`messages.transcribeAudioFile`, `messages.getTranscriptionLangs`), and the forum sidebar flags. `channels.toggleForum` gains a `sidebar` field. `Client::toggle_forum` sends it as `false`, so topics keep their current layout.

New wallet, widget and forum topic updates arrive as `Update::Raw`.

### Session file permissions

`BinaryFileBackend::owner_only(true)` and `PersistedSession::save_owner_only` make the session file, which holds the auth key, readable by its owner only (mode `0600` on Unix). It is off by default, so existing behavior is unchanged.

### Fixes

- `check_qr_login` always failed with `InvalidCode` because it called `auth.importLoginToken` from the new client. It now polls `auth.exportLoginToken`, and `export_login_token` accepts `auth.loginTokenSuccess` after a DC migration.
- Downloads picked the largest classic photo size and ignored progressive sizes, so a download could be smaller than `size()` reported. The largest size by bytes is used now, and `download_media` falls back to it for photos.
- Transfers retry on the regular DC address when the media-only endpoint can't be reached, and skip that endpoint afterwards.
- `RawUpdate::constructor_id` returned stale ids for `updateMessagePoll` and `updateMessagePollVote`.
- Reply headers built from sent messages in `ferogram-msgbox` keep `quote_rich`.
- The docs no longer claim downloads follow CDN redirects, and many method signatures and examples that had drifted from the code were corrected.

### Upgrading from 0.6.6

```toml
ferogram = "0.6.7"
```

No API changes required. Enable the `e2e` feature if you want secret chats.

---

## v0.6.6

Released 2026-10-21. TL layer 229. Inline and reply keyboards are separate types, QR login handles 2FA and DC migration, and the DC pool only uses connections that finished setup.

### Separate inline and reply buttons

`Button` now builds inline buttons only. Reply keyboard buttons moved to the new `ReplyButton`, exported at the crate root, and `ReplyKeyboard::row` takes `ReplyButton`. Layer 229 has no inline simple webview button, so `Button::mini_app_simple` is gone.

```rust
use ferogram::{Button, ReplyButton, ReplyKeyboard};

let inline = vec![vec![Button::callback("Yes", "yes")]];

let reply = ReplyKeyboard::new()
    .row(vec![ReplyButton::text("Menu"), ReplyButton::request_phone("Share phone")]);
```

### QR login

`Client::check_qr_login` returns `SignInError` instead of `InvocationError`. A 2FA account gives `PasswordRequired`, an expired or invalid token gives `InvalidCode`. `AUTH_TOKEN_ALREADY_ACCEPTED` counts as a successful login, and one DC migration is followed automatically.

### Inline results

`InlineResult::send` takes any `PeerRef` and returns `Option<IncomingMessage>` with the sent message when Telegram includes it.

### More reliable DC pool

`DcPool` no longer opens connections by itself, it only picks slots that finished initConnection and the auth import. A slot is evicted on `CONNECTION_NOT_INITED`, so the next `invoke_on_dc` call redoes initConnection instead of failing forever. Cancelled or failed RPCs no longer leave a slot marked busy. `get_or_create_slot` is removed.

### Other changes

- TL layer 229: ephemeral welcome messages and `EphemeralBotCallbackQuery`, the `manage_welcome_messages` admin right, Firebase PNV login methods and new page blocks.
- `ErrorKind::FileReferenceExpired` is returned for `FILE_REFERENCE_*` errors, so you can refresh the reference and retry. Before, these were `ErrorKind::Transfer`.
- Rich HTML and Markdown parsers support a `compact` attribute for tables.
- `DcConnection::auth_key_bytes` and `first_salt` returned the temporary PFS key and salt. They now return the permanent ones that are safe to save.
- `tracing` no longer enables its `log` feature in the workspace crates.

### Breaking changes

- `Button::text`, `request_phone`, `request_geo`, `request_poll`, `request_quiz` and `mini_app_simple` moved to `ReplyButton`.
- `check_qr_login` returns `SignInError`.
- `InlineResult::send` signature changed as described above.

### Upgrading from 0.6.5

```toml
ferogram = "0.6.6"
```

Move reply keyboard buttons from `Button` to `ReplyButton`, and match on `SignInError` where you call `check_qr_login`.

---

## v0.6.5

Released 2026-07-31. Recommended upgrade. All workspace dependencies are updated, and the release includes performance and safety work across the workspace.

### FakeTLS and crypto fixes

Several proxy bugs were fixed together:

- The FakeTLS ClientHello was a bare 4-extension hello that real proxies dropped. It now has GREASE values, `key_share` (X25519), `signature_algorithms`, ALPN and padding to about 517 bytes.
- FakeTLS framing was treating each TLS record as one MTProto frame. It now carries a PaddedIntermediate stream inside the decoy TLS records, the way real MTProxy does.
- `decrypt_data_v2` rejected valid frames when the server picked a non-zero pad length with `dd` or `ee` secrets, which showed up as intermittent `Crypto(InvalidBuffer)` after a good handshake.
- `MtpSender` silently skipped chunks when `cipher.try_lock()` failed, which permanently desynced the AES-CTR keystream. It now returns an error.
- `channelDifferenceTooLong` no longer panics when a channel dialog has no `pts`. It logs and retries with a fresh difference.

### Transport race

`ClientBuilder::probe_transport_race` takes an ordered list of `RaceLeg { transport, stagger }`. The default race is now two legs, `Full` immediately and `Obfuscated` 200 ms later. The old `Http` leg never sent HTTP frames, it fell back to `Abridged` on the same port, so it was removed.

### Dialogs

- `GetDialogsOptions` adds `exclude_pinned` and `folder_id`. `Client::get_dialogs(limit)` still works through `From<i32>`.
- `DialogCursor` is a serializable cursor, resume with `Client::iter_dialogs_from()`.
- `Client::stream_dialogs` returns a `futures::Stream`.

### Communities and sessions

Telegram Community chats are cached with their own access hashes and survive a session restore instead of turning into plain channels. The binary session format is now v8, older files load unchanged and upgrade on the next save. SQLite and libSQL add an `is_community` column automatically.

### `ferogram-msgbox` and `metrics`

`message_box` moved into its own `ferogram-msgbox` crate. `ferogram` re-exports it as `ferogram::message_box`, so existing code compiles unchanged. The optional `metrics` feature adds counters for RPC and connection events, and a no-op shim keeps the cost at zero when it is off.

### Transfer pipelining default

`download_pipeline_depth` and `upload_pipeline_depth` now default to `1` instead of `4`. Connection count still scales with file size. Raise the depth yourself through `ClientBuilder` or `TransferLimits` if you want in-flight requests per connection.

### Upgrading from 0.6.4

```toml
ferogram = "0.6.5"
```

No API changes required.

---

## v0.6.4

Released 2026-07-14. TL layer 228, tunable transfers, richer media downloads, a callback and inline query dispatcher, and a pipelined sender.

### Communities and ephemeral messages

Layer 228 adds Telegram Communities (`communities.*` methods, `Chat::Community`, `ChatFull::CommunityFull`, `Dialog::Community`) and ephemeral one-time messages (`ephemeral.*` methods, `EphemeralMessage`, `InputReplyTo::EphemeralMessage`).

### Transfer tuning and pipelining

`TransferLimits` and `ClientBuilder::transfer_limits(...)` set how many parallel connections a transfer may open and how deep each connection's request pipeline goes. Shorthands: `download_tcp_connections`, `upload_tcp_connections`, `max_tcp_connections`, `download_pipeline_depth`, `upload_pipeline_depth` and `bypass_tcp_allotments`. Under the hood, `ferogram-mtsender` gains `PipelinedSender` and `spawn_pipelined`, which keep several chunk requests in flight on one connection. Download chunks cap at 512 KB and uploads use a three-tier part size table.

### More things you can download

`Downloadable` now covers `tl::enums::MessageMedia` (resolved through games, web pages, polls, stories and paid media), `ProfilePhoto`, specific thumbnails (`Photo::thumb(type)`, `Document::thumb(type)`), `RawLocation`, video cover photos through `video_cover(media)`, and quality selection with `MediaQuality` and `available_qualities(media)`. `Client::download_file` works with any `D: Downloadable`.

### Dispatcher for callbacks and inline queries

`Router::on_callback_query`, `on_inline_query`, `on_inline_send` and `on_callback_query_fsm` route those updates next to messages. `Filter` is generic over the update type, so the same `&`, `|` and `!` combinators work on `CallbackQuery` and `InlineQuery`.

### Login

`request_login_code_with_options`, `resend_code` and `SendCodeOptions` cover flash call, missed call and Firebase delivery. `sign_out()` keeps the `future_auth_token` Telegram returns and replays it on the next `request_login_code`, so a re-login can skip code entry. In that case you get `SendCodeOutcome::AlreadyAuthorized`. `Client::interactive_sign_in` is now public.

### Smaller additions

- `BannedRightsBuilder` for granular channel ban rights. `ban`, `kick` and `restrict` all share one implementation built on it.
- `Client::get_chat_photos(peer, limit)` returns photo history for groups and channels.
- `Client::cache_entities(users, chats)` feeds users and chats from a raw RPC call into the peer cache.
- `ClientBuilder::dc_id_override(dc_id)` registers a connection under a custom DC id.
- `PeerCache::stats()`, `PersistedSession::stats()`, `UserFull` and `MessagePage` helpers.
- Session saves and snapshot flushes run on `spawn_blocking`, so SQLite and file writes no longer stall async tasks.

### Fixes

- The dc-tagged `p_q_inner_data` is no longer sent on normal connects. Telegram rejects it with RPC 444 outside DC2.
- Reconnect keeps a still-valid auth key on timeout, and clears a rejected one (RPC 401, `AUTH_KEY_UNREGISTERED`) before retrying with fresh DH.
- `get_chat_participants` returns a clear error for `ChatFull::CommunityFull`.
- `#[derive(FsmState)]` match-arm codegen and crate path resolution.

### Breaking changes

- `request_login_code` and `resend_code` return `SendCodeOutcome` instead of `LoginToken`. Match on `SendCodeOutcome::CodeRequired(token)` to get the old value.
- `media::download_worker_count` and `upload_worker_count` take an extra `max_workers: usize`.

### Upgrading from 0.6.3

```toml
ferogram = "0.6.4"
```

Update the `request_login_code` call sites to match on `SendCodeOutcome`.

---

## v0.6.3

Released 2026-06-28. Transfer progress tracking, resumable transfers, and typed errors.

### Transfer progress and control

`TransferHandle` and `TransferProgress` let you track, pause, resume, or cancel any upload or download. Pass a handle as the last argument to `upload_file`, `upload`, `download`, or `download_file`. Pass `None` to keep old behavior.

```rust
use ferogram::{Client, TransferHandle};

let handle = TransferHandle::new();
let ctl = handle.clone();

tokio::spawn(async move {
    tokio::time::sleep(std::time::Duration::from_secs(5)).await;
    ctl.cancel();
});

let uploaded = client.upload_file("/tmp/video.mp4", Some(&handle)).await?;
```

### Typed errors

`InvocationErrorExt` trait adds `.kind()` and `.friendly()` on `InvocationError`. `.kind()` returns an `ErrorKind` enum so you can tell a flood wait from a network failure from a cancelled transfer without matching on strings.

```rust
use ferogram::InvocationErrorExt;

match result {
    Err(e) if matches!(e.kind(), ferogram::ErrorKind::Transfer) => println!("transfer: {}", e.friendly()),
    Err(e) => println!("other: {e}"),
    Ok(_) => {}
}
```

### Auto media type and `From<UploadedFile>`

`UploadedFile::as_auto_media()` picks the right Telegram media type from MIME (photo, video, audio, voice, animation, or document). `UploadedFile` now implements `Into<InputMedia>` so you can pass it directly to `send_file`.

### Resumable transfers (experimental)

`download_resumable` and `upload_resumable` are available under `features = ["experimental"]`. Interrupted transfers save a checkpoint JSON and a `.partial` bytes file to a directory you specify. On the next call with the same media the download picks up from the saved offset. Checkpoint files are deleted on success.

### Breaking changes

- `upload_file_from_path` renamed to `upload_file`.
- `upload_file`, `upload`, `download`, `download_file` all take `handle: Option<&TransferHandle>` as a new last param.
- `send_file` now takes `impl Into<InputMedia>` instead of `InputMedia` directly.

### Upgrading from 0.5.2

```toml
ferogram = "0.6.3"
```

Rename `upload_file_from_path` call sites to `upload_file`. Add `None` as the last argument to all upload and download calls.

---

## v0.6.2

Released 2026-06-24. Transport, reconnect and update synchronization stabilization.

### Pagination

`Client::get_message_history` and `Client::get_replies` take an extra `add_offset: i32` for offset-based pagination. Pass `0` for the previous behavior.

### Reconnect and transport

Frame parsing and validation in Full transport mode was fixed, along with transport desyncs that caused CRC mismatches. Reconnect handling after network interruptions and auth-key reuse is more reliable, and several cases that left a client disconnected after connectivity returned are gone. Update gap recovery and account and channel difference scheduling were fixed too. Ownership boundaries between `ferogram`, `ferogram-connect`, `ferogram-mtsender`, `ferogram-session` and `ferogram-parsers` were cleaned up.

### Diagnostics

Logs now carry connection generation, reader lifecycle and session identity (`[conn_gen=X]`, `[reader#N]`, `[sid=...]`), plus frame-level transport tracing for sequence tracking and CRC validation.

### Upgrading from 0.6.0

```toml
ferogram = "0.6.2"
```

Add `0` as the `add_offset` argument to `get_message_history` and `get_replies` calls.

---

## v0.6.0

Released 2026-06-01. Transfer progress and control, typed errors, and resumable transfers. The 0.6.3 section below covers the same APIs with examples.

- `TransferHandle` and `TransferProgress` for progress, pause, resume and cancel.
- `InvocationErrorExt` with `.kind()` and `.friendly()`.
- `UploadedFile::as_auto_media()` and `From<UploadedFile> for InputMedia`.
- `download_resumable` and `upload_resumable` under the `experimental` feature.
- Fixed: `download_resumable` ignored its saved offset and restarted from byte 0, and compared a partial SHA-256 against the final file. Interrupted downloads now flush to a `.partial` file that survives a restart.
- Breaking: `upload_file_from_path` is now `upload_file`, upload and download calls take a `handle: Option<&TransferHandle>` last argument, and `send_file` takes `impl Into<InputMedia>`.

---

## v0.5.2

Released 2026-05-31. Channel kind tracking and session format v6.

### `channel_kind()` on `IncomingMessage`

Four new async methods: `channel_kind()`, `is_megagroup()`, `is_broadcast()`, `is_gigagroup()`. The fast path reads from a per-batch peer map built at dispatch time, so live updates never need to acquire the session cache lock. The slow path falls back to the session cache.

```rust
if msg.is_megagroup().await {
    // message is from a supergroup
}

match msg.channel_kind().await {
    Some(ChannelKind::Broadcast) => { /* channel post */ }
    Some(ChannelKind::Megagroup) => { /* supergroup */ }
    Some(ChannelKind::Gigagroup) => { /* gigagroup */ }
    None => { /* not a channel, or kind not yet known */ }
}
```

### Session format v6

Each channel peer now stores a `ChannelKind` byte (0xFF = absent). Old sessions from v2-v5 load without changes and pick up kind data the next time the peer appears in an update.

SQLite and libSQL both add the `channel_kind` column automatically on first open.

### Upgrading from 0.5.1

```toml
ferogram = "0.5.2"
```

No API changes required.

---

## v0.5.1

Released 2026-05-31. Single bug fix: bots were skipping every first message.

### Fixed: MessageBoxes dropping every first message

`force_update_entry` was seeding the Common pts from the arriving update instead of the real server baseline. This caused every odd-numbered incoming message to be dropped silently. Fixed.

### Upgrading from 0.5.0

```toml
ferogram = "0.5.1"
```

No API changes.

---

## v0.5.0

Released 2026-05-16. API consolidation release. Paired functions that differed only by a single boolean condition have been merged into one. Download and upload paths were redesigned around `AsyncRead`/`AsyncWrite`. No new protocol or behavioural changes.

### Breaking changes

**Merged paired functions into one**

| Removed | Replacement |
|---------|-------------|
| `set_online()` / `set_offline()` | `set_presence(online: bool)` |
| `block_user(peer)` / `unblock_user(peer)` | `block(peer, true/false)` |
| `pin_dialog(peer)` / `unpin_dialog(peer)` | `pin_dialog(peer, true/false)` |
| `archive_chat(peer)` / `unarchive_chat(peer)` | `archive(peer, true/false)` |
| `pin_message(peer, id, silent)` / `unpin_message(peer, id)` | `pin_message(peer, id, true/false)` |
| `delete_channel(peer)` / `delete_chat(id)` | `delete_chat(peer)` (dispatches by peer type) |
| `install_sticker_set(set, archived)` / `uninstall_sticker_set(set)` | `toggle_stickers(set, true/false)` |
| `get_broadcast_stats(peer)` / `get_megagroup_stats(peer)` | `stats(peer) -> ChannelStats` |
| `get_poll_stats(peer, id)` / `get_poll_results(peer, id)` | `poll_results(peer, id)` |
| `promote_participant` / `demote_participant` | `set_admin(peer, user, rights)` |
| `ban_participant(peer, user)` / `ban_participant_until(peer, user, ts)` | `ban(peer, user, until: Option<i32>)` |
| `kick_participant(peer, user)` | `kick(peer, user)` |
| `set_banned_rights(peer, user, rights)` | `restrict(peer, user, rights)` |
| `set_admin_rights(peer, user, rights)` | `set_admin(peer, user, rights)` |
| `set_profile(first, last, about)` / `set_username(u)` / `set_emoji_status(id, until)` / `edit_chat_title` / `edit_chat_about` / `edit_chat_photo` | `set_profile(peer) -> SetProfileBuilder` |
| `get_message_by_id(peer, id)` / `get_messages_by_id(peer, ids)` | `get_messages(peer, ids)` |
| `mark_as_read(peer)` / `mark_dialog_read(peer)` | `mark_read(peer)` |
| `resolve_peer(str)` | `resolve(str)` (or pass peer string directly via `PeerRef`) |
| `accept_invite_link(link)` | `join_link(link)` |

**Download and upload API redesigned**

Old API passed raw `InputFileLocation` handles. New API works directly with `&MessageMedia`:

```rust
// download to any AsyncWrite sink
client.download(msg.media().unwrap(), &mut file).await?;

// download to disk
client.download_file(msg.media().unwrap(), "photo.jpg").await?;

// lazy chunk iterator
let mut iter = client.iter_download(msg.media().unwrap()).unwrap();
while let Some(chunk) = iter.next().await? { ... }

// upload from any AsyncRead
let uploaded = client.upload(reader, "file.jpg").await?;

// upload from path (stats file, streams to upload)
let uploaded = client.upload_file("photo.jpg").await?;
```

`IncomingMessage` also gains two convenience methods:

```rust
msg.download(&mut buf).await?;   // stream to AsyncWrite
let bytes = msg.bytes().await?;  // into Vec<u8>
```

**`set_profile` is now a builder**

```rust
// user
client.set_profile("me").name("Alice", "").bio("Hello!").send().await?;

// channel / group
client.set_profile("@mychannel").title("New Name").bio("About text").send().await?;
```

**`stats` returns `ChannelStats`**

```rust
match client.stats("@mychannel").await? {
    ChannelStats::Broadcast(s) => { /* channel */ }
    ChannelStats::Megagroup(s) => { /* supergroup */ }
}
```

**Upload part-size table revised**

Five tiers keyed on file size (< 1 MB, 1-32 MB, 32-512 MB, 512 MB-1 GB, > 1 GB) replace the old two-tier heuristic. This reduces per-chunk overhead for small files and stays safely below the 4000-part hard limit for large ones.

### Upgrading from 0.4.1

```toml
ferogram = "0.5.0"
```

Rename call sites per the table above. The session format, feature flags, and all other APIs are unchanged.

---

## v0.4.1

Released 2026-05-14. Patch on top of 0.4.0: one new API for faster onboarding, a configurable update buffer, session schema improvements, and 15 new examples. No breaking changes.

### `Client::quick_connect`

One call handles the builder, connection, and the full auth flow from stdin. Returns immediately if the session is already authorized.

```rust
use ferogram::Client;

const API_ID: i32 = 12345;
const API_HASH: &str = "your_api_hash";

let (client, _shutdown) = Client::quick_connect("bot.session", API_ID, API_HASH).await?;
```

Bot tokens are detected automatically by their `<digits>:<string>` shape, so the same call works for both bots and users.

If you need an option `quick_connect` does not expose (proxy, PFS, custom transport, catch-up), switch to `Client::builder()`. The session file is compatible.

See [quick_connect reference](./api/quick-connect.md) for the full signature and error table.

### Configurable update buffer

Two new builder methods let you tune the user-facing dispatch queue. Internal MTProto state (pts, qts, getDifference) is unaffected; only the `Update` queue from `stream_updates()` is governed here.

```rust
use ferogram::{Client, update_config::OverflowStrategy};

let (client, _) = Client::builder()
    .api_id(API_ID)
    .api_hash(API_HASH)
    .session("bot.session")
    .update_queue_capacity(512)
    .update_overflow_strategy(OverflowStrategy::DropOldest)
    .connect().await?;
```

`DropOldest` (default) evicts ephemeral updates (typing, online status) first, then the oldest normal update, keeping the incoming one. `DropNewest` discards the incoming update instead. Default capacity is 2048.

For memory-constrained hosts (Termux, small VPS) there is a shortcut:

```rust
Client::builder()
    // ...
    .low_memory_mode(true)  // 256-slot queue, DropOldest
    .connect().await?;
```

### Session schema migration

Two additions to the SQLite/LibSQL schema, applied automatically when the database is opened. Both are no-ops on a fresh database.

- `peers` table gains an `is_chat` column for tracking basic groups.
- New `min_peers` table stores min-user message contexts needed for `InputPeerUserFromMessage`.

Existing session files migrate automatically.

### Periodic session save

The client now writes a full session snapshot (peers, channel_pts, min_peers, DC auth/salt data) to the session backend every 60 seconds whenever the peer cache has been mutated since the last save. A final save runs unconditionally on shutdown. Previously peers were only written on explicit `save_session()` calls, so a crash between calls could lose recently seen peers.

### Salt expiry fix past 2038

`valid_until` in future salts was stored as `i32`. Telegram sends validity windows that extend into the 2050s. Those values overflow `i32` and wrap negative, making every salt look expired on a signed comparison and causing constant re-fetches. Changed to `u32` throughout.

### `GuestChatAnswer` return type corrected

`GuestChatAnswer::send` now returns `InputBotInlineMessageID` matching the actual TL schema. The `setBotGuestChatResult` constructor ID was also wrong in the previous schema; both are fixed. If you were pattern-matching on the old `bool` return, update your call sites.

### New type and methods

- `ParticipantStatus` is now exported from the crate root.
- `User::bot_guestchat()` returns `true` if the bot supports guest-chat mode.
- `GuestChatQuery::via_from()` returns the original requester peer when Telegram includes `guestchat_via_from` in the message.

### 15 new examples

All examples live under `ferogram/examples/`.

**Userbot tools:** `admin_log`, `chat_history`, `dialogs_list`, `download_media`, `get_participants`, `schedule_message`, `search_messages`, `serverless_userbot`, `string_session_gen`.

**Bots:** `echo_bot`, `filters_showcase`, `hello_self`, `inline_keyboard`, `inline_query_bot`, `poll_bot`, `translate_bot`.

### Upgrading from 0.4.0

```toml
ferogram = "0.4.1"
```

No API changes required. The session migration is automatic.

---

## [0.4.0]: 2026-05-08

0.4.0 is the first production-ready release of ferogram. It ships Layer 225 support and a reworked poll API. All users are advised to upgrade to 0.4.0 (or 0.4.x+) as the most recommended and supported version.

If you run into any bugs, please open an issue on GitHub or reach us at [@FerogramChat](https://t.me/FerogramChat). Thank you for using ferogram!

For the latest git revision: https://github.com/ankit-chaubey/ferogram

> **Note:** 0.3.9 was a broken publish. The workspace internal deps were not bumped so crates.io resolved `ferogram-tl-types` to the old Layer 224 build. 0.4.0 fixes that and is the correct release to use.

---

## v0.3.9

Released 2026-05-07. Updated to TL Layer 225. Poll builder overhaul, guest-chat support for bots, two new client methods, and full MarkdownV2/HTML spec compliance in the parsers.

### `send_poll` now takes `PollBuilder` (breaking)

The old flat signature (`question, answers, quiz, correct_index, multiple_choice`) is gone. Pass a `PollBuilder` instead:

```rust
use ferogram::PollBuilder;

client.send_poll(peer,
    PollBuilder::new("Favourite runtime?")
        .answers(["Tokio", "async-std", "smol"])
        .public_voters(true)
        .close_period(300)
).await?;

// Quiz with answer explanation
client.send_poll(peer,
    PollBuilder::new("Capital of France?")
        .answers(["Berlin", "Paris", "Rome"])
        .quiz(true)
        .correct_index(1)
        .solution("It's Paris.")
        .hide_results_until_close(true)
).await?;
```

New fields the old API did not expose: `public_voters`, `shuffle_answers`, `hide_results_until_close`, `close_period`, `close_date`, `solution`, `subscribers_only`, `countries_iso2`.

### Guest-chat queries (bots only)

A new `Update::GuestChatQuery` variant handles `updateBotGuestChatQuery`. Fires when a user invites the bot into a guest-chat context. `GuestChatQuery` derefs to `IncomingMessage` and carries `query_id`, `message`, `reference_messages`, and `qts`.

Answer with the `GuestChatAnswer` builder:

```rust
if let Update::GuestChatQuery(q) = update {
    q.answer()
        .article("My result")
        .text("The answer")
        .send(&client)
        .await?;
}
```

Supported result kinds: `article`, `photo`, `document`, `game`, `location`, `venue`, `contact`, `webpage`, `invoice`, `raw`. Sends via `messages.setBotGuestChatResult`.

### New client methods

**`delete_reaction(peer, msg_id, participant)`** reports and removes a specific user's reaction on a message. Returns `true` on success.

**`get_poll_stats(peer, msg_id)`** returns detailed vote stats for a poll. Returns `tl::types::stats::PollStats`.

### `BannedRights::send_reactions`

New field on the ban-rights builder:

```rust
BannedRights::default().send_reactions(false)
```

### MarkdownV2 and HTML parser update (`ferogram-parsers`)

`parse_markdown` and `generate_markdown` now implement the full Telegram Bot API MarkdownV2 spec. The main breaking change: `__text__` is Underline now, not Italic. If you relied on the old behaviour, call `parse_markdown_v1` explicitly (deprecated, removed in 0.4.0).

Explicit aliases added: `parse_markdown_v2`, `generate_markdown_v2`.

HTML: added `<ins>` as underline, `<span class="tg-spoiler">` as spoiler, `<blockquote>`/`<blockquote expandable>` for block quotes, `<tg-time unix="N">` for formatted dates. The `<pre><code class="language-X">` bug that produced two entities instead of one is fixed. `generate_html` emits all of the above.

### Upgrading from 0.3.8

```toml
ferogram = "0.3.9"
```

Two things to fix: `send_poll` call sites (see above), and any markdown that relied on `__text__` being Italic (change to `_text_`).

---

## v0.3.8

Released 2026-05-06. A small patch release fixing two broken APIs from 0.3.7.

### `send_to_self` is fixed

In 0.3.7 the function body got swapped during a refactor. Calling `send_to_self(msg)` was silently hitting the wrong code path. It now correctly sends to your Saved Messages using `messages.sendMessage` with `InputPeer::PeerSelf`, and returns the sent message as before.

### `open_mini_app` is now public

`open_mini_app(peer, MiniApp)` was accidentally left private in 0.3.7. It's `pub` now. Supports all four mini-app types: `Main`, `Url`, `App`, and `Simple`.

### `get_chat_full` is now public

Was `pub(crate)` before. Now `pub`, so you can call it directly if you need the raw full chat info without going through a helper.

---

## v0.3.7

Released 2026-05-05. Workspace restructure, three new crates, and a handful of API cleanups.

### New crates

Three crates were extracted this release. The main `ferogram` crate re-exports everything from them, so if you're not doing anything low-level, your code doesn't need to change.

**ferogram-connect** is now a real crate instead of a throwaway demo binary. It owns the raw TCP connection layer, MTProto framing, transport negotiation (Intermediate, Obfuscated, FakeTLS), SOCKS5, and proxy support. Useful if you want to build something that speaks MTProto without pulling in the full client.

**ferogram-fsm** packages the FSM layer (`FsmState`, `StateContext`, `StateStorage`, `MemoryStorage`) as a standalone crate that can be versioned and published on its own.

**ferogram-mtsender** does the same for the sender pool and retry policy. `RetryPolicy`, `AutoSleep`, `CircuitBreaker`, `NoRetries` all live here now.

The old `ferogram-app` and `ferogram-bot` example binaries were removed. They've been replaced by examples inside `ferogram/examples/`.

### Getting a peer's ID with `.bare_id()`

You used to need a full match to pull a numeric ID out of a `tl::enums::Peer`. Now there's `PeerExt`:

```rust
use ferogram::{PeerExt, OptionPeerExt};

// any Peer variant → i64
let id = peer.bare_id();

// works on Option<&Peer> too, no .map() needed
let sender = msg.sender_id().bare_id(); // Option<i64>
let chat   = msg.peer_id().bare_id();   // Option<i64>
```

The name `bare_id` is intentional: it gives you the **native** Telegram ID, not the Bot-API-encoded one. A channel with native ID `1234567890` is `-1001234567890` in the Bot API.

### `PeerCache` and `ExperimentalFeatures` are public

`PeerCache` is now in its own file and fully public. It's what handles every peer lookup: user hashes, channel hashes, basic groups, min-users, the username index, the phone index. You can read from it directly if you need low-level access.

`ExperimentalFeatures` lets you opt into behaviours that deviate from strict Telegram spec. The main flag is `allow_zero_hash`, which lets bots skip needing a cached access hash (don't use this on user accounts):

```rust
Client::builder()
    .experimental_features(ExperimentalFeatures {
        allow_zero_hash: true,
        ..Default::default()
    })
    .connect().await?;
```

### Breaking changes

**`download_media_to_file` is now `download_file`:**

```rust
// before
client.download_media_to_file(location, &path).await?;

// now
client.download_file(location, &path).await?;
```

**`forward_messages` now takes a fourth argument:**

```rust
// before
client.forward_messages(dest, &[id], src).await?;

// now
client.forward_messages(dest, &[id], src, ForwardOptions::default()).await?;
```

**`respond_ex` is gone.** `respond` already accepts `InputMessage`, so it was redundant:

```rust
// before
msg.respond_ex(InputMessage::html("<b>hi</b>")).await?;

// now
msg.respond(InputMessage::html("<b>hi</b>")).await?;
```

### Upgrading from 0.3.6

```toml
ferogram = "0.3.7"
```

The three breaking changes above need fixing. The rest is additive.

---

## v0.3.6

Released 2026-04-30. API stabilization update towards v0.4.0.

Some APIs have been simplified, merged, or removed where redundant. This may require a one-time migration. The goal is a consistent, predictable API that does not need disruptive changes again.

Future updates will focus on new features and improvements.

See [FEATURES.md](https://github.com/ankit-chaubey/ferogram/blob/main/FEATURES.md) for the full current API surface.

### Upgrading from 0.3.5

```toml
ferogram = "0.3.6"
```

---

## v0.3.5

Released 2026-04-30. Critical deserialization fix and update-state hardening.

### PollResults deserialization fix

`PollResults` was incorrectly treated as a bare type throughout the codebase,
meaning the 4-byte constructor ID was never read from the wire. The deserializer
consumed that ID as the `flags` field instead, producing garbage flag values and
misaligning every subsequent field read. Any `getChannelDifference` or
`getDifference` response that contained a poll message would fail with an
unexpected constructor id error and drop the entire update batch.

The fix routes `PollResults` through `crate::enums::PollResults` like every
other boxed type, so the constructor ID is read and validated before fields are
deserialized. Both `MessageMediaPoll.results` and `updateMessagePoll.results`
are affected.

### getDifference self-deadlock fix

The `reader_loop` select arm that fires the MessageBoxes gap deadline was
directly awaiting `run_pending_differences()`. Because `reader_loop` is the
only task reading TCP frames, the getDifference RPC it sent could never receive
a response, producing a 30-second hang after any gap detection. The fix spawns
a separate task for the diff runner, matching the pattern already used by the
keepalive arm. A `diff_in_flight: AtomicBool` guard prevents duplicate spawns
while a diff is already in progress.

### Lazy access-hash resolution

Channel access hashes are now resolved purely from incoming update entities and
the persisted peer cache. The automatic `GetDialogs` call at startup and
catch-up has been removed. This makes ferogram resilient to Telegram schema
changes in dialog-related types without requiring a layer bump.

`Client::warm_peer_cache_from_dialogs()` is a new public opt-in method for
cases where you need access hashes before any update has arrived for a channel.
See [Raw API Access](./advanced/raw-api.md#peer-cache-helpers) for usage details.

### Upgrading from 0.3.4

```toml
ferogram = "0.3.5"
```

No API changes required. The fix is automatic.

---

## v0.3.4

Released 2026-04-28. MTProto hardening release: PFS temp-key sessions, access-hash prefetch on startup, and safer deserialization across the board.

### PFS (Perfect Forward Secrecy)

A new `.pfs(true)` method on `ClientBuilder` enables Perfect Forward Secrecy at the transport layer. When set, the DC pool performs a temporary DH key bind immediately after the permanent auth key is established. The connection then runs under a short-lived session key derived from that bind; the permanent key is never used to encrypt traffic directly. If the bind RPC fails for any reason the pool falls back to the standard session without disrupting the connection.

### Access-hash prefetch

`prefetch_channel_access_hashes` is now called automatically at startup and after every catch-up cycle. It issues a single `GetDialogs` request and caches all returned channel and user access hashes before the first live update is dispatched. In practice this eliminates the `CHANNEL_INVALID` errors that previously appeared on reconnects when an incoming update referenced a channel the in-memory cache had not yet seen.

### `from_bytes_exact`

`Deserializable::from_bytes_exact` is a new method available on all TL types. It wraps the common `Cursor::from_slice` + `deserialize` pattern and additionally returns an error if any bytes remain unconsumed after deserialization. All call sites across `lib.rs`, `dc_pool.rs`, and `pts.rs` have been migrated to it. Parse failures on incoming `Updates` frames are now logged as warnings instead of being silently discarded.

### Concurrent `get_difference` fix

Previously, if two tasks raced to call `get_difference` at the same time, the second would return immediately with an empty result and potentially miss a fill cycle. It now polls every 50 ms waiting for the in-flight call to finish, and gives up after 35 s with a warning so the next gap tick can retry rather than hanging indefinitely.

### Upgrading from 0.3.3

```toml
ferogram = "0.3.4"
```

To enable PFS:

```rust
let (client, _shutdown) = Client::builder()
    .api_id(12345)
    .api_hash("your_hash")
    .session("bot.session")
    .pfs(true)
    .connect()
    .await?;
```

---

## v0.3.3

Released 2026-04-22. Bot framework release: composable filters, finite state machine, middleware pipeline, conversation API, and a new proc-macro crate.

### ferogram-derive

A new `ferogram-derive` crate adds the `#[derive(FsmState)]` proc-macro. Applying it to a unit-variant enum generates `as_key` and `from_key` implementations automatically. The crate is gated behind a `derive` feature flag and `FsmState` is re-exported from the crate root, so the only import you need is `use ferogram::FsmState`.

### Filters

`ferogram::filters` provides composable, synchronous predicates over `IncomingMessage`. Built-in constructors cover the common cases: `command`, `private`, `text`, `media`, and others. Predicates compose with `&`, `|`, and `!` operators, so you can express things like `command("start") & private()` directly in the handler registration. Filters also integrate with the FSM via `StateContext`, letting you gate handlers on the current conversation state.

### FSM

`ferogram::fsm` provides the full finite state machine layer: the `FsmState` trait, `StateContext`, `StateKey`, `StateKeyStrategy`, and `StateStorage`. The default storage is an in-memory `DashMap`-backed store keyed by peer. Custom backends can be plugged in via an async-trait extension point, so SQLite or Redis-backed stores are easy to add. A new `examples/order_bot.rs` walks through a multi-step order flow driven by the FSM.

### Middleware

`ferogram::middleware` adds a `Middleware` trait and a `Next` chain that wraps every handler dispatch. The crate ships a ready-to-use rate-limit middleware backed by `DashMap`. `DispatchError` and `DispatchResult` are exported for use in custom middleware.

### Conversation

`ferogram::conversation` provides a `Conversation` type for sequential, stateful exchanges with a single peer. It wraps an `UpdateStream` scoped to the conversation lifetime and transparently buffers updates arriving from other peers during the exchange.

### IncomingMessage helpers

`IncomingMessage` gained a full set of inspection methods: `chat_id`, `is_private`, `is_group`, `is_channel`, `is_any_group`, `from_id`, `is_bot_command`, `command`, `is_command_named`, `command_args`, `has_media`, `has_photo`, `has_document`, `is_forwarded`, `is_reply`, and `album_id`.

### New update types

Eight new update types are now exported from the crate root: `ParticipantUpdate`, `JoinRequestUpdate`, `MessageReactionUpdate`, `PollVoteUpdate`, `BotStoppedUpdate`, `ShippingQueryUpdate`, `PreCheckoutQueryUpdate`, and `ChatBoostUpdate`.

### New API method

`Client::get_chat_administrators()` returns all admins and the creator for a channel or supergroup. For basic groups it returns all participants; use the `is_admin` field on the result to distinguish.

### New documentation pages

Bot Framework: Middleware & Dispatcher, Finite State Machine (FSM), Conversation API. API reference: Bot Configuration, Stats & Analytics.

### Upgrading from 0.3.2

```toml
ferogram = "0.3.3"
```

To use `#[derive(FsmState)]`:

```toml
ferogram = { version = "0.3.3", features = ["derive"] }
```

---

## v0.3.2

Released 2026-04-21. Correctness and session-save hardening.

### SeenMsgIds

The `SeenMsgIds` deque is now paired with a `HashSet` so duplicate checks under concurrent workers are O(1) instead of O(n). On busy connections receiving many server messages simultaneously this removes a hot path that was linear in the deque length.

### Session save race

Session temp files now get a unique name per write, and a `write_lock` serializes concurrent saves. Previously two concurrent saves could race on the rename step, which caused data loss on Windows. Both are now safe.

### Bug fixes

Five correctness bugs were patched:

The `PaddedIntermediate` handshake was not being sent on DC pool worker connections. Without it the server would silently drop or misparse every frame from those connections.

`new_session_created` was resetting the session on fresh connections even when it should not, which caused a session ID mismatch on every subsequent decrypt.

`scan_body` was passing `None` as `sent_msg_id` during container iterations, letting stale cached results overwrite live responses from the server.

The `importAuthorization` branch condition was inverted, so the import was skipped precisely in the cases where it was required and ran in cases where it was not.

Server 4-byte transport error codes received during the DH handshake are now surfaced properly instead of being misclassified as "plain frame too short".

---

## v0.3.1

Released 2026-04-20. Patch release fixing the docs.rs build. No functional changes from 0.3.0.

### Upgrading from 0.3.0

```toml
ferogram = "0.3.1"
```

---

## v0.3.0

Released 2026-04-19. The biggest release so far: two new crates, a redesigned connection stack, CDN file download support, and a much larger API reference.

### Two new crates

**ferogram-session** takes over all session persistence. It owns `PersistedSession`, `DcEntry`, `DcFlags`, `UpdatesStateSnap`, `CachedPeer`, `CachedMinPeer`, `default_dc_addresses`, and all storage backends: `BinaryFileBackend`, `InMemoryBackend`, `StringSessionBackend`, `SqliteBackend`, and `LibSqlBackend`. The main `ferogram` crate re-exports everything from it, so existing code needs no changes.

**ferogram-parsers** takes over Telegram entity parsing. It provides `parse_markdown`, `generate_markdown`, `parse_html`, and `generate_html`. An optional `html5ever` feature swaps in a spec-compliant HTML5 tokenizer. The main `ferogram::parsers` module re-exports these, so again, nothing changes for most users.

Both crates can also be used as standalone dependencies if you only need the session or parser layer without the full client.

### Session format

The binary session format moved to version 5. It now persists the home DC, the full DC table with per-DC auth keys and flags, update state (pts, qts, date, seq), per-channel pts values, the peer access-hash cache, and min-user message contexts for `InputPeerUserFromMessage`. Older session files still load without error. Saves are atomic: written to a `.tmp` file first, then renamed into place, so a crash during save cannot corrupt the session. DC flags are now persisted, which means media and CDN DC entries survive restarts.

### Connection options

`ClientBuilder` gained three new methods.

`.probe_transport(true)` races Obfuscated, Abridged, and HTTP transports at connect time and uses whichever one succeeds first. Useful on networks where one transport is throttled or blocked. Has no effect when MTProxy is configured.

`.resilient_connect(true)` adds two fallback layers when direct TCP fails. First it tries DNS-over-HTTPS, querying both Google DoH and Mozilla/Cloudflare DoH. If that also fails, it tries Telegram's Firebase/Google special-config endpoint to get working DC addresses. Intended for restricted networks where normal TCP and DNS are both unreliable.

`.experimental_features(...)` accepts an `ExperimentalFeatures` struct with three fields: `allow_zero_hash`, `allow_missing_channel_hash`, and `auto_resolve_peers` (reserved, not yet active).

### CDN downloads

A new `ferogram::cdn_download` module handles the full Telegram CDN file path. It requests chunks via `upload.getCdnFile`, re-uploads stale chunks via `upload.reuploadCdnFile`, and decrypts each chunk with AES-256-CTR using the key and IV provided by Telegram. Exports `CdnDownloader`, `CdnChunkResult`, and `CDN_CHUNK_SIZE`. Used internally when large files are served from a CDN DC rather than the main DC.

### DNS-over-HTTPS and special-config

`ferogram::dns_resolver` queries Google DoH and Mozilla/Cloudflare DoH, merges IPv4 and IPv6 answers, and caches results by TTL.

`ferogram::special_config` implements Telegram's last-resort fallback: decodes the encrypted response from Telegram's Firebase/Google endpoint and extracts DC addresses from `help.configSimple`.

### MTProto internals

`ferogram-mtproto` gained a `bind_temp_key` module and now re-exports `encrypt_bind_inner`, `gen_msg_id`, `serialize_bind_temp_auth_key`, `EncryptedSession`, `SeenMsgIds`, and `new_seen_msg_ids`. Primarily useful for library authors working at the MTProto layer directly.

### New documentation pages

Advanced: CDN Downloads, Transport Probing and Resilient Connect, Connection Restart Policy, Experimental Features.

API reference: ClientBuilder, Types Reference, Chat Management, Contacts and Blocking, Forum Topics, Games and Payments, Invite Links, Polls and Votes, Privacy and Notifications, Profile and Account, Stickers.

### Upgrading from 0.2.0

```toml
ferogram = "0.3.0"
```

No API changes required. If you want to use the new connection options:

```rust
let client = Client::builder()
    .probe_transport(true)
    .resilient_connect(true)
    .connect()
    .await?;
```

---

## v0.2.0

Released 2026-04-13. Focused on concurrency, protocol correctness, and transport hardening.

### Concurrency

The peer cache moved from `RwLock<HashMap>` to a `moka` concurrent cache, removing lock contention during peer lookups. The pending RPC map was replaced with `DashMap` for lock-free response routing. The DC pool switched from `parking_lot::Mutex` to `tokio::sync::Mutex` so it no longer blocks the async runtime during DC operations.

### Protocol correctness

Fresh DH sessions now wait 2 seconds after key derivation to allow Telegram to propagate the new auth key across DCs before the first request is sent. Stale key detection was simplified: only error `-404` triggers key rotation now. `getDifference` deserialization tolerates unknown server responses instead of failing and dropping buffered updates. Container message parsing validates inner message alignment and safely discards malformed frames.

### Transport

FakeTLS transport now prepends the Change Cipher Spec record to the first application data chunk, matching the TLS handshake pattern Telegram expects. Transport errors `-429` and `-444` are now logged clearly before reconnecting rather than failing silently.

---

## v0.1.0

Released 2026-04-11. The initial release of ferogram.

### Proxy and transport

Full MTProxy support via `t.me/proxy` or `tg://proxy` links, or manually with host, port, and secret. PaddedIntermediate transport (`0xDD` secrets) adds randomized padding to blend in with official Telegram client traffic. FakeTLS transport (`0xEE` secrets) wraps MTProto in TLS-like framing. SOCKS5 proxy with optional username and password. IPv6 connectivity for both Telegram DCs and proxy connections.

### Session backends

Binary file, in-memory, string/base64, SQLite, and libSQL.

### Protocol fixes

Auth key generation now uses the correct `PQInnerDataDc` constructor with the DC id included, resolving auth failures on many DCs. Incoming message validation uses a rolling buffer of the last 500 server `msg_id` values plus a 300 second timestamp window to prevent replay attacks. DH step 3 retry (`dh_gen_retry`) retries with cached params for up to 5 attempts, matching Telegram Desktop behavior. MTProxy connections now correctly route through the proxy host instead of going directly to Telegram DCs. `getChannelDifference` starts at limit 100 and increases to 1000 on subsequent calls.

---

See the full [CHANGELOG](https://github.com/ankit-chaubey/ferogram/blob/main/CHANGELOG.md) for the raw entry format.
