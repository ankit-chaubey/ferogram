# Proxies

`ferogram` supports two proxy types: SOCKS5 (generic TCP tunnel) and MTProxy (Telegram's native obfuscated proxy protocol).

## SOCKS5

### Without authentication

```rust
use ferogram::{Client, socks5::Socks5Config};

let (client, _shutdown) = Client::builder()
    .api_id(12345)
    .api_hash("your_hash")
    .session("bot.session")
    .socks5(Socks5Config::new("127.0.0.1:1080"))
    .connect()
    .await?;
```

### With username/password authentication

```rust
.socks5(Socks5Config::with_auth(
    "proxy.example.com:1080",
    "username",
    "password",
))
```

### Tor

Point SOCKS5 at `127.0.0.1:9050` (default Tor SOCKS port):

```rust
.socks5(Socks5Config::new("127.0.0.1:9050"))
```

Tor exit nodes are sometimes blocked by Telegram DCs. If connections fail consistently, try a different circuit or use `TransportKind::Obfuscated` alongside it.

---

## MTProxy

MTProxy is Telegram's own proxy protocol. It uses obfuscated transports and connects you directly to a Telegram DC via a third-party relay server.

Use `parse_proxy_link` to decode a `tg://proxy?...` or `https://t.me/proxy?...` link. The transport is selected automatically from the secret prefix.

```rust
use ferogram::{Client, proxy::parse_proxy_link};

let proxy = parse_proxy_link(
    "tg://proxy?server=proxy.example.com&port=443&secret=eedeadbeef..."
).expect("invalid proxy link");

let (client, _shutdown) = Client::builder()
    .api_id(12345)
    .api_hash("your_hash")
    .session("bot.session")
    .mtproxy(proxy)
    .connect()
    .await?;
```

`.mtproxy()` sets the transport automatically. Do not also call `.transport()` when using MTProxy.

### Secret format and transport mapping

| Secret prefix | Transport selected |
|---|---|
| 32 hex chars (plain) | `Obfuscated` (Obfuscated2, Abridged framing) |
| `dd` + 32 hex chars | `PaddedIntermediate` (Obfuscated2, padded framing) |
| `ee` + 32 hex + domain | `FakeTls` (TLS 1.3 ClientHello disguise) |

Secrets can be hex strings or base64url. `parse_proxy_link` handles both.

### Building MtProxyConfig manually

```rust
use ferogram::proxy::{MtProxyConfig, secret_to_transport};

let secret_hex = "dddeadbeefdeadbeefdeadbeefdeadbeef";
let secret_bytes: Vec<u8> = (0..secret_hex.len())
    .step_by(2)
    .map(|i| u8::from_str_radix(&secret_hex[i..i+2], 16).unwrap())
    .collect();

let proxy = MtProxyConfig {
    host:      "proxy.example.com".into(),
    port:      443,
    transport: secret_to_transport(&secret_bytes),
    secret:    secret_bytes,
};

let (client, _shutdown) = Client::builder()
    .api_id(12345)
    .api_hash("your_hash")
    .mtproxy(proxy)
    .connect()
    .await?;
```
