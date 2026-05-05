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
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use crate::update::Update;

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

/// An async interceptor in the dispatcher pipeline.
///
/// Middleware is called for **every** update regardless of which handler (if
/// any) matches. This makes it the correct place for cross-cutting concerns
/// such as logging, rate limiting, panic recovery, and metrics.
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
/// # use ferogram::{middleware::Next, update::Update};
/// let mut dp = Dispatcher::new();
/// dp.middleware(|upd: Update, next: Next| async move {
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
