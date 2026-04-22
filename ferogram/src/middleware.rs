// Copyright (c) Ankit Chaubey <ankitchaubey.dev@gmail.com>
// SPDX-License-Identifier: MIT OR Apache-2.0
//
// ferogram: async Telegram MTProto client in Rust
// https://github.com/ankit-chaubey/ferogram
//
// If you use or modify this code, keep this notice at the top of your file
// and include the LICENSE-MIT or LICENSE-APACHE file from this repository:
// https://github.com/ankit-chaubey/ferogram

use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::{Duration, Instant};

use dashmap::DashMap;

use crate::update::Update;

// DispatchError / DispatchResult

/// An error produced by middleware or the handler endpoint.
///
/// Wraps any `Box<dyn Error + Send + Sync>` source. Construct via `From` or
/// [`DispatchError::msg`].
#[derive(Debug)]
pub struct DispatchError {
    message: String,
    source: Option<Box<dyn std::error::Error + Send + Sync>>,
}

impl DispatchError {
    /// Create an error with a plain string message.
    pub fn msg(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            source: None,
        }
    }

    /// Create an error wrapping an existing error value.
    pub fn wrap(source: impl std::error::Error + Send + Sync + 'static) -> Self {
        Self {
            message: source.to_string(),
            source: Some(Box::new(source)),
        }
    }
}

impl fmt::Display for DispatchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for DispatchError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.source.as_ref().map(|e| e.as_ref() as _)
    }
}

/// The result type returned by [`Middleware::call`] and [`Next::run`].
pub type DispatchResult = Result<(), DispatchError>;

/// A pinned, boxed, `Send + 'static` future resolving to [`DispatchResult`].
pub type BoxFuture = Pin<Box<dyn Future<Output = DispatchResult> + Send + 'static>>;

// Next

/// A handle to the remainder of the middleware chain.
///
/// Call [`Next::run`] from inside [`Middleware::call`] to invoke the next
/// middleware (or the handler endpoint if this is the last middleware).
///
/// `Next` is cheaply cloneable (all internals are `Arc`-backed).
#[derive(Clone)]
pub struct Next {
    chain: Arc<[Arc<dyn Middleware>]>,
    index: usize,
    endpoint: Arc<dyn Fn(Update) -> BoxFuture + Send + Sync>,
}

impl Next {
    pub(crate) fn new(
        chain: Arc<[Arc<dyn Middleware>]>,
        endpoint: Arc<dyn Fn(Update) -> BoxFuture + Send + Sync>,
    ) -> Self {
        Self {
            chain,
            index: 0,
            endpoint,
        }
    }

    /// Pass the update to the next layer in the chain.
    ///
    /// If no more middleware remain, invokes the handler endpoint directly.
    pub fn run(self, update: Update) -> BoxFuture {
        Box::pin(async move {
            if self.index < self.chain.len() {
                let mw = Arc::clone(&self.chain[self.index]);
                let next = Next {
                    chain: Arc::clone(&self.chain),
                    index: self.index + 1,
                    endpoint: Arc::clone(&self.endpoint),
                };
                mw.call(update, next).await
            } else {
                (self.endpoint)(update).await
            }
        })
    }
}

impl fmt::Debug for Next {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Next")
            .field("index", &self.index)
            .field("chain_len", &self.chain.len())
            .finish()
    }
}

// Middleware trait

/// An async interceptor in the dispatcher pipeline.
///
/// Middleware is called for **every** update regardless of which handler (if
/// any) matches. This makes it the correct place for cross-cutting concerns
/// such as:
///
/// - Structured logging and tracing
/// - Authentication / authorization gates
/// - Rate limiting and flood control
/// - Audit trails
/// - Panic recovery
/// - Custom metrics
///
/// # Object safety and `'static` futures
///
/// `Middleware` is object-safe (`dyn Middleware` is valid). Because the
/// middleware chain is stored behind `Arc` and the futures must be `'static`,
/// any data accessed inside `call` must be cloned from `Arc`-backed fields
/// rather than borrowed from `&self`. See the module-level example.
pub trait Middleware: Send + Sync + 'static {
    /// Intercept an update and optionally pass it to the rest of the chain.
    ///
    /// The returned future **must** be `'static` - clone any `&self` data
    /// into `Arc`s before the `Box::pin(async move { ... })`.
    fn call(&self, update: Update, next: Next) -> BoxFuture;
}

/// Blanket implementation so that closures can be used directly as middleware:
///
/// ```rust,no_run
/// # use ferogram::filters::Dispatcher;
/// let mut dp = Dispatcher::new();
/// dp.middleware(|upd, next| async move {
///     tracing::debug!(?upd, "incoming update");
///     next.run(upd).await
/// });
/// ```
impl<F, Fut> Middleware for F
where
    F: Fn(Update, Next) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = DispatchResult> + Send + 'static,
{
    fn call(&self, update: Update, next: Next) -> BoxFuture {
        Box::pin((self)(update, next))
    }
}

// Built-in middleware

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
/// // Allow at most 5 messages per second per user.
/// dp.middleware(RateLimitMiddleware::new(5, Duration::from_secs(1)));
/// ```
#[derive(Clone)]
pub struct RateLimitMiddleware {
    inner: Arc<RateLimitInner>,
}

struct RateLimitInner {
    max_calls: u32,
    window: Duration,
    /// user_id → (call count in current window, window start)
    state: DashMap<i64, (u32, Instant)>,
}

impl RateLimitMiddleware {
    /// Create a new rate limiter.
    ///
    /// `max_calls` - maximum number of message updates allowed within `window`
    /// per user before the update is dropped.
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
            // Only rate-limit message updates; pass everything else through.
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
                    // Window expired - reset.
                    *count = 1;
                    *window_start = now;
                    true
                } else if *count < inner.max_calls {
                    *count += 1;
                    true
                } else {
                    // Limit exceeded.
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
/// `std::panic::catch_unwind` only catches panics that occur synchronously
/// during the construction of a future, **not** panics that occur during
/// `.await` inside that future. The only reliable way to catch async panics
/// in Tokio is to run the downstream chain in a `tokio::task::spawn` and
/// inspect the `JoinError` on completion.
///
/// This middleware does exactly that. The spawned task shares the current
/// runtime; overhead is a single task allocation per update, which is
/// negligible compared to I/O.
///
/// # Example
///
/// ```rust,no_run
/// use ferogram::filters::Dispatcher;
/// use ferogram::middleware::PanicRecoveryMiddleware;
///
/// let mut dp = Dispatcher::new();
/// // Register first - it must be the outermost layer to catch inner panics.
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
            // Spawn the downstream chain as a tracked Tokio task. A panic
            // anywhere inside - including across .await yield points - causes
            // the task to finish with a JoinError::is_panic() == true.
            let join_handle = tokio::task::spawn(async move { next.run(update).await });

            match join_handle.await {
                Ok(result) => result,
                Err(join_error) if join_error.is_panic() => {
                    // Extract the panic message if possible.
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
                    // Task was cancelled (runtime is shutting down).
                    tracing::warn!("dispatch task cancelled during shutdown");
                    Err(DispatchError::msg(format!(
                        "dispatch task cancelled: {join_error}"
                    )))
                }
            }
        })
    }
}
