// Copyright (c) Ankit Chaubey <ankitchaubey.dev@gmail.com>
// SPDX-License-Identifier: MIT OR Apache-2.0
//
// ferogram: async Telegram MTProto client in Rust
// https://github.com/ankit-chaubey/ferogram
//
// Based on layer: https://github.com/ankit-chaubey/layer
// Follows official Telegram client behaviour (tdesktop, TDLib).
//
// If you use or modify this code, keep this notice at the top of your file
// and include the LICENSE-MIT or LICENSE-APACHE file from this repository:
// https://github.com/ankit-chaubey/ferogram

//! Retry policies for handling `FLOOD_WAIT`, transient I/O errors, and DC-migration redirects.

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

impl RetryPolicy for AutoSleep {
    fn should_retry(&self, ctx: &RetryContext) -> ControlFlow<(), Duration> {
        match &ctx.error {
            // FLOOD_WAIT: sleep exactly as long as Telegram asks, for every
            // occurrence up to threshold. Removing the fail_count==1 guard
            // means a second consecutive FLOOD_WAIT is also honoured rather
            // than propagated immediately.
            InvocationError::Rpc(rpc) if rpc.code == 420 && rpc.name == "FLOOD_WAIT" => {
                let secs = rpc.value.unwrap_or(0) as u64;
                if secs <= self.threshold.as_secs() {
                    tracing::info!("FLOOD_WAIT_{secs}: sleeping before retry");
                    ControlFlow::Continue(Duration::from_secs(secs))
                } else {
                    ControlFlow::Break(())
                }
            }

            // SLOWMODE_WAIT: same semantics as FLOOD_WAIT; very common in
            // group bots that send messages faster than the channel's slowmode.
            InvocationError::Rpc(rpc) if rpc.code == 420 && rpc.name == "SLOWMODE_WAIT" => {
                let secs = rpc.value.unwrap_or(0) as u64;
                if secs <= self.threshold.as_secs() {
                    tracing::info!("SLOWMODE_WAIT_{secs}: sleeping before retry");
                    ControlFlow::Continue(Duration::from_secs(secs))
                } else {
                    ControlFlow::Break(())
                }
            }

            // Transient I/O errors: back off briefly and retry once.
            InvocationError::Io(_) if ctx.fail_count.get() == 1 => {
                if let Some(d) = self.io_errors_as_flood_of {
                    tracing::info!("I/O error: sleeping {d:?} before retry");
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
            ControlFlow::Continue(d) => assert_eq!(d, Duration::from_secs(30)),
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
    fn autosleep_no_second_flood_retry() {
        let policy = AutoSleep::default();
        // fail_count == 2 → already retried once, should give up
        let ctx = RetryContext {
            fail_count: NonZeroU32::new(2).unwrap(),
            slept_so_far: Duration::from_secs(30),
            error: flood(30),
        };
        assert!(matches!(policy.should_retry(&ctx), ControlFlow::Break(())));
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
}
