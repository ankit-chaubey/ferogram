# Changelog

All notable changes to this project will be documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.0.0/)
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

---

## [Unreleased]

### Fixed

- `ferogram-mtsender`: `MtpSender` now AES-CTR decrypts Obfuscated and
  PaddedIntermediate (`dd` MTProxy) bytes before Intermediate/Abridged peel,
  and strips PaddedIntermediate random padding before MTProto unpack. Without
  this, the first server frames were treated as plaintext length prefixes and
  failed with random `transport code -<n>` errors after a successful proxy
  handshake.
- `ferogram-mtsender`: `MtpSender` FakeTLS (`ee` MTProxy) receive now peels
  TLS Application Data records and CTR-decrypts only the payload, matching the
  encode path. Previously records were mis-parsed as Intermediate frames,
  causing `transport code`, hangs, or unpack failures after a successful proxy
  handshake.

### Added

- `ferogram-session`: binary session format bumped to v8, adding a per-peer
  `is_community` flag (`CachedPeer::is_community`) so Telegram Community
  entities survive a session restore instead of collapsing into a plain
  channel. Older session files load unchanged and upgrade automatically on
  the next save, no migration step or user action required. `SqliteBackend`
  and `LibSqlBackend` gain a matching `is_community` column with the same
  idempotent migration used for `channel_kind`.
- `PeerCache::communities`: Community chats are now cached by ID with their
  own access-hash map instead of being silently dropped by `cache_chat`'s
  catch-all. `PeerCache::community_input_peer` resolves a cached community
  to the `InputPeer::Channel` shape Telegram expects for it. Wired into the
  session save/restore loop in `Client` alongside `channels`/`chats`.
- `GetDialogsOptions`: Added configurable dialog retrieval options, allowing
  `exclude_pinned` and `folder_id` to be specified while keeping
  `Client::get_dialogs(limit)` fully backward compatible via `From<i32>`.
- `DialogCursor`: Added a serializable pagination cursor for dialogs,
  enabling iterator state to be saved and later resumed with
  `Client::iter_dialogs_from()`.
- `Client::stream_dialogs`: Added a `futures::Stream`-based dialog iterator
  (`DialogsStream`) for seamless integration with `StreamExt` and
  `TryStreamExt` combinators.

---

## [0.6.4] - 2026-07-14

TL layer bumped to 228 (Telegram Communities, ephemeral/one-time messages). Granular
channel-ban rights, tunable transfer concurrency, richer media downloading (profile
photos, thumbnails, quality selection), a callback/inline-query dispatcher, and a
pipelined MTProto sender for higher-throughput transfers.

### Added

- `BannedRightsBuilder`: fluent builder for granular channel ban rights (e.g. block file
  uploads but allow photos). `Client::ban`/`kick`/`restrict` are now thin wrappers around
  one shared `restrict` implementation built on it, instead of each hand-assembling its
  own `ChatBannedRights`.
- `TransferLimits` and `ClientBuilder::transfer_limits(...)` (plus the shorthands
  `download_tcp_connections`, `upload_tcp_connections`, `max_tcp_connections`,
  `download_pipeline_depth`, `upload_pipeline_depth`, `bypass_tcp_allotments`): tune how
  many parallel connections a single transfer may open and how deep each connection's
  request pipeline goes, instead of the previous fixed constants.
