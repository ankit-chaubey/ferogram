# Configuration

`Config` is the struct passed to `Client::connect`. The recommended way to build it is via `Client::builder()`. All fields except `api_id` and `api_hash` have defaults.

```rust
use ferogram::{Client, TransportKind};

let (client, _shutdown) = Client::builder()
    .api_id(12345)
    .api_hash("your_api_hash")
    .session("bot.session")       // default: "ferogram.session"
    .transport(TransportKind::Obfuscated { secret: None })
    .catch_up(false)
    .connect()
    .await?;
```

---

## Builder methods

### `api_id` / `api_hash`

Required. Get these from [my.telegram.org](https://my.telegram.org).

```rust
.api_id(12345)
.api_hash("your_hash")
```

### `session`

Path to a binary session file. Default: `"ferogram.session"`.

```rust
.session("mybot.session")
```

### `session_string`

Portable base64 session (for serverless / env-var storage). Pass an empty string to start fresh.

```rust
.session_string(std::env::var("SESSION").unwrap_or_default())
```

### `in_memory`

Non-persistent session. Useful for tests.

```rust
.in_memory()
```

### `session_backend`

Inject a custom `SessionBackend` directly, e.g. `LibSqlBackend`:

```rust
use std::sync::Arc;
use ferogram::LibSqlBackend;

.session_backend(Arc::new(LibSqlBackend::new("remote.db")))
```

### `transport`

Which MTProto framing to use. Default: `TransportKind::Abridged`.

| Variant | Notes |
|---|---|
| `Abridged` | Minimal overhead. Default. |
| `Intermediate` | 4-byte LE length prefix. Better compat with some proxies. |
| `Full` | Intermediate + seqno + CRC32 integrity check. |
| `Obfuscated { secret }` | AES-256-CTR (Obfuscated2). Pass `secret: None` for direct connections, or a 16-byte key for MTProxy with a plain secret. |
| `PaddedIntermediate { secret }` | Obfuscated2 with padded Intermediate framing. Required for `0xDD` MTProxy secrets. |
| `FakeTls { secret, domain }` | Disguises traffic as a TLS 1.3 ClientHello. Required for `0xEE` MTProxy secrets. |

```rust
use ferogram::TransportKind;

// plain obfuscation, no proxy
.transport(TransportKind::Obfuscated { secret: None })

// Intermediate framing
.transport(TransportKind::Intermediate)

// FakeTLS (manual, normally set by .mtproxy())
.transport(TransportKind::FakeTls {
    secret: [0xab; 16],
    domain: "example.com".into(),
})
```

When using `.mtproxy()`, the transport is set automatically. Do not also call `.transport()`.

### `socks5`

Route connections through a SOCKS5 proxy.

```rust
use ferogram::socks5::Socks5Config;

// no auth
.socks5(Socks5Config::new("127.0.0.1:1080"))

// with auth
.socks5(Socks5Config::with_auth("proxy.example.com:1080", "user", "pass"))
```

### `mtproxy`

Route connections through an MTProxy relay. The transport is auto-selected from the secret.

```rust
use ferogram::proxy::parse_proxy_link;

let proxy = parse_proxy_link("tg://proxy?server=...&port=443&secret=...").unwrap();
.mtproxy(proxy)
```

See [Proxies & Transports](./advanced/proxy.md) for full details.

### `dc_addr`

Override the initial DC address. After login the correct DC is cached in the session, so this is only needed if you know exactly which DC to target.

```rust
.dc_addr("149.154.167.51:443")  // DC2
```

### `catch_up`

When `true`, replays missed updates via `updates.getDifference` on reconnect. Default: `false`.

```rust
.catch_up(true)
```

### `allow_ipv6`

Allow IPv6 DC addresses. Default: `false`.

```rust
.allow_ipv6(true)
```

### `retry_policy`

How to handle `FLOOD_WAIT` errors. Default: `AutoSleep` (sleep the required duration and retry).

```rust
use std::sync::Arc;
use ferogram::retry::{AutoSleep, NoRetries};

.retry_policy(Arc::new(AutoSleep::default()))   // sleep and retry
.retry_policy(Arc::new(NoRetries))              // propagate immediately
```

### `restart_policy`

What to do when the TCP connection drops. Default: `NeverRestart` (exit the event loop; the shutdown signal fires).

```rust
use std::sync::Arc;
use std::time::Duration;
use ferogram::FixedInterval;

.restart_policy(Arc::new(FixedInterval {
    interval: Duration::from_secs(5),
}))
```

See [Connection Restart Policy](./advanced/connection-restart.md) for all built-in types and custom implementations.

### `probe_transport`

Race Obfuscated, Abridged, and HTTP transports in parallel on connect and keep the fastest. Incompatible with MTProxy. Default: `false`.

```rust
.probe_transport(true)
```

### `resilient_connect`

If direct TCP fails, retry via DNS-over-HTTPS then Firebase special-config. Useful in ISP-blocked regions. Default: `false`.

```rust
.resilient_connect(true)
```

See [Transport Probing & Resilient Connect](./advanced/transport-probing.md) for the full fallback chain.

### `experimental_features`

Opt-in flags that deviate from strict Telegram spec. All default to `false`.

```rust
use ferogram::ExperimentalFeatures;

.experimental_features(ExperimentalFeatures {
    allow_zero_hash: true,   // bots only
    ..Default::default()
})
```

See [Experimental Features](./advanced/experimental-features.md) for all flags and safety constraints.

---

## Building Config without connecting

```rust
let config = Client::builder()
    .api_id(12345)
    .api_hash("hash")
    .build()?;

// later
let (client, _shutdown) = Client::connect(config).await?;
```

`build()` returns `Err(BuilderError::MissingApiId)` or `Err(BuilderError::MissingApiHash)` if those fields are missing, before touching the network.
