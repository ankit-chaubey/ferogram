# ferogram-mtsender

MTProto sender pool and retry policy for ferogram.

[![Crates.io](https://img.shields.io/crates/v/ferogram-mtsender?color=fc8d62)](https://crates.io/crates/ferogram-mtsender)
[![Telegram](https://img.shields.io/badge/community-%40FerogramChat-2CA5E0?logo=telegram)](https://t.me/FerogramChat) [![Channel](https://img.shields.io/badge/channel-%40Ferogram-2CA5E0?logo=telegram)](https://t.me/Ferogram)
[![docs.rs](https://img.shields.io/badge/docs.rs-ferogram--mtsender-5865F2)](https://docs.rs/ferogram-mtsender)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](#license)

Manages DC connections and drives the retry loop for RPC calls. `ferogram` sits on top of this; most users never touch it directly.

`ferogram` re-exports the retry types. Existing code needs no changes.

## Installation

```toml
[dependencies]
ferogram-mtsender = "0.4.0"
```

## What it does

- `DcPool`: one `DcConnection` per DC, created on demand
- `DcConnection`: owns the sender loop for a single DC
- Retry policy trait with built-in `AutoSleep`, `NoRetries`, and `CircuitBreaker`
- `FLOOD_WAIT` and `SLOWMODE_WAIT` auto-sleep with jitter
- Exponential backoff for transient I/O errors

## Retry policies

### AutoSleep

Sleeps on `FLOOD_WAIT` and retries once on I/O errors. This is the default policy used by `ferogram`.

```rust
use ferogram_mtsender::AutoSleep;
use std::time::Duration;

let policy = AutoSleep {
    threshold: Duration::from_secs(60), // sleep through flood waits up to 60s
    io_errors_as_flood_of: Some(Duration::from_secs(1)),
};
```

### NoRetries

Propagates every error immediately without sleeping.

```rust
use ferogram_mtsender::NoRetries;

let policy = NoRetries;
```

### CircuitBreaker

Trips after a set number of consecutive failures and stays open for a cooldown window.

```rust
use ferogram_mtsender::CircuitBreaker;
use std::time::Duration;

// Trip after 5 consecutive errors, stay open for 30s
let policy = CircuitBreaker::new(5, Duration::from_secs(30));
```

### Custom policy

```rust
use ferogram_mtsender::{RetryPolicy, RetryContext};
use std::ops::ControlFlow;
use std::time::Duration;

struct MyPolicy;

impl RetryPolicy for MyPolicy {
    fn should_retry(&self, ctx: &RetryContext) -> ControlFlow<(), Duration> {
        if ctx.fail_count.get() < 3 {
            ControlFlow::Continue(Duration::from_secs(1))
        } else {
            ControlFlow::Break(())
        }
    }
}
```

## Stack position

```
ferogram
└ ferogram-mtsender  <-- here
  └ ferogram-connect
```

## License

MIT or Apache-2.0, at your option. See [LICENSE-MIT](../LICENSE-MIT) and [LICENSE-APACHE](../LICENSE-APACHE).

**Ankit Chaubey** - [github.com/ankit-chaubey](https://github.com/ankit-chaubey)
