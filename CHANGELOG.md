# Changelog

All notable changes to this project will be documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.0.0/)
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

> **Note:** ferogram is the continuation of [layer](https://github.com/ankit-chaubey/layer).
> For history prior to v0.1.0, see the [layer changelog](https://github.com/ankit-chaubey/layer/blob/main/CHANGELOG.md) (up to layer v0.5.0).

---

## [0.6.0] - 2026-05-29

### Added

- `TransferHandle` and `TransferProgress` in `ferogram::transfer`. Pass a handle to any upload or download to track progress, pause, resume, or cancel. Both are re-exported from the crate root.
- `InvocationErrorExt` trait with `.kind()` and `.friendly()` on `InvocationError`. Returns `ErrorKind` enum (`FloodWait`, `Network`, `Auth`, `Migration`, `Rpc`, `Transfer`, `Other`) or a readable string.
- `UploadedFile::as_auto_media()` picks the right Telegram media type from MIME (photo, video, audio, voice, animation, or document).
- `From<UploadedFile> for InputMedia` so you can pass an `UploadedFile` directly to `send_file`.
- `download_resumable` and `upload_resumable` under `features = ["experimental"]`. Interrupted transfers save a checkpoint JSON and a `.partial` bytes file to `<checkpoint_dir>/`. On the next call with the same media the partial bytes are restored, the download resumes from the saved byte offset (aligned to the nearest 1 MB boundary per Telegram's requirement), and all checkpoint files are deleted on success.
- `CheckpointStore::partial_path` - returns the path for the in-progress partial bytes file for a given download key.
- `download_streaming_on_dc_from` - internal method that starts `GetFile` requests from a given byte offset. Used by `download_resumable` to skip already-downloaded bytes.
- Unit tests in `ferogram/tests/resumable_transfers.rs` covering checkpoint roundtrip, delete, partial file I/O, key stability, SHA-256 correctness, TTL expiry detection, offset alignment math, and overlap-skip calculation. All tests run without a Telegram connection.
- Example `transfer_showcase.rs` demonstrating all 0.6.0 transfer APIs: progress, pause/resume/cancel, typed errors, auto media, and resumable transfers.

### Changed

- `upload_file_from_path` renamed to `upload_file`.
- `upload_file`, `upload`, `download`, `download_file` all take `handle: Option<&TransferHandle>` as a new last param. Pass `None` to keep old behavior.
- `send_file` now takes `impl Into<InputMedia>` instead of `InputMedia` directly. Existing callers unchanged.

### Fixed

- `download_resumable` now actually resumes. Previously the checkpoint offset was loaded and logged but never used; every call re-downloaded from byte 0. The fix passes `start_offset` to `download_streaming_on_dc_from`, which aligns it to the nearest 1 MB boundary and starts `GetFile` requests from there.
- `download_resumable` no longer computes or compares a partial SHA-256 hash. The old code saved `sha256_hex(dest)` on interruption (a hash of an incomplete buffer) then compared it against the final file. The hashes could never match. SHA-256 is now computed on the complete assembled file only and logged for auditing.
- Interrupted downloads now flush received bytes to a `.partial` file on disk so they survive a process restart. The file is deleted on successful completion.
## [0.5.2]: 2026-05-31

### Added

- **`ChannelKind` on `IncomingMessage`.** Three new async methods: `channel_kind()`,
  `is_megagroup()`, `is_broadcast()`, `is_gigagroup()`. The fast path reads from a
  per-batch `PeerMap` injected at dispatch time (no lock); the slow path falls back to
  the session peer cache.

- **`PeerMap` fast path for live updates.** `from_single_update_with_peers()` attaches
  the batch's chat list to every `IncomingMessage` produced in that batch, so
  `channel_kind()` never needs to acquire the `PeerCache` lock for the common case.

- **Session format v6.** Each channel peer entry now stores a `ChannelKind` byte
  (`0xFF` = absent for older entries). Fully backward-compatible: sessions from v2-v5
  load without changes, just without kind data until the peer is seen again.

- **SQLite and libSQL migration for `channel_kind`.** Both backends add the column if
  absent and persist/load kind alongside the existing peer fields.

- **`PeerCache::channel_kind_of(channel_id)`.** Direct kind lookup on the in-memory
  peer cache. Returns `None` for unknown or pre-v6 entries.

- **`build_peer_map(chats)` / `PeerMap` type** in `peer_cache`. Builds a cheap
  `Arc<HashMap>` from a batch's chat slice; shared across all messages in the batch.

### Fixed

- **`UpdateShortSentMessage` now boxed.** Reduced stack size of `EnvelopeResult` and
  `message_box` update paths by wrapping the type in `Box`.

- **`adaptor.rs`: flattened nested `if let` into `let ... && let ...`** (Rust 2024
  let-chains). Fixes an indentation issue that caused the reply-to reconstruction and
  synthesized `Message` block to be double-indented inside a dead scope.

- **`proxy.rs`: early-return refactor.** `strip_prefix` failure now uses `?` instead of
  an else branch, eliminating a nesting level.

- **`participants.rs`**: updated all `.copied()` calls on channel map entries to
  `.map(|&(hash, _)| hash)` following the `channels` value type change to `(i64,
  Option<ChannelKind>)`.

- **`builder_util.rs`**: use struct-update syntax for `PersistedSession` init instead of
  default-then-field-assign.

- **`ferogram-mtproto`: HTTP header byte literals** use `*b"..."` syntax instead of
  explicit `[u8; 4]` arrays.

---

## [0.5.1]: 2026-05-31

### Fixed

- **MessageBoxes: bot skipping every first message.** `force_update_entry` seeded the
  Common pts from the arriving update instead of the real server baseline, causing every
  odd-numbered incoming message to be dropped silently.

---

## [0.5.0]: 2026-05-16

API consolidation release. Paired functions that differed only by a single boolean condition have been merged into one. Download and upload paths were redesigned around `AsyncRead`/`AsyncWrite`. No new protocol or behavioural changes.

### Breaking changes

**Merged paired functions**

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
| `resolve_peer(str)` | `resolve(str)` |
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

// upload from path
let uploaded = client.upload_file("photo.jpg").await?;
```

`IncomingMessage` gains two convenience methods:

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

Five tiers keyed on file size (< 1 MB, 1-32 MB, 32-512 MB, 512 MB-1 GB, > 1 GB) replace the old two-tier heuristic.

---

## [0.4.1]: 2026-05-14

Patch release with one new API, configurable update buffering, session schema improvements, and 15 new examples.

No breaking changes from 0.4.0.

---

### Added

**`Client::quick_connect`**

Connects and authenticates in a single call. Handles the full auth flow from stdin: phone number or bot token, login code, 2FA password if needed. If the session is already authorized the prompt is skipped.

```rust
use ferogram::Client;

const API_ID: i32 = 12345;
const API_HASH: &str = "your_api_hash";

let (client, _shutdown) = Client::quick_connect("bot.session", API_ID, API_HASH).await?;
```

Bot tokens are detected automatically by their `<digits>:<string>` format, so the same prompt works for both bots and users.

For advanced options (proxy, PFS, custom transport, catch-up) use `Client::builder()` instead.

**`UpdateConfig` / `OverflowStrategy`**

Two new types for controlling the user-facing update dispatch buffer. Internal MTProto state (pts, qts, getDifference) is unaffected.

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

`DropOldest` (default) evicts ephemeral updates (typing, online status) first, then the oldest normal update. The incoming update is always buffered. `DropNewest` discards the incoming update instead. Default capacity is 2048.

**`ClientBuilder::low_memory_mode`**

Drops the dispatch buffer to 256 slots with `DropOldest` eviction. Good for Termux or any host where RAM is tight.

```rust
Client::builder()
    .api_id(API_ID)
    .api_hash(API_HASH)
    .session("bot.session")
    .low_memory_mode(true)
    .connect().await?;
```

**`ParticipantStatus`** is now re-exported from the crate root.

**`User::bot_guestchat()`** returns `true` if the bot supports guest-chat mode (`updateBotGuestChatQuery`).

**`GuestChatQuery::via_from()`** returns the original requester peer when Telegram includes `guestchat_via_from` in the message. Present when the bot is acting as an intermediary.

**Examples**

15 new examples under `ferogram/examples/`:

Userbot tools: `admin_log`, `chat_history`, `dialogs_list`, `download_media`, `get_participants`, `schedule_message`, `search_messages`, `serverless_userbot`, `string_session_gen`.

Bots: `echo_bot`, `filters_showcase`, `hello_self`, `inline_keyboard`, `inline_query_bot`, `poll_bot`, `translate_bot`.

**Docs**

New page: `docs/src/api/quick-connect.md` covering `quick_connect` usage and error handling.

---

### Changed

**Session schema** - two schema additions for existing databases, applied automatically via `migrate_legacy_sqlite_schema` on open. Safe on fresh databases; both operations are no-ops if the schema is already current.

- `peers` table gains an `is_chat` column for basic group tracking.
- New `min_peers` table stores min-user message contexts (`user_id`, `peer_id`, `msg_id`).

**`allow_missing_channel_hash`** in `ExperimentalFeatures` is now active. When set, a missing access hash during `getChannelDifference` triggers a `channels.getChannels` call with `access_hash = 0` to fetch and cache the hash, then retries the diff in the same loop iteration. Bots only.

**Periodic session snapshot saver** - the client now flushes the full session (peers, channel_pts, min_peers, DC auth/salt data) to the session backend every 60 seconds when the peer cache has been mutated. A final save runs unconditionally on shutdown. Previously peers were only flushed on explicit `save_session()` calls.

---

### Fixed

**Salt `valid_until` overflow** - `valid_until` was stored as `i32`. Telegram sends validity windows extending past 2038 (e.g. `valid_until = 2_751_656_413`, year 2057). Those values overflow `i32` and wrap negative, making every salt look expired on a signed comparison. Changed to `u32`.

**`GuestChatAnswer::send` return type** - was returning `bool`. Now returns `InputBotInlineMessageID` matching the corrected TL schema. The `setBotGuestChatResult` constructor ID was also wrong; both are fixed.

---

## [0.4.0]: 2026-05-08

0.4.0 is the first production-ready release of ferogram. It ships Layer 225 support. All users are advised to upgrade to 0.4.0 (or 0.4.x+) as the most recommended and supported version.

If you run into any bugs, please open an issue on GitHub or reach us at [@FerogramChat](https://t.me/FerogramChat). Thank you for using ferogram!

For the latest git revision: https://github.com/ankit-chaubey/ferogram

> **Note:** 0.3.9 was a broken publish. The workspace internal deps were not bumped so crates.io resolved `ferogram-tl-types` to the old Layer 224 build. 0.4.0 fixes that and is the correct release to use.

---

## [0.3.9]: 2026-05-07

Updated to TL Layer 225.

### Breaking changes in 0.3.9

- **`send_poll` signature changed.** Now takes a `PollBuilder` instead of individual positional args:

  ```rust
  // before
  client.send_poll(peer, "Best language?", &["Rust", "Go"], false, None, false).await?;

  // now
  use ferogram::PollBuilder;
  client.send_poll(peer, PollBuilder::new("Best language?").answers(["Rust", "Go"])).await?;
  ```

- **`parse_markdown` now implements MarkdownV2** (was a hybrid V1/V2 dialect):
  - `__text__` produces **Underline** now, not Italic
  - `~text~` is Strikethrough (single tilde, as Telegram specifies)
  - `> line` at line start produces Blockquote
  - `**> line` at line start produces Expandable blockquote
  - `![](tg://emoji?id=N)` with empty label now parses correctly
  - Escape set is the strict V2 one: `_ * [ ] ( ) ~ \ \` > # + - = | { } . !`
- **`generate_markdown` now emits V2 syntax:**
  - Italic: `_text_`
  - Underline: `__text__`
  - Strike: `~text~` (single tilde)
  - Blockquote: `> ` prefix; expandable: `**> ` prefix
  - All V2 special chars in plain text are backslash-escaped

### Added in 0.3.9

**`PollBuilder`** is a new fluent builder for `send_poll`. It covers the full `InputMediaPoll` field set:

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

New fields not in the old API: `public_voters`, `shuffle_answers`, `hide_results_until_close`, `close_period`, `close_date`, `solution`, `subscribers_only`, `countries_iso2`.

**`Update::GuestChatQuery`** is a new update variant for bots (`updateBotGuestChatQuery`). It fires when a user invites the bot into a guest-chat context. Carries `query_id`, `message`, `reference_messages`, and `qts`. `GuestChatQuery` derefs to `IncomingMessage`. Answer with `GuestChatAnswer`:

```rust
if let Update::GuestChatQuery(q) = update {
    q.answer()
        .article("Result title")
        .text("Answer body")
        .send(&client)
        .await?;
}
```

`GuestChatAnswer` supports all inline result kinds: `article`, `photo`, `document`, `game`, `location`, `venue`, `contact`, `webpage`, `invoice`, `raw`. Sends via `messages.setBotGuestChatResult`.

**`Client::delete_reaction(peer, msg_id, participant)`** reports and removes a specific user's reaction on a message (`messages.reportReaction`). Returns `true` on success.

**`Client::get_poll_stats(peer, msg_id)`** returns detailed vote stats for a poll (`stats.getPollStats`). Returns `tl::types::stats::PollStats`.

**`BannedRights::send_reactions`** is a new field on the ban-rights builder. Controls whether restricted users can add reactions:

```rust
BannedRights::default().send_reactions(false)
```

**User ID in builtins** is now resolved for `CallbackQuery`, `InlineQuery`, `InlineSend`, and `GuestChatQuery`, not just message updates.

**Parser additions (`ferogram-parsers`):**

- `parse_markdown_v2()` explicit V2 entry point (same as `parse_markdown`)
- `generate_markdown_v2()` explicit V2 generator (same as `generate_markdown`)
- `parse_markdown_v1()` legacy V1 parser, kept for backward compat, now deprecated
- HTML: `<ins>` accepted as underline alias (same as `<u>`)
- HTML: `<span class="tg-spoiler">` accepted as spoiler alias
- HTML: `<blockquote>` produces `MessageEntityBlockquote { collapsed: false }`
- HTML: `<blockquote expandable>` produces `MessageEntityBlockquote { collapsed: true }`
- HTML: `<tg-time unix="N" format="F">` produces `MessageEntityFormattedDate`
- HTML: `<pre><code class="language-X">` now correctly produces one `Pre` entity with the language set, not two separate entities
- `generate_html` now emits `<blockquote>`, `<blockquote expandable>`, and `<tg-time>`
- Both `parse_html` backends (hand-rolled and `html5ever`) are at full parity

### Deprecated in 0.3.9

- `parse_markdown_v1` marked `#[deprecated(since = "0.3.9")]`. Will be removed in 0.4.0.

## [0.3.8]: 2026-05-06

Bug fixes

### Changes in 0.3.8

- `send_to_self(msg)` is fixed: it now actually sends to Saved Messages again. In 0.3.7 it was pointing at the wrong function body.
- `open_mini_app(peer, MiniApp)` is now public. It was accidentally left private in 0.3.7.
- `get_chat_full(peer)` is now `pub` instead of `pub(crate)`, so you can call it directly from outside the crate.
- also fixed `download_media_to_file_on_dc` & `set_default_banned_rights_raw`

---

## [0.3.7]: 2026-05-05

The big story this release is workspace restructuring. Three crates were extracted out of the monolith, the connection stack got its own proper home, and a handful of API rough edges were smoothed out.

### New crates

- **ferogram-connect**: the raw TCP/transport layer is now its own publishable crate. It owns the connection, MTProto framing, transport selection (Intermediate, Obfuscated, FakeTLS), SOCKS5, and proxy handling. Previously this code lived inside a throwaway demo binary. If you're doing anything low-level with connections, this is where to look.

- **ferogram-fsm**: FSM state management is now its own crate. Same `FsmState`, `StateContext`, `StateStorage`, and `MemoryStorage` you already use, just extracted so it can be versioned and published independently.

- **ferogram-mtsender**: the MTProto sender pool and retry policy is now its own crate too. `RetryPolicy`, `AutoSleep`, `CircuitBreaker`, `NoRetries` all live here now. The main `ferogram` crate re-exports everything so nothing breaks.

The old `ferogram-app` and `ferogram-bot` standalone example binaries are gone. They've been replaced by proper examples under `ferogram/examples/` (`order_bot`, `showcase_bot`, `userbot`).

### New: `PeerExt` and `OptionPeerExt`

Getting a numeric ID out of a `tl::enums::Peer` used to mean writing a match every time:

```rust
let id = match peer {
    tl::enums::Peer::User(u)    => u.user_id,
    tl::enums::Peer::Chat(c)    => c.chat_id,
    tl::enums::Peer::Channel(c) => c.channel_id,
};
```

Now there's `.bare_id()`:

```rust
use ferogram::{PeerExt, OptionPeerExt};

let id     = peer.bare_id();
let sender = msg.sender_id().bare_id(); // Option<i64>
let chat   = msg.peer_id().bare_id();   // Option<i64>
```

`bare_id` returns the **native** Telegram ID, not the Bot-API-encoded one. A channel with native ID `1234567890` is `-1001234567890` in the Bot API.

### New: `PeerCache` and `ExperimentalFeatures`

`PeerCache` is now its own file (`peer_cache.rs`) and fully public. It's what backs every peer lookup under the hood: users, channels, basic groups, min-users, username index, phone index.

`ExperimentalFeatures` lets you opt into behaviours that deviate from strict spec:

```rust
Client::builder()
    .experimental_features(ExperimentalFeatures {
        allow_zero_hash: true, // bots only
        ..Default::default()
    })
    .connect().await?;
```

`allow_zero_hash` is the main one: bots can skip needing a cached access hash. Don't enable it on user accounts.

### Breaking changes

**`download_media_to_file` → `download_file`**

```rust
// before
client.download_media_to_file(location, &path).await?;

// now
client.download_file(location, &path).await?;
```

**`forward_messages` now requires `ForwardOptions`**

```rust
// before (3 args)
client.forward_messages(dest, &[id], src).await?;

// now (4 args)
client.forward_messages(dest, &[id], src, ForwardOptions::default()).await?;
```

**`respond_ex` removed**

`respond` already accepts `InputMessage` directly, so `respond_ex` was redundant:

```rust
// before
msg.respond_ex(InputMessage::html("<b>hi</b>")).await?;

// now
msg.respond(InputMessage::html("<b>hi</b>")).await?;
```

### Internals

The `ferogram/src/lib.rs` monolith has been split up. `client/` is now a proper module directory, `filters` and `middleware` are module directories instead of single files, and `peer_cache` is its own file. No public API changes; just much easier to navigate.

**Full Changelog**: https://github.com/ankit-chaubey/ferogram/compare/v0.3.6...v0.3.7

---

## [0.3.6]: 2026-04-30

### API Stabilization (Towards v0.4.0)

This release focuses on giving the high-level APIs their final shape before v0.4.0.

Some APIs have been simplified, merged, or removed where redundant. This may require a one-time migration, but it means a cleaner experience and no more overlapping methods going forward.

Once stabilized, future updates will focus on new features and improvements without disruptive API changes.

See [FEATURES.md](FEATURES.md) for the full list of what is currently public and supported.

---

## [0.3.5]: 2026-04-30

### Fixed

- **PollResults deserialization** - `PollResults` was being treated as a bare
  type (`crate::types::PollResults`) instead of a boxed type
  (`crate::enums::PollResults`). The deserializer skipped reading the 4-byte
  constructor ID, consuming it as the `flags` field instead. This misaligned
  all subsequent reads inside `getChannelDifference` and `getDifference`
  responses that contained a poll message, producing "unexpected constructor id"
  errors and dropping those updates entirely. Fixed by removing the
  special-case whitelist in `namegen.rs` so `PollResults` routes through
  `crate::enums::` like every other boxed type.

- **getDifference self-deadlock** - The `reader_loop` select arm that fires the
  MessageBoxes deadline was directly awaiting `run_pending_differences()`.
  `run_pending_differences` sends a getDifference RPC and then awaits the
  response frame. But `reader_loop` is the only task reading TCP frames and
  routing RPC responses - so the response could never arrive, producing a
  30-second hang after the first gap detection. The fix: spawn a separate
  task (same pattern already used by the Keepalive arm). A new
  `diff_in_flight: AtomicBool` guard prevents redundant concurrent spawns
  while a diff is already running.

### Removed

- **`prefetch_channel_access_hashes` from startup** - The automatic call to
  `messages.getDialogs` during startup and catch-up has been removed. This was
  the root cause of breakage on Telegram beta layers: the call forced full
  deserialization of `Dialog / DraftMessage / PollResults / PeerNotifySettings
  / Story`, all of which are high-churn objects that change without a layer
  bump. Removing it makes Ferogram resilient to Telegram schema drift.

### Changed

- **Lazy access_hash resolution** - Channel access hashes are now resolved
  purely lazily:
  1. Hashes for channels already seen in a previous session are restored from
     the persisted `peers` list in the session file.
  2. New channels receive their hash from the entities embedded in incoming
     updates / `getDifference` / `getChannelDifference` responses.
  3. If `getChannelDifference` is triggered for a channel whose hash is still
     unknown, `run_pending_differences` skips it gracefully
     (`end_channel_difference Banned`) and continues; the hash arrives with the
     next update entity.

### Added

- **`Client::warm_peer_cache_from_dialogs()`** - The former internal
  `prefetch_channel_access_hashes` function is now a public, opt-in method.
  Call it explicitly if you need a channel's access hash before any update has
  arrived. Do not call it at startup.

---

## [0.3.4]: 2026-04-28

### Added

- **PFS (Perfect Forward Secrecy)**: `use_pfs` flag on `ClientConfig`. When enabled, the DC pool performs a temp-key DH bind after auth-key negotiation. The session uses a short-lived encrypted session key; the permanent auth key is never exposed in plaintext on the wire. Falls back gracefully if the bind fails.
- **`prefetch_channel_access_hashes`**: called automatically at startup and after catch-up. Runs a single `GetDialogs` to pre-populate channel and user access hashes before the first update arrives, preventing `CHANNEL_INVALID` errors on reconnect.
- **`Deserializable::from_bytes_exact`**: convenience method on all TL types. Constructs a cursor, deserializes, and returns an error if bytes are left over. Replaces the repeated `Cursor::from_slice` + `deserialize` pattern throughout the codebase.
- **`auth_key_id_from_key`** utility in `ferogram-mtproto`.
- **`ferogram::util`** module with `decode_checked` helper used by the pts layer.

### Fixed

- `get_difference` no longer hangs when two tasks race to call it concurrently. The second caller now polls every 50 ms and gives up after 35 s with a warning, then lets the next gap tick retry.
- Parse errors on incoming `Updates` frames are now logged as warnings instead of being silently dropped.

### Changed

- All raw `Cursor::from_slice` + `deserialize` call sites in `lib.rs`, `dc_pool.rs`, and `pts.rs` migrated to `from_bytes_exact`.
- `step2_temp` re-exported from `ferogram-mtproto` auth module.

**Full Changelog**: https://github.com/ankit-chaubey/ferogram/compare/v0.3.3...v0.3.4

---

## [0.3.3]: 2026-04-22

### New crate

- **ferogram-derive**: proc-macro crate providing `#[derive(FsmState)]`. Generates `as_key` and `from_key` for unit-variant enums. Only active with the `derive` feature flag.

### New modules

- **ferogram::middleware**: `Middleware` trait, `Next` chain, `DispatchError`/`DispatchResult`, and a rate-limit middleware backed by `DashMap`. Wraps handler dispatch with pre/post logic.
- **ferogram::filters**: composable, synchronous predicates over `IncomingMessage`. Built-in constructors for command, private, text, media, and more. Supports `&`, `|`, `!` operators for compound expressions. Integrates with the FSM via `StateContext`.
- **ferogram::fsm**: `FsmState` trait, `StateContext`, `StateKey`, `StateKeyStrategy`, and `StateStorage`. In-memory `DashMap`-backed store by default; custom backends via async-trait extension point.
- **ferogram::conversation**: `Conversation` for stateful back-and-forth with a single peer. Wraps `UpdateStream` for the conversation lifetime and buffers updates from other peers.

### Added

- `IncomingMessage` gained message-inspection helpers: `chat_id()`, `is_private()`, `is_group()`, `is_channel()`, `is_any_group()`, `from_id()`, `is_bot_command()`, `command()`, `is_command_named()`, `command_args()`, `has_media()`, `has_photo()`, `has_document()`, `is_forwarded()`, `is_reply()`, `album_id()`.
- New update types exported from the crate root: `ParticipantUpdate`, `JoinRequestUpdate`, `MessageReactionUpdate`, `PollVoteUpdate`, `BotStoppedUpdate`, `ShippingQueryUpdate`, `PreCheckoutQueryUpdate`, `ChatBoostUpdate`.
- `Client::get_chat_administrators()`: returns all admins and the creator for a channel or supergroup. For basic groups, returns all participants (use `is_admin` on the result).
- `FsmState` re-exported from the crate root. With the `derive` feature, `#[derive(FsmState)]` is available directly from `ferogram`.
- Example: `examples/order_bot.rs` showing a multi-step FSM-driven order flow.

### Docs

New pages:

- Bot Framework: Middleware & Dispatcher, Finite State Machine (FSM), Conversation API
- API reference: Bot Configuration, Stats & Analytics

**Full Changelog**: https://github.com/ankit-chaubey/ferogram/compare/v0.3.2...v0.3.3

---

## [0.3.2]: 2026-04-21

### Changed

- `SeenMsgIds` deque now paired with a `HashSet` for O(1) duplicate checks under concurrent workers (was O(n) linear scan).
- Session temp files now use a unique name per write. A `write_lock` serializes concurrent saves to prevent a rename race on Windows.

### Fixed

- `PaddedIntermediate` handshake was missing in DC pool worker connections. Fixed.
- `new_session_created` was resetting the session on fresh connections when it should not, causing a session ID mismatch on every decrypt after. Fixed.
- `scan_body` was passing `None` as `sent_msg_id` on container iterations, letting stale results overwrite live responses. Fixed.
- `importAuthorization` branch logic was inverted; it was skipping the import exactly when it was needed. Fixed.
- Server 4-byte transport error codes during DH now surface properly instead of logging as "plain frame too short".

**Full Changelog**: https://github.com/ankit-chaubey/ferogram/compare/v0.3.1...v0.3.2

---

## [0.3.1]: 2026-04-20

Patch release to fix the docs.rs build. No functional changes from 0.3.0.

**Full Changelog**: https://github.com/ankit-chaubey/ferogram/compare/v0.3.0...v0.3.1

---

## [0.3.0]: 2026-04-19

0.3.0 is a substantial release. The workspace grew by two new crates, the session and parser layers were extracted into their own packages, and the connection stack gained CDN support, DNS-over-HTTPS fallback, and transport probing. About 16.7k lines were added across 114 changed files.

### New crates

- **ferogram-session**: session persistence is now its own crate. It owns `PersistedSession`, `DcEntry`, `DcFlags`, `UpdatesStateSnap`, `CachedPeer`, `CachedMinPeer`, `default_dc_addresses`, and all storage backends (`BinaryFileBackend`, `InMemoryBackend`, `StringSessionBackend`, `SqliteBackend`, `LibSqlBackend`). The main `ferogram` crate re-exports everything from it so existing code is unaffected.
- **ferogram-parsers**: Telegram Markdown and HTML entity parsing is now its own crate. It provides `parse_markdown`, `generate_markdown`, `parse_html`, and `generate_html`. An optional `html5ever` feature enables spec-compliant HTML5 tokenization. The main `ferogram::parsers` module re-exports these.

### Session changes

- Binary session format bumped to **v5**.
- Session now stores home DC, full DC table, update state (pts/qts/date/seq), per-channel pts, peer cache, and min-user message contexts.
- Legacy formats load without error.
- Saves are atomic: written to a `.tmp` file first, then renamed into place.
- DC flags are now persisted so media and CDN DC entries survive restarts.

### Client and builder

- `ClientBuilder` gained three new options:
  - `.probe_transport(true)`: races Obfuscated, Abridged, and HTTP transports and picks the first to succeed. Has no effect when using MTProxy.
  - `.resilient_connect(true)`: if direct TCP fails, falls back through DNS-over-HTTPS and then Telegram's Firebase/Google special-config.
  - `.experimental_features(...)`: takes an `ExperimentalFeatures` struct.
- New `ExperimentalFeatures` struct with fields: `allow_zero_hash`, `allow_missing_channel_hash`, `auto_resolve_peers` (reserved, not yet active).

### New modules

- `ferogram::cdn_download`: full CDN file download path. Handles `upload.getCdnFile`, `upload.reuploadCdnFile`, AES-256-CTR chunk decryption, and reassembly. Exports `CdnDownloader`, `CdnChunkResult`, and `CDN_CHUNK_SIZE`.
- `ferogram::dns_resolver`: DNS-over-HTTPS with TTL caching. Queries Google DoH and Mozilla/Cloudflare DoH, merges IPv4 and IPv6 answers.
- `ferogram::special_config`: Telegram's Firebase/Google fallback for DC configuration. Decodes the encrypted `help.configSimple` response and extracts DC options.

### MTProto internals

- `ferogram-mtproto` gained a new `bind_temp_key` module.
- Now re-exports `encrypt_bind_inner`, `gen_msg_id`, `serialize_bind_temp_auth_key`, `EncryptedSession`, `SeenMsgIds`, and `new_seen_msg_ids`.

### Docs

New pages added:

- Advanced: CDN Downloads, Transport Probing and Resilient Connect, Connection Restart Policy, Experimental Features
- API reference: ClientBuilder, Types Reference, Chat Management, Contacts, Forum Topics, Games, Invite Links, Polls, Privacy, Profile, Stickers

---

## [0.2.0]: 2026-04-13

### Changed

- Peer cache moved from `RwLock<HashMap>` to `moka` concurrent cache to eliminate lock contention during peer lookups.
- Pending RPC map replaced with `DashMap`, enabling lock-free response routing.
- `dc_pool` now uses `tokio::sync::Mutex` instead of `parking_lot::Mutex` to avoid blocking the async runtime.
- Fresh DH sessions now wait **2 seconds** after key derivation to allow Telegram to propagate the new auth key across DCs.
- Stale key detection simplified: only error `-404` now triggers key rotation.
- FakeTLS transport now prepends the **Change Cipher Spec** record to the first application data chunk to match Telegram’s expected TLS handshake pattern.
- `getDifference` deserialization now tolerates unknown server responses instead of failing and dropping buffered updates.
- Container message parsing now validates inner message alignment and safely discards malformed frames.
- Transport errors `-429` and `-444` are now logged clearly before reconnecting.

### Fixed

- `getDifference` deserialization no longer fails hard on unknown server responses; unknown variants are discarded and buffered updates are preserved.
- Container message parsing now validates inner message alignment and discards malformed frames instead of propagating a parse error.
- Transport errors `-429` and `-444` are now surfaced as log warnings before reconnecting rather than being swallowed silently.

## [0.1.0]: 2026-04-11

Renamed and rebranded from [layer](https://github.com/ankit-chaubey/layer) v0.5.0.

### Changed
- Project renamed from `layer` to `ferogram`
- All crate names updated (`layer-*` → `ferogram-*`)
- Repository moved to `github.com/ankit-chaubey/ferogram`

### Inherited from layer v0.5.0
- Full MTProto 2.0 implementation (DH handshake, AES-IGE, salt tracking, DC migration)
- MTProxy support (PaddedIntermediate, FakeTLS, SOCKS5)
- User + bot authentication with 2FA SRP
- Typed async update stream (NewMessage, MessageEdited, CallbackQuery, InlineQuery, ChatAction, UserStatus)
- PTS/seq/qts gap detection and recovery
- String, SQLite, and libsql session backends
- Auto-generated TL Layer 224 types (2,329 constructors)
