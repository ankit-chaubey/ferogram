# ferogram-connect

Raw TCP connection, MTProto framing, and transport layer for ferogram.

[![Crates.io](https://img.shields.io/crates/v/ferogram-connect?style=flat-square&logo=rust&logoColor=white&color=F97316)](https://crates.io/crates/ferogram-connect)
[![Telegram Channel](https://img.shields.io/badge/Channel-Ferogram-06B6D4?style=flat-square&logo=telegram&logoColor=white)](https://t.me/Ferogram) [![Telegram Chat](https://img.shields.io/badge/Chat-FerogramChat-06B6D4?style=flat-square&logo=telegram&logoColor=white)](https://t.me/FerogramChat)
[![docs.rs](https://img.shields.io/badge/docs.rs-ferogram--connect-5865F2?style=flat-square&logo=docs.rs&logoColor=white)](https://docs.rs/ferogram-connect)
[![License](https://img.shields.io/badge/License-MIT%20%7C%20Apache--2.0-64748B?style=flat-square)](#license)

Sits between a raw `TcpStream` and decrypted MTProto messages. The `ferogram` crate uses this internally; most people never need to depend on it directly. If you're just building a bot or a client, start with [`ferogram`](https://crates.io/crates/ferogram) instead.

`ferogram` re-exports everything here, so existing code needs no changes.

## What it does

Takes a TCP connection to a Telegram DC and gives back decrypted, framed MTProto messages.

- Abridged, Intermediate, Padded Intermediate, and Full transport framing
- Obfuscated2 AES-256-CTR transport for bypassing DPI and MTProxy
- FakeTLS transport for `0xee` MTProxy secrets
- SOCKS5 proxy with optional username/password (feature: `socks5`)
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

## Direct address-family fallback

Direct connections to recognized production Telegram addresses on port 443
(DCs 1-5) try the supplied address first until a successful route is learned.
An I/O failure or a four-second reachability timeout tries the opposite IP
family of the same DC once. Both failures are returned if neither route works;
protocol/auth rejections propagate without an address retry.

Reachability requires an unauthenticated, nonce-matched `req_pq_multi` / `resPQ`
exchange, since TCP acceptance alone does not detect networks that reset the
first MTProto packet. The established stream keeps its framing state, and cached
auth keys are preserved. Every eligible new socket pays one extra round trip;
the four-second deadline is per attempt, not an end-to-end login deadline.

One atomic, process-wide hint remembers the last successful address family for
fresh connections, migration, media sockets and reconnects. If the preferred
family fails and the other succeeds, the preference switches back. It resets on
process restart, is shared across clients and DCs, and is never persisted in a
session. Concurrent attempts snapshot the hint independently; the last
successful probe to store its result wins. Login-code/password failures do not
change it, and no login requests are replayed by this policy.

SOCKS5 and MTProxy bypass this policy. Custom endpoints, non-443 ports, test DCs
and CDN DC IDs outside 1-5 are not redirected and do not teach the preference.
The built-in alternate endpoints are bootstrap routes, not a replacement for
Telegram's dynamic config. Numeric IPv6 endpoints accept both bracketed and
unbracketed config forms.

Offline tests use injected dial results and isolated preferences:

```sh
cargo test -p ferogram-connect --lib
```

An ignored, opt-in test exercises live DC2/DC5 fresh DH, cached-key preservation,
and the sender/pool entry point. It uses only temporary in-memory keys, with no
account login or disk sessions:

```sh
cargo test -p ferogram-connect live_production_routes -- --ignored --nocapture
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

## SOCKS5 (feature: `socks5`)

Pulls in `tokio-socks`; off by default since it's not needed for direct connections to Telegram datacenters.

```rust
use ferogram_connect::Socks5Config;

let socks = Socks5Config::new("127.0.0.1:1080");
let socks_auth = Socks5Config::with_auth("127.0.0.1:1080", "user", "pass");
```

## Feature flags

| Flag | What it enables |
|---|---|
| `socks5` | `Socks5Config` and SOCKS5 proxy support for outgoing connections |

## Stack position

```
ferogram
└ ferogram-mtsender
  └ ferogram-connect  <-- here
    ├ ferogram-mtproto
    └ ferogram-crypto
```

## License

This project is licensed under either the MIT License or Apache License 2.0, at your option. See [`LICENSE-MIT`](https://github.com/ankit-chaubey/ferogram/blob/main/LICENSE-MIT) and [`LICENSE-APACHE`](https://github.com/ankit-chaubey/ferogram/blob/main/LICENSE-APACHE) for details.

**Author:** Ankit Chaubey ([@ankit-chaubey](https://github.com/ankit-chaubey))
