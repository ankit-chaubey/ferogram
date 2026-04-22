# Changelog

All notable changes to this project will be documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.0.0/)
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

> **Note:** ferogram is the continuation of [layer](https://github.com/ankit-chaubey/layer).
> For history prior to v0.1.0, see the [layer changelog](https://github.com/ankit-chaubey/layer/blob/main/CHANGELOG.md) (up to layer v0.5.0).

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
