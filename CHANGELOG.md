# Changelog

All notable changes to this project will be documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.0.0/)
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

> **Note:** ferogram is the continuation of [layer](https://github.com/ankit-chaubey/layer).
> For history prior to v0.1.0, see the [layer changelog](https://github.com/ankit-chaubey/layer/blob/main/CHANGELOG.md) (up to layer v0.5.0).

---

## [Unreleased]

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
