# Release History

ferogram started as a renamed continuation of [layer](https://github.com/ankit-chaubey/layer) v0.5.0. Every release since then has been a proper source release with a tagged version and a changelog entry.

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
