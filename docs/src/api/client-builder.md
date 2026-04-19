# ClientBuilder

`ClientBuilder` is the fluent, type-safe constructor for a `Client` connection.
Obtain one via `Client::builder()`.

```rust,no_run
use ferogram::Client;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let (client, _shutdown) = Client::builder()
        .api_id(12345)
        .api_hash("abc123")
        .session("my.session")
        .catch_up(true)
        .connect()
        .await?;
    Ok(())
}
```

`connect()` returns `Result<(Client, ShutdownToken), BuilderError>`. The
`BuilderError` can be `MissingApiId`, `MissingApiHash`, or a network-level
`Connect(InvocationError)`.

---

## Credentials

<div class="api-card">
<div class="api-card-header">
<span class="api-badge">sync</span>
<span class="api-card-sig">.api_id(id: i32) → ClientBuilder</span>
</div>
<div class="api-card-body">
Set the Telegram API ID obtained from <a href="https://my.telegram.org">my.telegram.org</a>.
Required  -  <code>connect()</code> returns <code>BuilderError::MissingApiId</code> if not set.
</div>
</div>

<div class="api-card">
<div class="api-card-header">
<span class="api-badge">sync</span>
<span class="api-card-sig">.api_hash(hash: impl Into&lt;String&gt;) → ClientBuilder</span>
</div>
<div class="api-card-body">
Set the Telegram API hash from <a href="https://my.telegram.org">my.telegram.org</a>.
Required  -  <code>connect()</code> returns <code>BuilderError::MissingApiHash</code> if not set.
</div>
</div>

---

## Session

Three session backends are available. They are mutually exclusive  -  the **last call wins**.

<div class="api-card">
<div class="api-card-header">
<span class="api-badge">sync</span>
<span class="api-card-sig">.session(path: impl AsRef&lt;Path&gt;) → ClientBuilder</span>
</div>
<div class="api-card-body">
Use a binary file session at <code>path</code>. This is the default backend
(<code>"ferogram.session"</code> in the working directory if no session method is called).
<pre><code>.session("mybot.session")</code></pre>
</div>
</div>

<div class="api-card">
<div class="api-card-header">
<span class="api-badge">sync</span>
<span class="api-card-sig">.session_string(s: impl Into&lt;String&gt;) → ClientBuilder</span>
</div>
<div class="api-card-body">
Use a portable base64 string session. Pass an empty string to start fresh; the
string exported by <code>client.export_session_string()</code> can be injected here directly
(e.g. via an environment variable). No file is written to disk.
<pre><code>.session_string(std::env::var("SESSION").unwrap_or_default())</code></pre>
</div>
</div>

<div class="api-card">
<div class="api-card-header">
<span class="api-badge">sync</span>
<span class="api-card-sig">.in_memory() → ClientBuilder</span>
</div>
<div class="api-card-body">
Use a non-persistent in-memory session. The session is lost when the process
exits. Useful for tests and throwaway scripts.
</div>
</div>

<div class="api-card">
<div class="api-card-header">
<span class="api-badge">sync</span>
<span class="api-card-sig">.session_backend(backend: Arc&lt;dyn SessionBackend&gt;) → ClientBuilder</span>
</div>
<div class="api-card-body">
Inject a fully custom <code>SessionBackend</code> implementation. Use this for
<code>LibSqlBackend</code> (bundled SQLite, no system dependency) or any
custom persistence layer.
<pre><code>#[cfg(feature = "libsql-session")]
use ferogram::LibSqlBackend;
use std::sync::Arc;

.session_backend(Arc::new(LibSqlBackend::new("my.db")))</code></pre>
Requires the <code>libsql-session</code> feature for <code>LibSqlBackend</code>.
See <a href="../features.md">Feature Flags</a>.
</div>
</div>

---

## Updates

<div class="api-card">
<div class="api-card-header">
<span class="api-badge">sync</span>
<span class="api-card-sig">.catch_up(enabled: bool) → ClientBuilder</span>
</div>
<div class="api-card-body">
When <code>true</code>, replay missed updates via <code>updates.getDifference</code> immediately after
connecting. Useful for bots or userbots that must not miss messages during downtime.
Default: <code>false</code>.
</div>
</div>

---

## Network

<div class="api-card">
<div class="api-card-header">
<span class="api-badge">sync</span>
<span class="api-card-sig">.dc_addr(addr: impl Into&lt;String&gt;) → ClientBuilder</span>
</div>
<div class="api-card-body">
Override the first DC address. Useful when connecting to a test server.
<pre><code>.dc_addr("149.154.167.40:443")   // production DC 1
.dc_addr("149.154.167.40:80")    // test DC</code></pre>
</div>
</div>

<div class="api-card">
<div class="api-card-header">
<span class="api-badge">sync</span>
<span class="api-card-sig">.allow_ipv6(allow: bool) → ClientBuilder</span>
</div>
<div class="api-card-body">
Allow IPv6 DC addresses when resolving the DC table. Default: <code>false</code>.
</div>
</div>

