# Retry & Flood Wait

Telegram's rate limiting system sends `FLOOD_WAIT_X` errors when you call the API too frequently. `X` is the number of seconds you must wait before retrying.

## Default behaviour: AutoSleep

By default, `ferogram` uses `AutoSleep`: it transparently sleeps for the required duration, then retries. Your code never sees the error.

```rust
use ferogram::{Config, AutoSleep};
use std::sync::Arc;

let (client, _shutdown) = Client::connect(Config {
    retry_policy: Arc::new(AutoSleep::default()),
    ..Default::default()
}).await?;
```

This is the default. You don't need to set it explicitly.

## NoRetries: propagate immediately

If you want to handle FLOOD_WAIT yourself:

```rust
use ferogram::NoRetries;

let (client, _shutdown) = Client::connect(Config {
    retry_policy: Arc::new(NoRetries),
    ..Default::default()
}).await?;
```

Then in your code:

```rust
use ferogram::{InvocationError, RpcError};
use tokio::time::{sleep, Duration};

loop {
    match client.send_message("@user", "hi").await {
        Ok(_) => break,
        Err(InvocationError::Rpc(RpcError { code: 420, ref message, .. })) => {
            let secs: u64 = message
                .strip_prefix("FLOOD_WAIT_")
                .and_then(|s| s.parse().ok())
                .unwrap_or(60);
            println!("Rate limited. Waiting {secs}s");
            sleep(Duration::from_secs(secs)).await;
        }
        Err(e) => return Err(e.into()),
    }
}
```

## Custom retry policy

Implement `RetryPolicy` for full control: cap the wait, log, or give up after N attempts:

```rust
use ferogram::{RetryPolicy, RetryContext};
use std::ops::ControlFlow;
use std::time::Duration;

struct CappedSleep {
    max_wait_secs: u64,
    max_attempts:  u32,
}

impl RetryPolicy for CappedSleep {
    fn should_retry(&self, ctx: &RetryContext) -> ControlFlow<(), Duration> {
        if ctx.attempt() >= self.max_attempts {
            log::warn!("Giving up after {} attempts", ctx.attempt());
            return ControlFlow::Break(());
        }

        let wait = ctx.flood_wait_secs();
        if wait > self.max_wait_secs {
            log::warn!("FLOOD_WAIT too long ({wait}s), giving up");
            return ControlFlow::Break(());
        }

        log::info!("FLOOD_WAIT {wait}s (attempt {})", ctx.attempt());
        ControlFlow::Continue(Duration::from_secs(wait))
    }
}

let (client, _shutdown) = Client::connect(Config {
    retry_policy: Arc::new(CappedSleep {
        max_wait_secs: 30,
        max_attempts:  3,
    }),
    ..Default::default()
}).await?;
```

## RetryContext fields

| Method | Returns | Description |
|---|---|---|
| `ctx.flood_wait_secs()` | `u64` | How long Telegram wants you to wait |
| `ctx.attempt()` | `u32` | How many times this call has been retried |
| `ctx.error_message()` | `&str` | The raw error message string |

## Avoiding flood waits

- Add small delays between bulk operations: `tokio::time::sleep(Duration::from_millis(100)).await`
- Cache peer resolutions: don't resolve the same username repeatedly
- Don't send messages in tight loops
- Bots have more generous limits than user accounts
- Some methods (e.g. `GetHistory`) have separate, more generous limits
- Use `send_message` for a single message; avoid rapid-fire parallel calls
