# Connection Restart Policy

`ConnectionRestartPolicy` controls whether ferogram automatically re-establishes
a dropped connection and how long it waits between attempts.

The policy is set via `.restart_policy()` on the builder and defaults to
`NeverRestart`.

---

## Built-in policies

### `NeverRestart` (default)

Does not restart. When the underlying TCP connection drops, the event loop
exits and the shutdown signal fires. Your code is responsible for reconnecting
or exiting.

```rust
use std::sync::Arc;
use ferogram::NeverRestart;

let (client, _shutdown) = Client::builder()
    .api_id(12345)
    .api_hash("your_hash")
    .session("bot.session")
    .restart_policy(Arc::new(NeverRestart))   // this is already the default
    .connect()
    .await?;
```

Use `NeverRestart` when you manage the process lifecycle externally (systemd,
supervisord, Docker restart policies) or when you want the process to crash
loudly on a disconnect rather than silently loop.

---

### `FixedInterval`

Restarts the connection after a fixed duration following a drop. The interval
is measured from the moment the drop is detected to the next connect attempt.

```rust
use std::sync::Arc;
use std::time::Duration;
use ferogram::FixedInterval;

let (client, _shutdown) = Client::builder()
    .api_id(12345)
    .api_hash("your_hash")
    .session("bot.session")
    .restart_policy(Arc::new(FixedInterval {
        interval: Duration::from_secs(5),
    }))
    .connect()
    .await?;
```

Common values:

| Interval | Use case |
|---|---|
| `Duration::from_secs(1)` | Low-latency bots where a 1-second gap is acceptable |
| `Duration::from_secs(5)` | General-purpose bots, reasonable default |
| `Duration::from_secs(30)` | Rate-limited or metered connections |

---

## Custom policy

Implement `ConnectionRestartPolicy` to add exponential backoff, jitter, or
circuit-breaker logic.

```rust
use std::sync::Arc;
use std::time::Duration;
use std::sync::atomic::{AtomicU32, Ordering};
use ferogram::ConnectionRestartPolicy;

struct ExponentialBackoff {
    attempt: AtomicU32,
    base_ms: u64,
    max_ms: u64,
}

impl ExponentialBackoff {
    fn new(base_ms: u64, max_ms: u64) -> Self {
        Self {
            attempt: AtomicU32::new(0),
            base_ms,
            max_ms,
        }
    }
}

impl ConnectionRestartPolicy for ExponentialBackoff {
    fn restart_interval(&self) -> Option<Duration> {
        let n = self.attempt.fetch_add(1, Ordering::Relaxed);
        let ms = (self.base_ms * (1u64 << n.min(10))).min(self.max_ms);
        Some(Duration::from_millis(ms))
    }
}

let (client, _shutdown) = Client::builder()
    .api_id(12345)
    .api_hash("your_hash")
    .session("bot.session")
    .restart_policy(Arc::new(ExponentialBackoff::new(500, 60_000)))
    .connect()
    .await?;
```

### Trait definition

```rust
pub trait ConnectionRestartPolicy: Send + Sync + 'static {
    /// Return `Some(duration)` to restart after that delay, or `None` to not restart.
    fn restart_interval(&self) -> Option<Duration>;
}
```

`restart_interval` is called each time a connection drop is detected. Return
`None` at any point to stop restarting (e.g. after N attempts).

---

## Scheduled periodic restarts

`FixedInterval` can be used to schedule **periodic** connection refreshes even
when the connection has not dropped, by setting the interval to a wall-clock
cycle. This is useful for very long-lived processes where you want to rotate
the TCP session once a day to avoid silent stale connections.

```rust
use ferogram::FixedInterval;

// Restart the connection every 12 hours
.restart_policy(Arc::new(FixedInterval {
    interval: Duration::from_secs(12 * 60 * 60),
}))
```

Note: a scheduled restart drops the TCP connection cleanly and reconnects.
Any in-flight RPC calls at that moment will return an error and need to be
retried by your application. Pair this with `.catch_up(true)` to replay any
updates missed during the restart window.

```rust
.catch_up(true)
.restart_policy(Arc::new(FixedInterval {
    interval: Duration::from_secs(12 * 60 * 60),
}))
```

---

## Related

- [Retry & Flood Wait](./retry.md) - how `FLOOD_WAIT` and other rate-limit errors
  are handled independently of connection restart
- [Configuration](../config.md) - full builder reference