<div class="api-card">
<div class="api-card-header">
<span class="api-badge">sync</span>
<span class="api-card-sig">.transport(kind: TransportKind) → ClientBuilder</span>
</div>
<div class="api-card-body">
Choose the MTProto transport framing layer.

| Variant | Description |
|---|---|
| `TransportKind::Abridged` | Smallest overhead (default) |
| `TransportKind::Intermediate` | Compatible with more proxies |
| `TransportKind::Obfuscated` | Deep-packet-inspection resistant |
| `TransportKind::Http` | Plain HTTP wrapping |

Note: when using `.mtproxy()` or `.proxy_link()`, the transport is set automatically from the secret prefix  -  do not also call `.transport()`.
</div>
</div>

<div class="api-card">
<div class="api-card-header">
<span class="api-badge">sync</span>
<span class="api-card-sig">.probe_transport(enabled: bool) → ClientBuilder</span>
</div>
<div class="api-card-body">
Race Obfuscated, Abridged, and HTTP transports in parallel and connect using
whichever completes the DH handshake first. The losing attempts are cancelled
immediately. Ideal when you don't know which transport your network or firewall
permits. Incompatible with MTProxy. Default: <code>false</code>.

```rust
.probe_transport(true)
```

See <a href="../advanced/transport-probing.md">Transport Probing & Resilient Connect</a> for the race schedule, timing details, and interaction with MTProxy.
</div>
</div>

<div class="api-card">
<div class="api-card-header">
<span class="api-badge">sync</span>
<span class="api-card-sig">.resilient_connect(enabled: bool) → ClientBuilder</span>
</div>
<div class="api-card-body">
If direct TCP fails, retry via DNS-over-HTTPS (Mozilla + Google DoH), then
fall back to Firebase / Google special-config endpoints. Useful in restricted
networks where Telegram DCs are ISP-blocked. Default: <code>false</code>.

```rust
.resilient_connect(true)
```

See <a href="../advanced/transport-probing.md">Transport Probing & Resilient Connect</a> for the full fallback chain and when to combine with <code>probe_transport</code>.
</div>
</div>

---

## Proxy

<div class="api-card">
<div class="api-card-header">
<span class="api-badge">sync</span>
<span class="api-card-sig">.socks5(proxy: Socks5Config) → ClientBuilder</span>
</div>
<div class="api-card-body">
Route all connections through a SOCKS5 proxy. Build the config with:
<pre><code>use ferogram::Socks5Config;
let proxy = Socks5Config {
    host: "127.0.0.1".to_string(),
    port: 1080,
    user: None,
    password: None,
};
.socks5(proxy)</code></pre>
</div>
</div>

<div class="api-card">
<div class="api-card-header">
<span class="api-badge">sync</span>
<span class="api-card-sig">.mtproxy(proxy: MtProxyConfig) → ClientBuilder</span>
</div>
<div class="api-card-body">
Route all connections through an MTProxy. The transport is automatically selected
from the proxy secret prefix; do not also call <code>.transport()</code>.
Build with <code>ferogram::parse_proxy_link(url)</code> or construct manually.
</div>
</div>

<div class="api-card">
<div class="api-card-header">
<span class="api-badge">sync</span>
<span class="api-card-sig">.proxy_link(url: &str) → ClientBuilder</span>
</div>
<div class="api-card-body">
Set an MTProxy from a <code>https://t.me/proxy?...</code> or <code>tg://proxy?...</code> link.
An empty string is a no-op. Transport is selected from the secret prefix automatically.
<pre><code>.proxy_link("https://t.me/proxy?server=1.2.3.4&port=443&secret=abc123")</code></pre>
See <a href="../advanced/proxy.md">Proxies & Transports</a> for full details.
</div>
</div>

---

## Identity (InitConnection)

These strings are sent to Telegram in the `InitConnection` call and appear in
the active sessions list on <a href="https://my.telegram.org">my.telegram.org</a>.

<div class="api-card">
<div class="api-card-header">
<span class="api-badge">sync</span>
<span class="api-card-sig">.device_model(model: impl Into&lt;String&gt;) → ClientBuilder</span>
</div>
<div class="api-card-body">Device model shown in sessions. Default: <code>"Linux"</code>.<br>Example: <code>.device_model("Pixel 9 Pro")</code></div>
</div>

<div class="api-card">
<div class="api-card-header">
<span class="api-badge">sync</span>
<span class="api-card-sig">.system_version(version: impl Into&lt;String&gt;) → ClientBuilder</span>
</div>
<div class="api-card-body">OS / system version shown in sessions. Default: <code>"1.0"</code>.<br>Example: <code>.system_version("Android 15")</code></div>
</div>

<div class="api-card">
<div class="api-card-header">
<span class="api-badge">sync</span>
<span class="api-card-sig">.app_version(version: impl Into&lt;String&gt;) → ClientBuilder</span>
</div>
<div class="api-card-body">App version shown in sessions. Default: the crate version from <code>CARGO_PKG_VERSION</code>.</div>
</div>

