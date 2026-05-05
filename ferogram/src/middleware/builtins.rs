// Copyright (c) Ankit Chaubey <ankitchaubey.dev@gmail.com>
//
// ferogram: async Telegram MTProto client in Rust
// https://github.com/ankit-chaubey/ferogram
//
// Licensed under either the MIT License or the Apache License 2.0.
// See the LICENSE-MIT or LICENSE-APACHE file in this repository:
// https://github.com/ankit-chaubey/ferogram
//
// Feel free to use, modify, and share this code.
// Please keep this notice when redistributing.

use std::fmt;
use std::sync::Arc;
use std::time::{Duration, Instant};

use dashmap::DashMap;

use super::core::{BoxFuture, DispatchError, Middleware, Next};
use crate::update::Update;

// TracingMiddleware

/// Logs every update with its type and processing time via `tracing`.
///
/// # Example
///
/// ```rust,no_run
/// use ferogram::filters::Dispatcher;
/// use ferogram::middleware::TracingMiddleware;
///
/// let mut dp = Dispatcher::new();
/// dp.middleware(TracingMiddleware::new());
/// ```
#[derive(Debug, Clone, Default)]
pub struct TracingMiddleware;

impl TracingMiddleware {
    /// Create a new [`TracingMiddleware`].
    pub fn new() -> Self {
        Self
    }
}

impl Middleware for TracingMiddleware {
    fn call(&self, update: Update, next: Next) -> BoxFuture {
        Box::pin(async move {
            let kind = update_kind(&update);
            let start = Instant::now();
            tracing::debug!(update_kind = kind, "dispatching update");

            let result = next.run(update).await;
            let elapsed = start.elapsed();

            match &result {
                Ok(()) => tracing::debug!(update_kind = kind, elapsed = ?elapsed, "update handled"),
                Err(e) => {
                    tracing::error!(update_kind = kind, elapsed = ?elapsed, error = %e, "dispatch error")
                }
            }

            result
        })
    }
}

fn update_kind(update: &Update) -> &'static str {
    match update {
        Update::NewMessage(_) => "NewMessage",
        Update::MessageEdited(_) => "MessageEdited",
        Update::MessageDeleted(_) => "MessageDeleted",
        Update::CallbackQuery(_) => "CallbackQuery",
        Update::InlineQuery(_) => "InlineQuery",
        Update::InlineSend(_) => "InlineSend",
        Update::UserStatus(_) => "UserStatus",
        Update::UserTyping(_) => "UserTyping",
        Update::ParticipantUpdate(_) => "ParticipantUpdate",
        Update::JoinRequest(_) => "JoinRequest",
        Update::MessageReaction(_) => "MessageReaction",
        Update::PollVote(_) => "PollVote",
        Update::BotStopped(_) => "BotStopped",
        Update::ShippingQuery(_) => "ShippingQuery",
        Update::PreCheckoutQuery(_) => "PreCheckoutQuery",
        Update::ChatBoost(_) => "ChatBoost",
        Update::Raw(_) => "Raw",
    }
}

// RateLimitMiddleware

/// Per-user rate limiting middleware.
///
/// Counts incoming message updates per user. When a user exceeds
/// `max_calls` within `window`, the update is silently dropped.
///
/// Non-message updates (callback queries, inline queries, etc.) are always
/// passed through unchanged.
///
/// # Example
///
/// ```rust,no_run
/// use ferogram::filters::Dispatcher;
/// use ferogram::middleware::RateLimitMiddleware;
/// use std::time::Duration;
///
/// let mut dp = Dispatcher::new();
/// dp.middleware(RateLimitMiddleware::new(5, Duration::from_secs(1)));
/// ```
#[derive(Clone)]
pub struct RateLimitMiddleware {
    inner: Arc<RateLimitInner>,
}

struct RateLimitInner {
    max_calls: u32,
    window: Duration,
    /// user_id -> (call count in current window, window start)
    state: DashMap<i64, (u32, Instant)>,
}

impl RateLimitMiddleware {
    /// Create a new rate limiter.
    pub fn new(max_calls: u32, window: Duration) -> Self {
        Self {
            inner: Arc::new(RateLimitInner {
                max_calls,
                window,
                state: DashMap::new(),
            }),
        }
    }

    /// Return the number of users currently tracked.
    pub fn tracked_users(&self) -> usize {
        self.inner.state.len()
    }
}

impl Middleware for RateLimitMiddleware {
    fn call(&self, update: Update, next: Next) -> BoxFuture {
        let inner = Arc::clone(&self.inner);
        Box::pin(async move {
            let sender_id = match &update {
                Update::NewMessage(m) | Update::MessageEdited(m) => m.sender_user_id(),
                _ => return next.run(update).await,
            };

            let user_id = match sender_id {
                Some(id) => id,
                None => return next.run(update).await,
            };

            let now = Instant::now();
            let allowed = {
                let mut entry = inner.state.entry(user_id).or_insert((0, now));
                let (count, window_start) = &mut *entry;

                if now.duration_since(*window_start) >= inner.window {
                    *count = 1;
                    *window_start = now;
                    true
                } else if *count < inner.max_calls {
                    *count += 1;
                    true
                } else {
                    false
                }
            };

            if allowed {
                next.run(update).await
            } else {
                tracing::debug!(user_id, "rate limit exceeded - update dropped");
                Ok(())
            }
        })
    }
}

impl fmt::Debug for RateLimitMiddleware {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RateLimitMiddleware")
            .field("max_calls", &self.inner.max_calls)
            .field("window", &self.inner.window)
            .finish()
    }
}

// PanicRecoveryMiddleware

/// Catches panics in downstream middleware and handlers, converting them to
/// [`DispatchError`] so the bot stays alive.
///
/// # How it works
///
/// Runs the downstream chain inside a `tokio::task::spawn` and inspects the
/// `JoinError` on completion. This reliably catches panics across `.await`
/// yield points, which `std::panic::catch_unwind` cannot do.
///
/// # Example
///
/// ```rust,no_run
/// use ferogram::filters::Dispatcher;
/// use ferogram::middleware::PanicRecoveryMiddleware;
///
/// let mut dp = Dispatcher::new();
/// dp.middleware(PanicRecoveryMiddleware::new());
/// ```
#[derive(Debug, Clone, Default)]
pub struct PanicRecoveryMiddleware;

impl PanicRecoveryMiddleware {
    /// Create a new `PanicRecoveryMiddleware`.
    pub fn new() -> Self {
        Self
    }
}

impl Middleware for PanicRecoveryMiddleware {
    fn call(&self, update: Update, next: Next) -> BoxFuture {
        Box::pin(async move {
            let join_handle = tokio::task::spawn(async move { next.run(update).await });

            match join_handle.await {
                Ok(result) => result,
                Err(join_error) if join_error.is_panic() => {
                    let msg = join_error
                        .into_panic()
                        .downcast_ref::<&str>()
                        .map(|s| s.to_string())
                        .or(None)
                        .unwrap_or_else(|| "unknown panic payload".to_string());

                    tracing::error!(
                        panic = %msg,
                        "handler panicked - caught by PanicRecoveryMiddleware"
                    );
                    Err(DispatchError::msg(format!("handler panicked: {msg}")))
                }
                Err(join_error) => {
                    tracing::warn!("dispatch task cancelled during shutdown");
                    Err(DispatchError::msg(format!(
                        "dispatch task cancelled: {join_error}"
                    )))
                }
            }
        })
    }
}
