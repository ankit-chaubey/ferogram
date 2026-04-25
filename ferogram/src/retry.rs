// Copyright (c) Ankit Chaubey <ankitchaubey.dev@gmail.com>
// SPDX-License-Identifier: MIT OR Apache-2.0
//
// ferogram: async Telegram MTProto client in Rust
// https://github.com/ankit-chaubey/ferogram
//
// If you use or modify this code, keep this notice at the top of your file
// and include the LICENSE-MIT or LICENSE-APACHE file from this repository:
// https://github.com/ankit-chaubey/ferogram

use std::num::NonZeroU32;
use std::ops::ControlFlow;
use std::sync::Arc;
use std::time::Duration;

use tokio::time::sleep;

use crate::errors::InvocationError;

/// Extension methods on [`crate::errors::RpcError`] for routing decisions.
impl crate::errors::RpcError {
    /// If this is a DC-migration redirect (code 303), returns the target DC id.
    ///
    /// Telegram sends these for:
    /// - `PHONE_MIGRATE_X`  : user's home DC during auth
    /// - `NETWORK_MIGRATE_X`: general redirect
    /// - `FILE_MIGRATE_X`   : file download/upload DC
    /// - `USER_MIGRATE_X`   : account migration
    ///
    /// All have `code == 303` and a numeric suffix that is the DC id.
    pub fn migrate_dc_id(&self) -> Option<i32> {
        if self.code != 303 {
            return None;
        }
        //  pattern: any *_MIGRATE_* name with a numeric value
        let is_migrate = self.name == "PHONE_MIGRATE"
            || self.name == "NETWORK_MIGRATE"
            || self.name == "FILE_MIGRATE"
            || self.name == "USER_MIGRATE"
            || self.name.ends_with("_MIGRATE");
        if is_migrate {
            // value is the DC id; fall back to DC 2 (Amsterdam) if missing
            Some(self.value.unwrap_or(2) as i32)
        } else {
            None
        }
    }
}

/// Extension on [`InvocationError`] for migrate detection.
impl InvocationError {
    /// If this error is a DC-migration redirect, returns the target DC id.
    pub fn migrate_dc_id(&self) -> Option<i32> {
        match self {
            Self::Rpc(r) => r.migrate_dc_id(),
            _ => None,
        }
    }
}

// RetryPolicy trait

/// Controls how the client reacts when an RPC call fails.
///
/// Implement this trait to provide custom flood-wait handling, circuit
/// breakers, or exponential back-off.
pub trait RetryPolicy: Send + Sync + 'static {
    /// Decide whether to retry the failed request.
    ///
    /// Return `ControlFlow::Continue(delay)` to sleep `delay` and retry.
    /// Return `ControlFlow::Break(())` to propagate `ctx.error` to the caller.
    fn should_retry(&self, ctx: &RetryContext) -> ControlFlow<(), Duration>;
}

/// Context passed to [`RetryPolicy::should_retry`] on each failure.
pub struct RetryContext {
    /// Number of times this request has failed (starts at 1).
    pub fail_count: NonZeroU32,
    /// Total time already slept for this request across all prior retries.
    pub slept_so_far: Duration,
    /// The most recent error.
    pub error: InvocationError,
}

// Built-in policies

/// Never retry: propagate every error immediately.
pub struct NoRetries;

impl RetryPolicy for NoRetries {
    fn should_retry(&self, _: &RetryContext) -> ControlFlow<(), Duration> {
        ControlFlow::Break(())
    }
}

/// Automatically sleep on `FLOOD_WAIT` and retry once on transient I/O errors.
///
/// Default retry policy. Sleeps on `FLOOD_WAIT`, backs off on I/O errors.
///
/// ```rust
/// # use ferogram::retry::AutoSleep;
/// let policy = AutoSleep {
/// threshold: std::time::Duration::from_secs(60),
/// io_errors_as_flood_of: Some(std::time::Duration::from_secs(1)),
/// };
/// ```
pub struct AutoSleep {
    /// Maximum flood-wait the library will automatically sleep through.
    ///
    /// If Telegram asks us to wait longer than this, the error is propagated.
    pub threshold: Duration,

    /// If `Some(d)`, treat the first I/O error as a `d`-second flood wait
    /// and retry once.  `None` propagates I/O errors immediately.
    pub io_errors_as_flood_of: Option<Duration>,
}