<div class="api-card">
<div class="api-card-header">
<span class="api-badge">sync</span>
<span class="api-card-sig">.lang_code(code: impl Into&lt;String&gt;) → ClientBuilder</span>
</div>
<div class="api-card-body">BCP-47 language code sent in <code>InitConnection</code>. Default: <code>"en"</code>.</div>
</div>

<div class="api-card">
<div class="api-card-header">
<span class="api-badge">sync</span>
<span class="api-card-sig">.system_lang_code(code: impl Into&lt;String&gt;) → ClientBuilder</span>
</div>
<div class="api-card-body">System language code. Default: <code>"en"</code>.</div>
</div>

<div class="api-card">
<div class="api-card-header">
<span class="api-badge">sync</span>
<span class="api-card-sig">.lang_pack(pack: impl Into&lt;String&gt;) → ClientBuilder</span>
</div>
<div class="api-card-body">Language pack name. Default: <code>""</code> (empty). Leave unset unless building a client that mirrors an official Telegram app.</div>
</div>

---

## Retry & Restart

<div class="api-card">
<div class="api-card-header">
<span class="api-badge">sync</span>
<span class="api-card-sig">.retry_policy(policy: Arc&lt;dyn RetryPolicy&gt;) → ClientBuilder</span>
</div>
<div class="api-card-body">
Override the flood-wait / rate-limit retry strategy.
The default is <code>AutoSleep</code>: automatically sleep for <code>FLOOD_WAIT</code> durations.
See <a href="../advanced/retry.md">Retry & Flood Wait</a>.
</div>
</div>

<div class="api-card">
<div class="api-card-header">
<span class="api-badge">sync</span>
<span class="api-card-sig">.restart_policy(policy: Arc&lt;dyn ConnectionRestartPolicy&gt;) → ClientBuilder</span>
</div>
<div class="api-card-body">
Override the reconnect behaviour after a connection drop. The default is
<code>NeverRestart</code>: the event loop exits on disconnect and the shutdown
signal fires. Use <code>FixedInterval</code> for automatic reconnection, or
implement the trait for custom backoff logic.

```rust
use std::sync::Arc;
use std::time::Duration;
use ferogram::FixedInterval;

// Reconnect 5 seconds after any drop
.restart_policy(Arc::new(FixedInterval {
    interval: Duration::from_secs(5),
}))
```

See <a href="../advanced/connection-restart.md">Connection Restart Policy</a> for all built-in types, custom implementation, and scheduled periodic restarts.
</div>
</div>

---

## Experimental Features

<div class="api-card">
<div class="api-card-header">
<span class="api-badge">sync</span>
<span class="api-card-sig">.experimental_features(features: ExperimentalFeatures) → ClientBuilder</span>
</div>
<div class="api-card-body">
Opt in to experimental behaviours that deviate from strict Telegram spec.
All flags default to <code>false</code>. Always use <code>..Default::default()</code>
to stay forward-compatible with new flags.

```rust
use ferogram::{Client, ExperimentalFeatures};

Client::builder()
    .api_id(12345)
    .api_hash("abc")
    .experimental_features(ExperimentalFeatures {
        allow_zero_hash: true,  // bots only: allow hash=0 on cache miss
        ..Default::default()
    })
    .connect()
    .await?;
```

See <a href="../advanced/experimental-features.md">Experimental Features</a> for all flags, safety constraints, and when to use each one.
</div>
</div>

---

## Terminal Methods

<div class="api-card">
<div class="api-card-header">
<span class="api-badge">sync</span>
<span class="api-card-sig">.build() → Result&lt;Config, BuilderError&gt;</span>
</div>
<div class="api-card-body">
Build the <code>Config</code> struct without establishing a network connection. Useful if
you want to pass the <code>Config</code> to <code>Client::connect(config)</code> manually, or to
inspect the built configuration.
</div>
</div>

<div class="api-card">
<div class="api-card-header">
<span class="api-badge api-badge-async">async</span>
<span class="api-card-sig">.connect() → Result&lt;(Client, ShutdownToken), BuilderError&gt;</span>
</div>
<div class="api-card-body">
Build the <code>Config</code> and connect in one step. Returns
<code>Err(BuilderError::MissingApiId)</code> or <code>Err(BuilderError::MissingApiHash)</code>
before attempting any network I/O if required fields are absent.
</div>
</div>

---

## BuilderError

| Variant | Meaning |
|---|---|
| `BuilderError::MissingApiId` | `.api_id()` was not called (or set to 0) |
| `BuilderError::MissingApiHash` | `.api_hash()` was not called (or left empty) |
| `BuilderError::Connect(InvocationError)` | Network / MTProto connection failed |

```rust,no_run
match client_result {
    Err(BuilderError::MissingApiId) => eprintln!("Set API_ID"),
    Err(BuilderError::MissingApiHash) => eprintln!("Set API_HASH"),
    Err(BuilderError::Connect(e)) => eprintln!("Network error: {e}"),
    Ok((client, _)) => { /* use client */ }
}
```