- `ferogram-mtsender::PipelinedSender` / `spawn_pipelined`: a background sender task that
  keeps multiple chunk requests in flight on one connection (the "X pieces in flight" half
  of Telegram's upload/download performance guidance), used internally by the new
  pipelined upload/download paths.
- `Downloadable` now covers far more than `Photo`/`Document`/`Sticker`:
  - `tl::enums::MessageMedia` itself, resolved recursively through games, web page
    previews, polls, stories, and paid-media wrappers.
  - `ProfilePhoto` (`Client`-independent wrapper over `UserProfilePhoto`/`ChatPhoto`, with
    a `.small()` variant), `PhotoThumb`/`DocumentThumb` (a specific thumbnail size via
    `Photo::thumb(type)` / `Document::thumb(type)`), and `RawLocation` for anything else.
  - `video_cover(media)`: extract a video's cover photo (`video_cover` field).
  - `MediaQuality` (`Original`/`Highest`/`Lowest`) and `available_qualities(media)`: pick
    among a video's `alt_documents` transcode variants by resolution.
- `Client::request_login_code_with_options`, `Client::resend_code`, `SendCodeOptions`
  (flash call / missed call / Firebase delivery, logout tokens), and
  `ClientBuilder::future_auth_token(...)`: `sign_out()` now captures the
  `future_auth_token` Telegram returns on logout and replays it automatically on the next
  `request_login_code`, letting a re-login skip code entry when Telegram still recognizes
  the device (surfaced via the new `SendCodeOutcome::AlreadyAuthorized` variant).
- `Client::interactive_sign_in`: the stdin-prompt sign-in flow used by `quick_connect`,
  now exposed as a reusable public method.
- `dispatcher` module: routes callback queries and inline queries alongside the existing
  message dispatcher, via `Router::on_callback_query`/`on_inline_query`/`on_inline_send`/
  `on_callback_query_fsm`. `Filter`/`BoxFilter` are now generic over the update type
  (`Filter<T>`, default `T = IncomingMessage`) so the same `&`/`|`/`!` combinators work
  over `CallbackQuery` and `InlineQuery` too.
- `Client::get_chat_photos(peer, limit)`: photo/avatar history for groups and channels.
  Current photo comes from the chat's full info (survives message deletion); older photos
  come from `messageActionChatEditPhoto` search history.
- `Client::cache_entities(users, chats)`: public entry point to feed `users`/`chats` from
  a hand-issued raw RPC call into the peer cache, so custom calls not yet covered by the
  client's API surface can still populate access hashes correctly.
- `ClientBuilder::dc_id_override(dc_id)`: register a fresh connection under a custom
  `dc_id`, paired with `.dc_addr(...)`, instead of the hardcoded DC2 default.
- `PeerCache::stats()` / `PeerCacheStats` and `PeerCache::clear_min_contexts()` for
  inspecting and pruning cache size; `ExperimentalFeatures::cache_min_peers` (default
  `false`) gates whether min-user message contexts are stored at all.
- `PersistedSession::stats()` / `SessionStats` for inspecting a session's size and
  contents (DC count, peer counts, approximate byte size, ...).
- `UserFull` and `MessagePage` typed wrappers: `UserFull` pairs `users.getFull`'s bare
  response with the account's name/username/status from the same reply; `MessagePage`
  flattens the four `messages.Messages` response shapes into one with `has_more()` /
  `next_offset()`.
- `PhotoSize`-aware `Downloadable::size()` on `Photo`, so known-size photos can use the
  concurrent download path instead of always falling back to the sequential unknown-size
  one.
- TL layer 228: Telegram Communities (`communities.*` methods, `Chat::Community`,
  `ChatFull::CommunityFull`, `Dialog::Community`) and ephemeral/one-time messages
  (`ephemeral.*` methods, `EphemeralMessage`, `InputReplyTo::EphemeralMessage`), plus
  related `AiCompose`, admin-rights (`manage_linked_peers`), and rich-message additions.

### Changed

- `Client::request_login_code` now returns `SendCodeOutcome` instead of `LoginToken`
  directly (see Breaking Changes).
- `download_chunk_size`/`upload_part_size` tables simplified: downloads cap at 512 KB
  (previously stepping up to 1 MB past 500 MB) and uploads use a 3-tier table instead of
  5, matching the pipelined transfer engine's chunk-size assumptions.
- `download_worker_count`/`upload_worker_count` now take a `max_workers` ceiling supplied
  by `TransferLimits` and clamp their tiered result to it, instead of always resolving up
  to the previous hard-coded `MAX_WORKERS_PER_FILE` (4).
- `Client::download_file` (and the underlying `DownloadFile` builder) are generic over any
  `D: Downloadable`, not just the built-in media types.
- `order_router()` / `Router::on_callback_query_fsm` etc. now take a `StateStorage` handle
  directly rather than relying on ambient state.
- Session backend I/O (`save_session`, periodic snapshot flush, update-state persistence)
  now runs on `spawn_blocking`, so SQLite/file writes no longer stall async tasks when
  multiple clients share a runtime.
- `order_bot` example: bootstraps its initial FSM state correctly.

### Fixed

- MTProto handshake: stopped sending the dc-tagged `p_q_inner_data` constructor on normal
  connects. Telegram rejects it with RPC 444 outside of DC2; it's now only used for flows
  that actually need dc verification (`step2_dc_tagged`).
- Reconnect: keep the cached auth key on a timeout and drop stale in-flight RPCs before
  re-initializing, instead of discarding a still-valid key. Also clear the cached key and
  retry with fresh DH when the server rejects it outright (RPC 401,
  `AUTH_KEY_UNREGISTERED`) rather than looping on a dead key.
- `Client::get_chat_participants`: return a clear error for `ChatFull::CommunityFull`
  instead of failing deserialization, since communities have no basic-chat-style
  participant list.
- `#[derive(FsmState)]`: corrected match-arm codegen and crate path resolution.

### Breaking Changes

- `Client::request_login_code` and `Client::resend_code` return `Result<SendCodeOutcome, InvocationError>`
  instead of `Result<LoginToken, InvocationError>`. Match on `SendCodeOutcome::CodeRequired(token)`
  to get the previous `LoginToken`.
- `media::download_worker_count`/`media::upload_worker_count` take an additional
  `max_workers: usize` argument.
- `Client::kick`'s channel path no longer takes a snapshot of the user's access hash
  before dispatching; it now delegates to `restrict`, which resolves it internally. No
  signature change, but custom callers relying on the old private `ban_participant_raw`
  helper (removed) will need to switch to `Client::ban`/`restrict`.

---

## [0.6.3] - 2026-06-28

Rich messaging, new client modules, separated upload/download APIs, and a broad documentation pass across the entire codebase.

### Added

**Rich messages**

- `InputMessage::rich_text(blocks)` attaches structured `PageBlock` content to any message. Rich messages render as full documents inside Telegram (headings, tables, code blocks, collapsible sections, math) rather than flat text.
- `parse_rich_markdown(text)` in `ferogram-parsers` converts a Markdown string into `Vec<PageBlock>`. Supports ATX headings, fenced code blocks with language tag, GFM tables, ordered and unordered lists, `<details>/<summary>` collapsible sections, horizontal rules, LaTeX math (`$...$` inline, `$$...$$` block), and standard inline formatting (bold, italic, strikethrough, spoiler, inline code).
- `parse_rich_html(html)` does the same from an HTML source. Both parsers live in `ferogram-parsers` alongside the existing flat-text parsers and are re-exported from `ferogram::parsers`.
- `rich_common` internal module shared by both parsers (block construction helpers, inline-text rendering, `RichText` builder).
- `rich_message.rs` example: sends a structured article with headings, a table, a code block, a collapsible section, and a LaTeX math block to a target chat.
- New doc page: `docs/src/messaging/rich-messages.md` with the full syntax reference and a worked example.

**New client modules**

Eight new files under `ferogram/src/client/`, each covering a previously unaddressed API surface:

- `reactions.rs`: `get_reactions`, `delete_reaction`, `iter_reaction_users`, `send_paid_reaction`.
- `forum.rs`: `get_forum_topics`, `get_forum_topics_by_id`, `create_forum_topic`, `edit_forum_topic`, `delete_forum_topic_history`, `toggle_forum`.
- `invites.rs`: `export_invite_link`, `revoke_invite_link`, `edit_invite_link`, `get_invite_links`, `delete_invite_link`, `delete_revoked_invite_links`, `join_request`, `all_join_requests`, `get_invite_link_members`, `get_admins_with_invites`.
- `payments.rs`: `answer_precheckout_query`, `answer_shipping_query`, `send_invoice`.
- `polls.rs`: `send_poll`, `send_vote`, `poll_results`, `get_poll_votes`.
- `privacy.rs`: privacy rule getters and setters.
- `resolve.rs`: `resolve`, `join_link`, `check_invite` extracted from the main module into their own file.
- `stickers.rs`: `get_sticker_set`, `toggle_stickers`, `get_all_stickers`, `get_custom_emoji_documents`.

**Upload/download API separation**

- `DownloadFile<'a>` builder type returned by `Client::download_file`. Supports an optional `.handle(&TransferHandle)` for progress tracking before `.await`.
- `Upload<'a, R>` builder type returned by `Client::upload`. Same `.handle()` chaining pattern.
- `UploadFile<'a>` builder type returned by `Client::upload_file`. Mirrors the above.
- All three types implement `IntoFuture`, so existing `.await` callsites work without change. The builders are a non-breaking extension: calling `.handle(h).await` is the opt-in path.
- `html.rs` and `markdown.rs` added as named entry points in `ferogram-parsers` alongside the existing `lib.rs` re-exports.

### Changed

- `client/mod.rs` is substantially leaner. Logic for reactions, forum, invites, payments, polls, privacy, resolve, and stickers has moved into the dedicated files listed above.
- `ferogram-parsers` reorganized: `rich_common`, `rich_html`, `rich_markdown` sit next to the flat-text parsers rather than in a separate sub-crate.
- Doc comments across the public API rewritten for clarity and accuracy. Covers `InputMessage`, `Client`, `transfer`, `media`, `parsers`, and several builder types.
- `docs/src/messaging/media.md`, `docs/src/api/client.md`, `docs/src/introduction.md`, `docs/src/installation.md`, `docs/src/features.md`, and `docs/src/whats-new.md` updated.
- `FEATURES.md` and `README.md` updated to reflect the 0.6.3 additions.
- Several examples cleaned up: `chat_history.rs`, `order_bot.rs`, `progress_transfer.rs`, `transfer_showcase.rs` updated for current API.

### Internal

- `ferogram-parsers/src/lib.rs` re-exports all four public parsers (`parse_markdown`, `parse_html`, `parse_rich_markdown`, `parse_rich_html`) from a single location.
- `IntoFuture` impls for the three upload/download builders avoid breaking existing `.await` callsites while enabling optional progress wiring.
- General cleanup and dead-code removal across `ferogram-mtsender`, `ferogram-crypto`, `ferogram-session`, `ferogram-tl-gen`, and `ferogram-tl-parser`.

---

## [0.6.2] - 2026-06-24

Transport, reconnect, and update synchronization stabilization release.

### Added

- `add_offset` on `Client::get_message_history` and `Client::get_replies` for offset-based pagination.
- Connection generation, reader lifecycle, and session identity tracing (`[conn_gen=X]`, `[reader#N]`, `[sid=...]`).
- Frame-level transport diagnostics for sequence tracking, CRC validation, and transport debugging.

### Changed

- `Client::get_message_history` and `Client::get_replies` now accept an additional `add_offset: i32` parameter. Pass `0` to preserve previous behavior.
- Refactored ownership boundaries across `ferogram`, `ferogram-connect`, `ferogram-mtsender`, `ferogram-session`, and `ferogram-parsers`.
- Reduced coupling between transport, session, connection, and RPC dispatch layers.
- Improved transport initialization, fallback selection, and reconnect flow.
- Merged transport and reconnect improvements previously tracked in the unreleased branch.

### Fixed

- Fixed multiple frame parsing and validation issues in Full transport mode.
- Fixed transport synchronization issues that could result in CRC mismatches.
- Fixed reconnect handling after network interruptions and auth-key reuse.
- Fixed transport recovery paths under unstable network conditions.
- Fixed update gap recovery and difference scheduling.
- Fixed several edge cases that could leave clients disconnected after connectivity was restored.
- Reduced reconnect loops caused by transient transport failures.
- Improved account and channel difference synchronization.

### Internal

- Refactored networking and connection management internals.
- Simplified transport recovery code paths.
- Reduced unnecessary reconnect attempts during outage scenarios.
- General reliability improvements, cleanup, and maintenance updates.

## [0.6.0] - 2026-06-01

### Added

- `TransferHandle` and `TransferProgress` in `ferogram::transfer`. Pass a handle to any upload or download to track progress, pause, resume, or cancel. Both re-exported from the crate root.
- `InvocationErrorExt` trait with `.kind()` and `.friendly()` on `InvocationError`. Returns an `ErrorKind` enum or a readable string.
- `UploadedFile::as_auto_media()` picks the right Telegram media type from MIME.
- `From<UploadedFile> for InputMedia` so you can pass an `UploadedFile` directly to `send_file`.
- `download_resumable` and `upload_resumable` under `features = ["experimental"]`. Interrupted transfers save a checkpoint JSON and a `.partial` file to `<checkpoint_dir>/`. Next call with the same media resumes from the saved offset, aligned to 1 MB per Telegram's requirement. Checkpoint files are deleted on success.
- `CheckpointStore::partial_path` returns the path for the in-progress partial file for a given download key.
- `download_streaming_on_dc_from` starts `GetFile` requests from a given byte offset. Used internally by `download_resumable`.
- Unit tests in `ferogram/tests/resumable_transfers.rs` covering checkpoint roundtrip, delete, partial file I/O, key stability, SHA-256 correctness, TTL expiry, offset alignment, and overlap-skip. All run without a Telegram connection.
- Example `transfer_showcase.rs` covering all 0.6.0 transfer APIs.

### Changed

- `upload_file_from_path` renamed to `upload_file`.
- `upload_file`, `upload`, `download`, `download_file` all take `handle: Option<&TransferHandle>` as a new last param. Pass `None` for old behavior.
- `send_file` now takes `impl Into<InputMedia>` instead of `InputMedia` directly. Existing callers unchanged.

### Fixed

- `download_resumable` now actually resumes. The checkpoint offset was previously loaded but never used, so every call re-downloaded from byte 0. Fixed by passing `start_offset` to `download_streaming_on_dc_from`.
- `download_resumable` no longer computes or compares a partial SHA-256. The old code hashed an incomplete buffer on interruption then compared it against the final file, so the hashes could never match. SHA-256 is now computed on the complete file only and logged for auditing.
- Interrupted downloads now flush received bytes to a `.partial` file so they survive a process restart. Deleted on success.

---

## [0.5.2] - 2026-05-31

### Added

- `ChannelKind` on `IncomingMessage`. Three new async methods: `channel_kind()`, `is_megagroup()`, `is_broadcast()`, `is_gigagroup()`. Fast path reads from a per-batch `PeerMap` at dispatch time with no lock. Falls back to the session peer cache.
- `PeerMap` fast path for live updates. `from_single_update_with_peers()` attaches the batch's chat list to every `IncomingMessage` in that batch, so `channel_kind()` never needs the `PeerCache` lock for the common case.
- Session format v6. Each channel peer entry now stores a `ChannelKind` byte. Fully backward-compatible: sessions from v2-v5 load fine, just without kind data until the peer is seen again.
- SQLite and libSQL migration for `channel_kind`. Both backends add the column if absent and persist/load kind alongside existing peer fields.
- `PeerCache::channel_kind_of(channel_id)`. Direct kind lookup on the in-memory peer cache. Returns `None` for unknown or pre-v6 entries.
- `build_peer_map(chats)` / `PeerMap` type in `peer_cache`. Builds a cheap `Arc<HashMap>` from a batch's chat slice, shared across all messages in the batch.

### Fixed

- `UpdateShortSentMessage` now boxed. Reduced stack size of `EnvelopeResult` and `message_box` update paths.
- `adaptor.rs`: flattened nested `if let` into `let ... && let ...` (Rust 2024 let-chains). Fixes a double-indent issue that put the reply-to reconstruction inside a dead scope.
- `proxy.rs`: `strip_prefix` failure now uses `?` instead of an else branch, removing a nesting level.
- `participants.rs`: updated `.copied()` calls on channel map entries to `.map(|&(hash, _)| hash)` after the `channels` value type changed to `(i64, Option<ChannelKind>)`.
- `builder_util.rs`: use struct-update syntax for `PersistedSession` init.
- `ferogram-mtproto`: HTTP header byte literals use `*b"..."` syntax instead of explicit `[u8; 4]` arrays.

---

## [0.5.1] - 2026-05-31

### Fixed

- MessageBoxes: bots were silently dropping every first message. `force_update_entry` was seeding the Common pts from the arriving update instead of the real server baseline, causing every odd-numbered message to be discarded.

---

## [0.5.0] - 2026-05-16

API consolidation release. Paired functions that differed only by a boolean have been merged. Download and upload paths were redesigned around `AsyncRead`/`AsyncWrite`. No protocol or behavioral changes.

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

## [0.4.1] - 2026-05-14

Patch release with one new API, configurable update buffering, session schema improvements, and 15 new examples.

No breaking changes from 0.4.0.

### Added

**`Client::quick_connect`**

Connects and authenticates in a single call. Handles the full auth flow from stdin: phone number or bot token, login code, 2FA password if needed. Skips the prompt if the session is already authorized.

```rust
use ferogram::Client;

const API_ID: i32 = 12345;
const API_HASH: &str = "your_api_hash";

let (client, _shutdown) = Client::quick_connect("bot.session", API_ID, API_HASH).await?;
```

Bot tokens are detected automatically by their `<digits>:<string>` format, so the same prompt works for both bots and users. For advanced options (proxy, PFS, custom transport, catch-up) use `Client::builder()` instead.

**`UpdateConfig` / `OverflowStrategy`**

Two new types for controlling the update dispatch buffer. Internal MTProto state (pts, qts, getDifference) is unaffected.

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

`DropOldest` (default) evicts ephemeral updates first (typing, online status), then the oldest normal update. `DropNewest` discards the incoming update instead. Default capacity is 2048.

**`ClientBuilder::low_memory_mode`**

Drops the dispatch buffer to 256 slots with `DropOldest` eviction. Good for Termux or any RAM-constrained host.

```rust
Client::builder()
    .api_id(API_ID)
    .api_hash(API_HASH)
    .session("bot.session")
    .low_memory_mode(true)
    .connect().await?;
```

**Other additions**

- `ParticipantStatus` re-exported from the crate root.
- `User::bot_guestchat()` returns `true` if the bot supports guest-chat mode.
- `GuestChatQuery::via_from()` returns the original requester peer when `guestchat_via_from` is present.

**Examples**

15 new examples under `ferogram/examples/`:

Userbot tools: `admin_log`, `chat_history`, `dialogs_list`, `download_media`, `get_participants`, `schedule_message`, `search_messages`, `serverless_userbot`, `string_session_gen`.

Bots: `echo_bot`, `filters_showcase`, `hello_self`, `inline_keyboard`, `inline_query_bot`, `poll_bot`, `translate_bot`.

**Docs**

New page: `docs/src/api/quick-connect.md`.

### Changed

**Session schema** - two additions applied automatically via `migrate_legacy_sqlite_schema` on open:

- `peers` table gains an `is_chat` column for basic group tracking.
- New `min_peers` table stores min-user message contexts.

**`allow_missing_channel_hash`** in `ExperimentalFeatures` is now active. A missing access hash during `getChannelDifference` triggers a `channels.getChannels` call with `access_hash = 0`, then retries the diff in the same loop iteration. Bots only.

**Periodic session snapshot saver** - the client now flushes the full session to the backend every 60 seconds when the peer cache has been mutated. A final save runs on shutdown. Previously peers were only flushed on explicit `save_session()` calls.

### Fixed

- `valid_until` was stored as `i32`. Telegram sends values past 2038 (e.g. year 2057) that overflow and go negative, making every salt look expired. Changed to `u32`.
- `GuestChatAnswer::send` return type fixed to `InputBotInlineMessageID`. The `setBotGuestChatResult` constructor ID was also wrong; both are fixed.

---

## [0.4.0] - 2026-05-08

0.4.0 is the first production-ready release of ferogram. Ships Layer 225 support. All users should upgrade to 0.4.0 or later.

If you run into any bugs, open an issue on GitHub or reach us at [@FerogramChat](https://t.me/FerogramChat).

For the latest git revision: https://github.com/ankit-chaubey/ferogram

> **Note:** 0.3.9 was a broken publish. Workspace internal deps were not bumped so crates.io resolved `ferogram-tl-types` to the old Layer 224 build. 0.4.0 fixes that.

---

## [0.3.9] - 2026-05-07

Updated to TL Layer 225.

### Breaking changes

- **`send_poll` signature changed.** Now takes a `PollBuilder`:

  ```rust
  // before
  client.send_poll(peer, "Best language?", &["Rust", "Go"], false, None, false).await?;

  // now
  use ferogram::PollBuilder;
  client.send_poll(peer, PollBuilder::new("Best language?").answers(["Rust", "Go"])).await?;
  ```

- **`parse_markdown` now implements MarkdownV2:**
  - `__text__` produces Underline (was Italic)
  - `~text~` is Strikethrough (single tilde)
  - `> line` at line start produces Blockquote
  - `**> line` at line start produces Expandable blockquote
  - `![](tg://emoji?id=N)` with empty label now parses correctly
  - Escape set is the strict V2 one

- **`generate_markdown` now emits V2 syntax:**
  - Italic: `_text_`
  - Underline: `__text__`
  - Strike: `~text~`
  - Blockquote: `> ` prefix; expandable: `**> ` prefix
  - All V2 special chars in plain text are backslash-escaped

### Added

**`PollBuilder`** - fluent builder for `send_poll`, covering the full `InputMediaPoll` field set:

```rust
use ferogram::PollBuilder;

client.send_poll(peer,
    PollBuilder::new("Favourite runtime?")
        .answers(["Tokio", "async-std", "smol"])
        .public_voters(true)
        .close_period(300)
).await?;

// Quiz with explanation
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

**`Update::GuestChatQuery`** - new update variant for `updateBotGuestChatQuery`. Fires when a user invites the bot into a guest-chat context. Carries `query_id`, `message`, `reference_messages`, and `qts`. Derefs to `IncomingMessage`. Answer with `GuestChatAnswer`:

```rust
if let Update::GuestChatQuery(q) = update {
    q.answer()
        .article("Result title")
        .text("Answer body")
        .send(&client)
        .await?;
}
```

Supports all inline result kinds: `article`, `photo`, `document`, `game`, `location`, `venue`, `contact`, `webpage`, `invoice`, `raw`. Sends via `messages.setBotGuestChatResult`.

**Other additions**

- `Client::delete_reaction(peer, msg_id, participant)` removes a specific user's reaction (`messages.reportReaction`). Returns `true` on success.
- `Client::get_poll_stats(peer, msg_id)` returns detailed vote stats (`stats.getPollStats`).
- `BannedRights::send_reactions` field on the ban-rights builder.
- User ID resolution for `CallbackQuery`, `InlineQuery`, `InlineSend`, and `GuestChatQuery`.

**Parser additions (`ferogram-parsers`):**

- `parse_markdown_v2()` / `generate_markdown_v2()` explicit V2 entry points.
- `parse_markdown_v1()` legacy V1 parser, now deprecated.
- HTML: `<ins>` accepted as underline alias.
- HTML: `<span class="tg-spoiler">` accepted as spoiler alias.
- HTML: `<blockquote>` and `<blockquote expandable>` produce the right `MessageEntityBlockquote` variants.
- HTML: `<tg-time unix="N" format="F">` produces `MessageEntityFormattedDate`.
- HTML: `<pre><code class="language-X">` correctly produces one `Pre` entity with the language set.
- `generate_html` now emits `<blockquote>`, `<blockquote expandable>`, and `<tg-time>`.
- Both `parse_html` backends are at full parity.

### Deprecated

- `parse_markdown_v1` marked `#[deprecated(since = "0.3.9")]`. Will be removed in 0.4.0.

---

## [0.3.8] - 2026-05-06

### Fixed

- `send_to_self(msg)` now actually sends to Saved Messages. Was pointing at the wrong function body in 0.3.7.
- `open_mini_app(peer, MiniApp)` is now public. Was accidentally left private in 0.3.7.
- `get_chat_full(peer)` is now `pub` instead of `pub(crate)`.
- Fixed `download_media_to_file_on_dc` and `set_default_banned_rights_raw`.

---

## [0.3.7] - 2026-05-05

The big story this release is workspace restructuring. Three crates were extracted from the monolith, the connection stack got its own home, and a handful of API rough edges were smoothed out.

### New crates

- **ferogram-connect**: the raw TCP/transport layer is now its own publishable crate. Owns the connection, MTProto framing, transport selection (Intermediate, Obfuscated, FakeTLS), SOCKS5, and proxy handling.
- **ferogram-fsm**: FSM state management extracted into its own crate. Same `FsmState`, `StateContext`, `StateStorage`, and `MemoryStorage` as before.
- **ferogram-mtsender**: the MTProto sender pool and retry policy. `RetryPolicy`, `AutoSleep`, `CircuitBreaker`, `NoRetries` all live here. The main `ferogram` crate re-exports everything.

The old `ferogram-app` and `ferogram-bot` binaries are gone, replaced by proper examples under `ferogram/examples/` (`order_bot`, `showcase_bot`, `userbot`).

### New: `PeerExt` and `OptionPeerExt`

```rust
use ferogram::{PeerExt, OptionPeerExt};

let id     = peer.bare_id();
let sender = msg.sender_id().bare_id(); // Option<i64>
let chat   = msg.peer_id().bare_id();   // Option<i64>
```

`bare_id` returns the native Telegram ID, not the Bot-API-encoded one.

### New: `PeerCache` and `ExperimentalFeatures`

`PeerCache` is now its own file and fully public.

`ExperimentalFeatures` lets you opt into behaviors that deviate from strict spec:

```rust
Client::builder()
    .experimental_features(ExperimentalFeatures {
        allow_zero_hash: true, // bots only
        ..Default::default()
    })
    .connect().await?;
```

### Breaking changes

**`download_media_to_file` renamed to `download_file`**

```rust
// before
client.download_media_to_file(location, &path).await?;

// now
client.download_file(location, &path).await?;
```

**`forward_messages` now requires `ForwardOptions`**

```rust
// before
client.forward_messages(dest, &[id], src).await?;

// now
client.forward_messages(dest, &[id], src, ForwardOptions::default()).await?;
```

**`respond_ex` removed**

`respond` already accepts `InputMessage` directly:

```rust
// before
msg.respond_ex(InputMessage::html("<b>hi</b>")).await?;

// now
msg.respond(InputMessage::html("<b>hi</b>")).await?;
```

### Internals

`ferogram/src/lib.rs` has been split up. `client/`, `filters`, and `middleware` are now proper module directories, and `peer_cache` is its own file. No public API changes.

**Full Changelog**: https://github.com/ankit-chaubey/ferogram/compare/v0.3.6...v0.3.7

---

## [0.3.6] - 2026-04-30

### API Stabilization (Towards v0.4.0)

This release focuses on giving the high-level APIs their final shape before v0.4.0. Some APIs were simplified, merged, or removed where redundant. Once stabilized, future updates will focus on new features without disruptive API changes.

See [FEATURES.md](FEATURES.md) for the full list of what is currently public and supported.

---

## [0.3.5] - 2026-04-30

### Fixed

- **PollResults deserialization** - `PollResults` was being treated as a bare type instead of a boxed type. The deserializer skipped reading the 4-byte constructor ID, consuming it as the `flags` field, which misaligned all subsequent reads inside `getChannelDifference` and `getDifference` responses containing a poll message. Fixed by removing the special-case whitelist in `namegen.rs`.

- **getDifference self-deadlock** - The `reader_loop` select arm was directly awaiting `run_pending_differences()`, which sends a getDifference RPC and awaits the response. But `reader_loop` is the only task reading TCP frames, so the response could never arrive, causing a 30-second hang after the first gap detection. Fixed by spawning a separate task. A new `diff_in_flight: AtomicBool` guard prevents concurrent redundant spawns.

### Removed

- **`prefetch_channel_access_hashes` from startup** - The automatic `messages.getDialogs` call during startup was the root cause of breakage on Telegram beta layers, forcing full deserialization of high-churn objects that change without a layer bump. Removing it makes ferogram resilient to Telegram schema drift.

### Changed

- **Lazy access_hash resolution** - Channel access hashes are now resolved purely lazily: restored from the session on startup, received from incoming update entities, or skipped gracefully if still unknown when `getChannelDifference` fires.

### Added

- **`Client::warm_peer_cache_from_dialogs()`** - The former internal `prefetch_channel_access_hashes` is now a public, opt-in method. Call it explicitly if you need a channel's access hash before any update has arrived. Do not call at startup.

---

## [0.3.4] - 2026-04-28

### Added

- **PFS (Perfect Forward Secrecy)**: `use_pfs` flag on `ClientConfig`. When enabled, the DC pool performs a temp-key DH bind after auth-key negotiation. Falls back gracefully if the bind fails.
- **`prefetch_channel_access_hashes`**: called automatically at startup and after catch-up. Runs a single `GetDialogs` to pre-populate access hashes before the first update arrives.
- **`Deserializable::from_bytes_exact`**: constructs a cursor, deserializes, and errors if bytes are left over. Replaces repeated `Cursor::from_slice` + `deserialize` patterns throughout the codebase.
- **`auth_key_id_from_key`** utility in `ferogram-mtproto`.
- **`ferogram::util`** module with `decode_checked` helper used by the pts layer.

### Fixed

- `get_difference` no longer hangs when two tasks race to call it concurrently. The second caller polls every 50 ms and gives up after 35 s, then lets the next gap tick retry.
- Parse errors on incoming `Updates` frames are now logged as warnings instead of being silently dropped.

### Changed

- All raw `Cursor::from_slice` + `deserialize` call sites migrated to `from_bytes_exact`.
- `step2_temp` re-exported from `ferogram-mtproto` auth module.

**Full Changelog**: https://github.com/ankit-chaubey/ferogram/compare/v0.3.3...v0.3.4

---

## [0.3.3] - 2026-04-22

### New crate

- **ferogram-derive**: proc-macro crate providing `#[derive(FsmState)]`. Generates `as_key` and `from_key` for unit-variant enums. Only active with the `derive` feature flag.

### New modules

- **ferogram::middleware**: `Middleware` trait, `Next` chain, `DispatchError`/`DispatchResult`, and a rate-limit middleware backed by `DashMap`.
- **ferogram::filters**: composable, synchronous predicates over `IncomingMessage`. Built-in constructors for command, private, text, media, and more. Supports `&`, `|`, `!` operators. Integrates with the FSM via `StateContext`.
- **ferogram::fsm**: `FsmState` trait, `StateContext`, `StateKey`, `StateKeyStrategy`, and `StateStorage`. In-memory `DashMap`-backed store by default; custom backends via async-trait extension point.
- **ferogram::conversation**: `Conversation` for stateful back-and-forth with a single peer. Wraps `UpdateStream` for the conversation lifetime and buffers updates from other peers.

### Added

- `IncomingMessage` gained message-inspection helpers: `chat_id()`, `is_private()`, `is_group()`, `is_channel()`, `is_any_group()`, `from_id()`, `is_bot_command()`, `command()`, `is_command_named()`, `command_args()`, `has_media()`, `has_photo()`, `has_document()`, `is_forwarded()`, `is_reply()`, `album_id()`.
- New update types exported from the crate root: `ParticipantUpdate`, `JoinRequestUpdate`, `MessageReactionUpdate`, `PollVoteUpdate`, `BotStoppedUpdate`, `ShippingQueryUpdate`, `PreCheckoutQueryUpdate`, `ChatBoostUpdate`.
- `Client::get_chat_administrators()`: returns all admins and the creator for a channel or supergroup.
- `FsmState` re-exported from the crate root. With the `derive` feature, `#[derive(FsmState)]` is available directly from `ferogram`.
- Example: `examples/order_bot.rs` showing a multi-step FSM-driven order flow.

### Docs

New pages: Middleware & Dispatcher, Finite State Machine (FSM), Conversation API, Bot Configuration, Stats & Analytics.

**Full Changelog**: https://github.com/ankit-chaubey/ferogram/compare/v0.3.2...v0.3.3

---

## [0.3.2] - 2026-04-21

### Changed

- `SeenMsgIds` deque now paired with a `HashSet` for O(1) duplicate checks under concurrent workers (was O(n)).
- Session temp files now use a unique name per write. A `write_lock` serializes concurrent saves to prevent a rename race on Windows.

### Fixed

- `PaddedIntermediate` handshake was missing in DC pool worker connections.
- `new_session_created` was resetting the session on fresh connections when it should not, causing a session ID mismatch on every decrypt after.
- `scan_body` was passing `None` as `sent_msg_id` on container iterations, letting stale results overwrite live responses.
- `importAuthorization` branch logic was inverted; it was skipping the import exactly when it was needed.
- Server 4-byte transport error codes during DH now surface properly instead of logging as "plain frame too short".

**Full Changelog**: https://github.com/ankit-chaubey/ferogram/compare/v0.3.1...v0.3.2

---

## [0.3.1] - 2026-04-20

Patch release to fix the docs.rs build. No functional changes from 0.3.0.

**Full Changelog**: https://github.com/ankit-chaubey/ferogram/compare/v0.3.0...v0.3.1

---

## [0.3.0] - 2026-04-19

0.3.0 is a substantial release. The workspace grew by two new crates, the session and parser layers were extracted into their own packages, and the connection stack gained CDN support, DNS-over-HTTPS fallback, and transport probing. About 16.7k lines added across 114 changed files.

### New crates

- **ferogram-session**: session persistence is now its own crate. Owns `PersistedSession`, `DcEntry`, `DcFlags`, `UpdatesStateSnap`, `CachedPeer`, `CachedMinPeer`, `default_dc_addresses`, and all storage backends (`BinaryFileBackend`, `InMemoryBackend`, `StringSessionBackend`, `SqliteBackend`, `LibSqlBackend`). The main `ferogram` crate re-exports everything.
- **ferogram-parsers**: Telegram Markdown and HTML entity parsing is now its own crate. Provides `parse_markdown`, `generate_markdown`, `parse_html`, and `generate_html`. Optional `html5ever` feature for spec-compliant HTML5 tokenization.

### Session changes

- Binary session format bumped to **v5**.
- Now stores home DC, full DC table, update state (pts/qts/date/seq), per-channel pts, peer cache, and min-user message contexts.
- Legacy formats load without error.
- Saves are atomic: written to `.tmp` first, then renamed into place.
- DC flags are now persisted so media and CDN DC entries survive restarts.

### Client and builder

- `ClientBuilder` gained three new options:
  - `.probe_transport(true)`: races Obfuscated, Abridged, and HTTP transports and picks the first to succeed.
  - `.resilient_connect(true)`: if direct TCP fails, falls back through DNS-over-HTTPS and then Telegram's Firebase/Google special-config.
  - `.experimental_features(...)`: takes an `ExperimentalFeatures` struct.
- New `ExperimentalFeatures` struct: `allow_zero_hash`, `allow_missing_channel_hash`, `auto_resolve_peers` (reserved).

### New modules

- `ferogram::cdn_download`: full CDN file download path. Handles `upload.getCdnFile`, `upload.reuploadCdnFile`, AES-256-CTR chunk decryption, and reassembly. Exports `CdnDownloader`, `CdnChunkResult`, and `CDN_CHUNK_SIZE`.
- `ferogram::dns_resolver`: DNS-over-HTTPS with TTL caching. Queries Google DoH and Mozilla/Cloudflare DoH, merges IPv4 and IPv6 answers.
- `ferogram::special_config`: Telegram's Firebase/Google fallback for DC configuration.

### MTProto internals

- `ferogram-mtproto` gained a new `bind_temp_key` module.
- Now re-exports `encrypt_bind_inner`, `gen_msg_id`, `serialize_bind_temp_auth_key`, `EncryptedSession`, `SeenMsgIds`, and `new_seen_msg_ids`.

### Docs

New pages: CDN Downloads, Transport Probing and Resilient Connect, Connection Restart Policy, Experimental Features, ClientBuilder, Types Reference, Chat Management, Contacts, Forum Topics, Games, Invite Links, Polls, Privacy, Profile, Stickers.

---

## [0.2.0] - 2026-04-13

### Changed

- Peer cache moved from `RwLock<HashMap>` to `moka` concurrent cache to eliminate lock contention.
- Pending RPC map replaced with `DashMap` for lock-free response routing.
- `dc_pool` now uses `tokio::sync::Mutex` instead of `parking_lot::Mutex`.
- Fresh DH sessions now wait 2 seconds after key derivation to allow Telegram to propagate the new auth key across DCs.
- Stale key detection simplified: only error `-404` triggers key rotation.
- FakeTLS transport now prepends the Change Cipher Spec record to the first application data chunk.
- `getDifference` deserialization now tolerates unknown server responses instead of failing and dropping buffered updates.
- Container message parsing now validates inner message alignment and discards malformed frames safely.
- Transport errors `-429` and `-444` are now logged clearly before reconnecting.

### Fixed

- `getDifference` deserialization no longer fails hard on unknown server responses.
- Container message parsing now discards malformed frames instead of propagating a parse error.
- Transport errors `-429` and `-444` now surface as log warnings before reconnecting.

---

## [0.1.0] - 2026-04-11

Initial release.