impl Default for AutoSleep {
    fn default() -> Self {
        Self {
            threshold: Duration::from_secs(60),
            io_errors_as_flood_of: Some(Duration::from_secs(1)),
        }
    }
}

/// Add deterministic ±`max_jitter_secs` jitter to `base`.
///
/// Uses a fast integer hash of `seed` (the fail count) so no `rand` crate is
/// needed. Different bots have different fail counts at any given moment, so
/// the spread is sufficient to avoid thundering-herd on simultaneous FLOOD_WAITs.
fn jitter_duration(base: Duration, seed: u32, max_jitter_secs: u64) -> Duration {
    // Murmur3-inspired finalizer.
    let h = {
        let mut v = seed as u64 ^ 0x9e37_79b9_7f4a_7c15;
        v ^= v >> 30;
        v = v.wrapping_mul(0xbf58_476d_1ce4_e5b9);
        v ^= v >> 27;
        v = v.wrapping_mul(0x94d0_49bb_1331_11eb);
        v ^= v >> 31;
        v
    };
    // Map into [-max_jitter_secs, +max_jitter_secs] in milliseconds.
    let range_ms = max_jitter_secs * 1000 * 2 + 1;
    let jitter_ms = (h % range_ms) as i64 - (max_jitter_secs * 1000) as i64;
    let base_ms = base.as_millis() as i64;
    let final_ms = (base_ms + jitter_ms).max(0) as u64;
    Duration::from_millis(final_ms)
}

impl RetryPolicy for AutoSleep {
    fn should_retry(&self, ctx: &RetryContext) -> ControlFlow<(), Duration> {
        match &ctx.error {
            // FLOOD_WAIT: sleep as long as Telegram asks, plus ±2 s jitter.
            // Jitter spreads retries across clients that all hit the same limit
            // simultaneously (e.g. after a server-side rate-limit window resets).
            InvocationError::Rpc(rpc) if rpc.code == 420 && rpc.name == "FLOOD_WAIT" => {
                let secs = rpc.value.unwrap_or(0) as u64;
                if secs <= self.threshold.as_secs() {
                    let delay = jitter_duration(Duration::from_secs(secs), ctx.fail_count.get(), 2);
                    tracing::info!("FLOOD_WAIT_{secs}: sleeping {delay:?} before retry");
                    ControlFlow::Continue(delay)
                } else {
                    ControlFlow::Break(())
                }
            }

            // SLOWMODE_WAIT: same semantics as FLOOD_WAIT; very common in
            // group bots that send messages faster than the channel's slowmode.
            InvocationError::Rpc(rpc) if rpc.code == 420 && rpc.name == "SLOWMODE_WAIT" => {
                let secs = rpc.value.unwrap_or(0) as u64;
                if secs <= self.threshold.as_secs() {
                    let delay = jitter_duration(Duration::from_secs(secs), ctx.fail_count.get(), 2);
                    tracing::info!("SLOWMODE_WAIT_{secs}: sleeping {delay:?} before retry");
                    ControlFlow::Continue(delay)
                } else {
                    ControlFlow::Break(())
                }
            }

            // Transient I/O errors: back off briefly and retry up to 3 times.
            InvocationError::Io(_) if ctx.fail_count.get() <= 3 => {
                if let Some(d) = self.io_errors_as_flood_of {
                    tracing::info!(
                        "I/O error (attempt {}): sleeping {d:?} before retry",
                        ctx.fail_count.get()
                    );
                    ControlFlow::Continue(d)
                } else {
                    ControlFlow::Break(())
                }
            }

            _ => ControlFlow::Break(()),
        }
    }
}

// RetryLoop

/// Drives the retry loop for a single RPC call.
///
/// Create one per call, then call `advance` after every failure.
///
/// ```rust,ignore
/// let mut rl = RetryLoop::new(Arc::clone(&self.inner.retry_policy));
/// loop {
/// match self.do_rpc_call(req).await {
///     Ok(body) => return Ok(body),
///     Err(e)   => rl.advance(e).await?,
/// }
/// }
/// ```
///
/// `advance` either:
/// - sleeps the required duration and returns `Ok(())` → caller should retry, or
/// - returns `Err(e)` → caller should propagate.
///
/// This is the single source of truth; previously the same loop was
/// copy-pasted into `rpc_call_raw`, `rpc_write`, and the reconnect path.
pub struct RetryLoop {
    policy: Arc<dyn RetryPolicy>,
    ctx: RetryContext,
}

