# Transport Probing & Resilient Connect

Two independent options that improve connectivity in restricted networks. They
can be used separately or together.

---

## `probe_transport`

Races three MTProto transports in parallel at connect time and keeps whichever
completes the DH handshake first. The remaining attempts are cancelled
immediately so no extra bandwidth is used after a winner is picked.

```rust
let (client, _shutdown) = Client::builder()
    .api_id(12345)
    .api_hash("your_hash")
    .session("bot.session")
    .probe_transport(true)
    .connect()
    .await?;
```

### Race schedule

| Transport | Start delay | Notes |
|---|---|---|
| `Obfuscated` | 0 ms | Runs first. Best for DPI-heavy networks. |
| `Abridged` | 200 ms | Staggered so Obfuscated has a head start. |
| `Http` | 800 ms | Last resort. SOCKS5 is not used for HTTP probes. |

The winner's connection is reused directly. No second DH exchange is
performed, so there is no extra round trip cost.

If all three transports fail, the connection attempt returns the last
`InvocationError` from the race.

### When to use

Use `probe_transport` when:

- You are deploying to a network you do not control (shared hosting, cloud
  functions, university networks).
- Your region has inconsistent DPI filtering where some transports are blocked
  but others work.
- You want automatic transport selection without hardcoding `TransportKind`.

### Incompatibility with MTProxy

`probe_transport` is **incompatible with MTProxy**. An MTProxy enforces its own
transport (set by the secret prefix), so probing makes no sense. If you set
both, `probe_transport` is silently skipped and the MTProxy transport is used
as normal.

```rust
// Wrong: probe_transport has no effect when mtproxy is set
.mtproxy(proxy)
.probe_transport(true)   // ignored

// Correct: use one or the other
.probe_transport(true)   // for direct connections
```

---

## `resilient_connect`

If the initial direct TCP connect fails, `resilient_connect` tries two
additional fallback paths before giving up. Normal operation is unaffected when
the direct connect succeeds; the fallbacks only activate on failure.

```rust
let (client, _shutdown) = Client::builder()
    .api_id(12345)
    .api_hash("your_hash")
    .session("bot.session")
    .resilient_connect(true)
    .connect()
    .await?;
```

### Fallback chain

```
Direct TCP
    |
    v (fails)
DNS-over-HTTPS  (Mozilla DoH + Google DoH)
    resolves venus.web.telegram.org -> IP list
    tries each IP on the DC port
    |
    v (all fail)
Firebase / Google special-config
    fetches Telegram's Firebase-hosted DC address list
    tries each matching DC option
    |
    v (all fail)
Returns the original direct-connect error
```

**Step 1: DNS-over-HTTPS.** Queries `venus.web.telegram.org` via Mozilla and
Google DoH resolvers. Each resolved IP is tried on the same port as the default
DC address. This bypasses ISP-level DNS poisoning.

**Step 2: Firebase / Google special-config.** Fetches the alternate DC address
list that Telegram publishes to Firebase. This is the same endpoint the
official Telegram apps fall back to when all normal connections fail. Only
addresses matching the target DC ID are tried.

If every path fails, the original direct-connect error is returned.

### When to use

Use `resilient_connect` when:

- You are deploying in a region where Telegram DCs are ISP-blocked (Iran,
  Russia, some corporate networks).
- You want your bot to self-recover from transient DNS outages without manual
  restarts.
- You are running behind a network that intercepts DNS but allows HTTPS.

### Combining both options

`probe_transport` and `resilient_connect` are orthogonal and can be enabled
together. `probe_transport` changes which transport framing is used for a
successful direct connect. `resilient_connect` adds fallback paths when direct
connect fails entirely.

```rust
let (client, _shutdown) = Client::builder()
    .api_id(12345)
    .api_hash("your_hash")
    .session("bot.session")
    .probe_transport(true)      // pick best transport when direct works
    .resilient_connect(true)    // fall back via DoH + Firebase when it doesn't
    .connect()
    .await?;
```

`probe_transport` wins if direct TCP succeeds on any transport. Only if every
direct attempt fails does `resilient_connect` activate its fallback chain, which
also benefits from whatever transport was last used by the probe.
