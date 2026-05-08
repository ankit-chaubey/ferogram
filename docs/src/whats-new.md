# Release History

ferogram started as a renamed continuation of [layer](https://github.com/ankit-chaubey/layer) v0.5.0. Every release since then has been a proper source release with a tagged version and a changelog entry.

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

Released 2026-04-11. The initial release of ferogram, renamed and rebranded from [layer](https://github.com/ankit-chaubey/layer) v0.5.0.

### Proxy and transport

Full MTProxy support via `t.me/proxy` or `tg://proxy` links, or manually with host, port, and secret. PaddedIntermediate transport (`0xDD` secrets) adds randomized padding to blend in with official Telegram client traffic. FakeTLS transport (`0xEE` secrets) wraps MTProto in TLS-like framing. SOCKS5 proxy with optional username and password. IPv6 connectivity for both Telegram DCs and proxy connections.

### Session backends

Binary file, in-memory, string/base64, SQLite, and libSQL.

### Protocol fixes

Auth key generation now uses the correct `PQInnerDataDc` constructor with the DC id included, resolving auth failures on many DCs. Incoming message validation uses a rolling buffer of the last 500 server `msg_id` values plus a 300 second timestamp window to prevent replay attacks. DH step 3 retry (`dh_gen_retry`) retries with cached params for up to 5 attempts, matching Telegram Desktop behavior. MTProxy connections now correctly route through the proxy host instead of going directly to Telegram DCs. `getChannelDifference` starts at limit 100 and increases to 1000 on subsequent calls.

---

See the full [CHANGELOG](https://github.com/ankit-chaubey/ferogram/blob/main/CHANGELOG.md) for the raw entry format.