impl RetryLoop {
    pub fn new(policy: Arc<dyn RetryPolicy>) -> Self {
        Self {
            policy,
            ctx: RetryContext {
                fail_count: NonZeroU32::new(1).unwrap(),
                slept_so_far: Duration::default(),
                error: InvocationError::Dropped,
            },
        }
    }

    /// Record a failure and either sleep+return-Ok (retry) or return-Err (give up).
    ///
    /// Mutates `self` to track cumulative state across retries.
    pub async fn advance(&mut self, err: InvocationError) -> Result<(), InvocationError> {
        self.ctx.error = err;
        match self.policy.should_retry(&self.ctx) {
            ControlFlow::Continue(delay) => {
                sleep(delay).await;
                self.ctx.slept_so_far += delay;
                // saturating_add: if somehow we overflow NonZeroU32, clamp at MAX
                self.ctx.fail_count = self.ctx.fail_count.saturating_add(1);
                Ok(())
            }
            ControlFlow::Break(()) => {
                // Move the error out so the caller doesn't have to clone it
                Err(std::mem::replace(
                    &mut self.ctx.error,
                    InvocationError::Dropped,
                ))
            }
        }
    }
}

// CircuitBreaker

/// Internal state of a [`CircuitBreaker`].
#[derive(Debug)]
enum CbState {
    /// Normal operation: counting consecutive failures.
    Closed { consecutive_failures: u32 },
    /// Breaker tripped: all calls rejected until cooldown expires.
    Open { tripped_at: std::time::Instant },
}

/// A [`RetryPolicy`] that stops retrying after `threshold` consecutive
/// failures and stays silent for a `cooldown` window before resetting.
///
/// # States
/// - **Closed** (normal): forwards calls, increments a failure counter on
///   each error, and applies an exponential back-off up to `threshold − 1`
///   attempts.  On the `threshold`-th consecutive failure the breaker trips.
/// - **Open** (tripped): rejects every call immediately (`Break`) for the
///   duration of `cooldown`.
/// - **Reset**: once `cooldown` has elapsed the breaker closes again and
///   the failure counter resets to zero.
///
/// Because [`RetryPolicy`] has no success callback the breaker cannot
/// distinguish a successful probe from a clean run; the counter simply
/// resets when the cooldown expires.  For a full half-open probe you can
/// wrap `CircuitBreaker` in a custom `RetryPolicy`.
///
/// # Example
/// ```rust
/// use ferogram::retry::CircuitBreaker;
/// use std::time::Duration;
///
/// // Trip after 5 consecutive errors; stay open for 30 s.
/// let policy = CircuitBreaker::new(5, Duration::from_secs(30));
/// ```
pub struct CircuitBreaker {
    /// Number of consecutive failures before the breaker trips.
    threshold: u32,
    /// How long the breaker stays open before resetting.
    cooldown: Duration,
    state: std::sync::Mutex<CbState>,
}

impl CircuitBreaker {
    /// Create a new `CircuitBreaker`.
    ///
    /// - `threshold`: failures before the breaker trips (minimum 1).
    /// - `cooldown`: how long the breaker stays open.
    pub fn new(threshold: u32, cooldown: Duration) -> Self {
        assert!(
            threshold >= 1,
            "CircuitBreaker threshold must be at least 1"
        );
        Self {
            threshold,
            cooldown,
            state: std::sync::Mutex::new(CbState::Closed {
                consecutive_failures: 0,
            }),
        }
    }
}

