# ferogram-connect

Raw TCP connection, MTProto framing, and transport layer for ferogram.

[![Crates.io](https://img.shields.io/crates/v/ferogram-connect?color=fc8d62)](https://crates.io/crates/ferogram-connect)
[![Telegram](https://img.shields.io/badge/community-%40FerogramChat-2CA5E0?logo=telegram)](https://t.me/FerogramChat) [![Channel](https://img.shields.io/badge/channel-%40Ferogram-2CA5E0?logo=telegram)](https://t.me/Ferogram)
[![docs.rs](https://img.shields.io/badge/docs.rs-ferogram--connect-5865F2)](https://docs.rs/ferogram-connect)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](#license)

Sits between a raw `TcpStream` and decrypted MTProto messages. The `ferogram` crate uses this internally; most people never need to depend on it directly. If you're just building a bot or a client, start with [`ferogram`](https://crates.io/crates/ferogram) instead.

`ferogram` re-exports everything here, so existing code needs no changes.

## What it does

Takes a TCP connection to a Telegram DC and gives back decrypted, framed MTProto messages.

- Abridged, Intermediate, Padded Intermediate, and Full transport framing
- Obfuscated2 AES-256-CTR transport for bypassing DPI and MTProxy
- FakeTLS transport for `0xee` MTProxy secrets
- SOCKS5 proxy with optional username/password
- MTProxy (`tg://proxy?...` and `https://t.me/proxy?...`) parsing and connection
- Keepalive pings with configurable interval
- gzip inflate and compress for MTProto containers
- MTProto envelope unwrapping and entity extraction

## Transports

```rust
use ferogram_connect::TransportKind;

// Plain abridged (default)
TransportKind::Abridged

// Obfuscated2: bypasses ISP blocks, works with plain MTProxy secrets
TransportKind::Obfuscated { secret: None }

// Padded Intermediate: required for 0xdd MTProxy secrets
TransportKind::PaddedIntermediate { secret: Some(key) }

// FakeTLS: required for 0xee MTProxy secrets
TransportKind::FakeTls { secret: key, domain: "...".into() }
```

## MTProxy

```rust
use ferogram_connect::MtProxyConfig;

// Parse a proxy link directly
let proxy = ferogram::proxy::parse_proxy_link("https://t.me/proxy?server=...&port=443&secret=...")?;

// Or build manually
let proxy = MtProxyConfig {
    host: "proxy.example.com".into(),
    port: 443,
    secret: vec![/* raw bytes */],
    transport: TransportKind::Obfuscated { secret: None },
};
```

## SOCKS5

```rust
use ferogram_connect::Socks5Config;

let socks = Socks5Config::new("127.0.0.1:1080");
let socks_auth = Socks5Config::with_auth("127.0.0.1:1080", "user", "pass");
```

## Stack position

```
ferogram
└ ferogram-mtsender
  └ ferogram-connect  <-- here
    ├ ferogram-mtproto
    └ ferogram-crypto
```

## License

MIT or Apache-2.0, at your option. See [LICENSE-MIT](../LICENSE-MIT) and [LICENSE-APACHE](../LICENSE-APACHE).

**Ankit Chaubey** - [github.com/ankit-chaubey](https://github.com/ankit-chaubey)
