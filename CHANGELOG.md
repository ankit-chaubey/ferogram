# Changelog

All notable changes to this project will be documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.0.0/)
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

> **Note:** ferogram is the continuation of [layer](https://github.com/ankit-chaubey/layer).
> For history prior to v0.1.0, see the [layer changelog](https://github.com/ankit-chaubey/layer/blob/main/CHANGELOG.md) (up to layer v0.5.0).

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

- All known bugs yet


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