impl RetryPolicy for CircuitBreaker {
    fn should_retry(&self, _ctx: &RetryContext) -> ControlFlow<(), Duration> {
        let mut state = self.state.lock().unwrap();
        match &*state {
            CbState::Open { tripped_at } => {
                if tripped_at.elapsed() >= self.cooldown {
                    // Cooldown expired: reset to Closed, allow retry with small delay.
                    *state = CbState::Closed {
                        consecutive_failures: 1,
                    };
                    ControlFlow::Continue(Duration::from_millis(200))
                } else {
                    // Still open: reject immediately.
                    ControlFlow::Break(())
                }
            }
            CbState::Closed {
                consecutive_failures,
            } => {
                let new_count = consecutive_failures + 1;
                if new_count >= self.threshold {
                    tracing::warn!(
                        "[ferogram] CircuitBreaker tripped after {new_count} consecutive failures"
                    );
                    *state = CbState::Open {
                        tripped_at: std::time::Instant::now(),
                    };
                    ControlFlow::Break(())
                } else {
                    // Exponential back-off: 200 ms × 2^(n-1), capped at ~3 s.
                    let backoff_ms = 200u64 * (1u64 << new_count.saturating_sub(1).min(4));
                    *state = CbState::Closed {
                        consecutive_failures: new_count,
                    };
                    ControlFlow::Continue(Duration::from_millis(backoff_ms))
                }
            }
        }
    }
}

// Tests

#[cfg(test)]
mod tests {
    use super::*;
    use std::io;

    fn flood(secs: u32) -> InvocationError {
        InvocationError::Rpc(crate::errors::RpcError {
            code: 420,
            name: "FLOOD_WAIT".into(),
            value: Some(secs),
        })
    }

    fn io_err() -> InvocationError {
        InvocationError::Io(io::Error::new(io::ErrorKind::ConnectionReset, "reset"))
    }

    fn rpc(code: i32, name: &str, value: Option<u32>) -> InvocationError {
        InvocationError::Rpc(crate::errors::RpcError {
            code,
            name: name.into(),
            value,
        })
    }

    // NoRetries

    #[test]
    fn no_retries_always_breaks() {
        let policy = NoRetries;
        let ctx = RetryContext {
            fail_count: NonZeroU32::new(1).unwrap(),
            slept_so_far: Duration::default(),
            error: flood(10),
        };
        assert!(matches!(policy.should_retry(&ctx), ControlFlow::Break(())));
    }

    // AutoSleep

    #[test]
    fn autosleep_retries_flood_under_threshold() {
        let policy = AutoSleep::default(); // threshold = 60s
        let ctx = RetryContext {
            fail_count: NonZeroU32::new(1).unwrap(),
            slept_so_far: Duration::default(),
            error: flood(30),
        };
        match policy.should_retry(&ctx) {
            // Jitter of ±2s is applied; accept 28..=32 s.
            ControlFlow::Continue(d) => {
                let secs = d.as_secs_f64();
                assert!(
                    secs >= 28.0 && secs <= 32.0,
                    "expected 28-32s delay (jitter), got {secs:.3}s"
                );
            }
            other => panic!("expected Continue, got {other:?}"),
        }
    }

    #[test]
    fn autosleep_breaks_flood_over_threshold() {
        let policy = AutoSleep::default(); // threshold = 60s
        let ctx = RetryContext {
            fail_count: NonZeroU32::new(1).unwrap(),
            slept_so_far: Duration::default(),
            error: flood(120),
        };
        assert!(matches!(policy.should_retry(&ctx), ControlFlow::Break(())));
    }

    #[test]
    fn autosleep_second_flood_retry_is_honoured() {
        let policy = AutoSleep::default();
        let ctx = RetryContext {
            fail_count: NonZeroU32::new(2).unwrap(),
            slept_so_far: Duration::from_secs(30),
            error: flood(30),
        };
        match policy.should_retry(&ctx) {
            // Jitter of ±2s; accept 28..=32 s.
            ControlFlow::Continue(d) => {
                let secs = d.as_secs_f64();
                assert!(
                    secs >= 28.0 && secs <= 32.0,
                    "expected 28-32s on second FLOOD_WAIT, got {secs:.3}s"
                );
            }
            other => panic!("expected Continue on second FLOOD_WAIT, got {other:?}"),
        }
    }

    #[test]
    fn autosleep_retries_io_once() {
        let policy = AutoSleep::default();
        let ctx = RetryContext {
            fail_count: NonZeroU32::new(1).unwrap(),
            slept_so_far: Duration::default(),
            error: io_err(),
        };
        match policy.should_retry(&ctx) {
            ControlFlow::Continue(d) => assert_eq!(d, Duration::from_secs(1)),
            other => panic!("expected Continue, got {other:?}"),
        }
    }

    #[test]
    fn autosleep_no_second_io_retry() {
        let policy = AutoSleep::default();
        let ctx = RetryContext {
            fail_count: NonZeroU32::new(2).unwrap(),
            slept_so_far: Duration::from_secs(1),
            error: io_err(),
        };
        assert!(matches!(policy.should_retry(&ctx), ControlFlow::Break(())));
    }

