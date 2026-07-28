# ferogram-mtsender

MTProto sender pool and retry policy for ferogram.

[![Crates.io](https://img.shields.io/crates/v/ferogram-mtsender?style=flat-square&logo=rust&logoColor=white&color=F97316)](https://crates.io/crates/ferogram-mtsender)
[![Telegram Channel](https://img.shields.io/badge/Channel-Ferogram-06B6D4?style=flat-square&logo=telegram&logoColor=white)](https://t.me/Ferogram) [![Telegram Chat](https://img.shields.io/badge/Chat-FerogramChat-06B6D4?style=flat-square&logo=telegram&logoColor=white)](https://t.me/FerogramChat)
[![docs.rs](https://img.shields.io/badge/docs.rs-ferogram--mtsender-5865F2?style=flat-square&logo=docs.rs&logoColor=white)](https://docs.rs/ferogram-mtsender)
[![License](https://img.shields.io/badge/License-MIT%20%7C%20Apache--2.0-64748B?style=flat-square)](#license)

Manages DC connections and drives the retry loop for RPC calls. `ferogram` sits on top of this; most users never touch it directly.

`ferogram` re-exports the retry types. For installation instructions see the [ferogram README](https://github.com/ankit-chaubey/ferogram).

## What it does

- `DcPool`: one `DcConnection` per DC, created on demand
- `DcConnection`: owns the sender loop for a single DC
- Retry policy trait with built-in `AutoSleep`, `NoRetries`, and `CircuitBreaker`
- `FLOOD_WAIT` and `SLOWMODE_WAIT` auto-sleep with jitter
- Exponential backoff for transient I/O errors

## Retry policies

### AutoSleep

Sleeps on `FLOOD_WAIT` and retries once on I/O errors. Default policy used by `ferogram`.

```rust
use ferogram_mtsender::AutoSleep;
use std::time::Duration;

let policy = AutoSleep {
    threshold: Duration::from_secs(60),
    io_errors_as_flood_of: Some(Duration::from_secs(1)),
};
```

### NoRetries

Propagates every error immediately.

```rust
use ferogram_mtsender::NoRetries;
let policy = NoRetries;
```

### CircuitBreaker

Trips after a set number of consecutive failures and stays open for a cooldown window.

```rust
use ferogram_mtsender::CircuitBreaker;
use std::time::Duration;

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

## Feature flags

| Flag | What it enables |
|---|---|
| `metrics` | RPC/connection instrumentation (counters, histograms, gauges) via the `metrics` crate. Off by default; a zero-cost no-op shim is used otherwise. |

## Stack position

```
ferogram
└ ferogram-mtsender  <-- here
  └ ferogram-connect
```

## License

This project is licensed under either the MIT License or Apache License 2.0, at your option. See [`LICENSE-MIT`](https://github.com/ankit-chaubey/ferogram/blob/main/LICENSE-MIT) and [`LICENSE-APACHE`](https://github.com/ankit-chaubey/ferogram/blob/main/LICENSE-APACHE) for details.

**Author:** Ankit Chaubey ([@ankit-chaubey](https://github.com/ankit-chaubey))