    #[test]
    fn autosleep_breaks_other_rpc() {
        let policy = AutoSleep::default();
        let ctx = RetryContext {
            fail_count: NonZeroU32::new(1).unwrap(),
            slept_so_far: Duration::default(),
            error: rpc(400, "BAD_REQUEST", None),
        };
        assert!(matches!(policy.should_retry(&ctx), ControlFlow::Break(())));
    }

    // RpcError::migrate_dc_id

    #[test]
    fn migrate_dc_id_detected() {
        let e = crate::errors::RpcError {
            code: 303,
            name: "PHONE_MIGRATE".into(),
            value: Some(5),
        };
        assert_eq!(e.migrate_dc_id(), Some(5));
    }

    #[test]
    fn network_migrate_detected() {
        let e = crate::errors::RpcError {
            code: 303,
            name: "NETWORK_MIGRATE".into(),
            value: Some(3),
        };
        assert_eq!(e.migrate_dc_id(), Some(3));
    }

    #[test]
    fn file_migrate_detected() {
        let e = crate::errors::RpcError {
            code: 303,
            name: "FILE_MIGRATE".into(),
            value: Some(4),
        };
        assert_eq!(e.migrate_dc_id(), Some(4));
    }

    #[test]
    fn non_migrate_is_none() {
        let e = crate::errors::RpcError {
            code: 420,
            name: "FLOOD_WAIT".into(),
            value: Some(30),
        };
        assert_eq!(e.migrate_dc_id(), None);
    }

    #[test]
    fn migrate_falls_back_to_dc2_when_no_value() {
        let e = crate::errors::RpcError {
            code: 303,
            name: "PHONE_MIGRATE".into(),
            value: None,
        };
        assert_eq!(e.migrate_dc_id(), Some(2));
    }

    // RetryLoop

    #[tokio::test]
    async fn retry_loop_gives_up_on_no_retries() {
        let mut rl = RetryLoop::new(Arc::new(NoRetries));
        let err = rpc(400, "SOMETHING_WRONG", None);
        let result = rl.advance(err).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn retry_loop_increments_fail_count() {
        let mut rl = RetryLoop::new(Arc::new(AutoSleep {
            threshold: Duration::from_secs(60),
            io_errors_as_flood_of: Some(Duration::from_millis(1)),
        }));
        assert!(rl.advance(io_err()).await.is_ok());
        assert!(rl.advance(io_err()).await.is_err());
    }

    // CircuitBreaker

    #[test]
    fn circuit_breaker_trips_after_threshold() {
        let cb = CircuitBreaker::new(3, Duration::from_secs(60));
        let ctx = |n: u32| RetryContext {
            fail_count: NonZeroU32::new(n).unwrap(),
            slept_so_far: Duration::default(),
            error: rpc(500, "INTERNAL", None),
        };
        // First two failures: Continue (backoff)
        assert!(matches!(cb.should_retry(&ctx(1)), ControlFlow::Continue(_)));
        assert!(matches!(cb.should_retry(&ctx(2)), ControlFlow::Continue(_)));
        // Third: trips the breaker → Break
        assert!(matches!(cb.should_retry(&ctx(3)), ControlFlow::Break(())));
        // Subsequent calls while open → Break immediately
        assert!(matches!(cb.should_retry(&ctx(4)), ControlFlow::Break(())));
    }

    #[test]
    fn circuit_breaker_resets_after_cooldown() {
        let cb = CircuitBreaker::new(2, Duration::from_millis(10));
        let ctx = |n: u32| RetryContext {
            fail_count: NonZeroU32::new(n).unwrap(),
            slept_so_far: Duration::default(),
            error: rpc(500, "INTERNAL", None),
        };
        // Trip the breaker
        assert!(matches!(cb.should_retry(&ctx(1)), ControlFlow::Continue(_)));
        assert!(matches!(cb.should_retry(&ctx(2)), ControlFlow::Break(())));
        // Wait for cooldown
        std::thread::sleep(Duration::from_millis(20));
        // After cooldown: breaker resets → Continue again
        assert!(matches!(cb.should_retry(&ctx(1)), ControlFlow::Continue(_)));
    }
}
