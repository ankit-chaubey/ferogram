// Copyright (c) Ankit Chaubey <ankitchaubey.dev@gmail.com>
// SPDX-License-Identifier: MIT OR Apache-2.0
//
// ferogram: async Telegram MTProto client in Rust
// https://github.com/ankit-chaubey/ferogram
//
// If you use or modify this code, keep this notice at the top of your file
// and include the LICENSE-MIT or LICENSE-APACHE file from this repository:
// https://github.com/ankit-chaubey/ferogram

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use crate::fsm::{FsmState, StateContext, StateKey, StateKeyStrategy, StateStorage};
use crate::middleware::{BoxFuture, DispatchResult, Middleware, Next, PanicRecoveryMiddleware};
use crate::update::{IncomingMessage, Update};

// Filter trait

/// A composable, synchronous predicate over an [`IncomingMessage`].
///
/// Use the built-in constructors ([`command`], [`private`], [`text`], …) and
/// combine them with `&`, `|`, `!` operators rather than implementing this
/// trait directly. For arbitrary logic use [`custom`].
pub trait Filter: Send + Sync + 'static {
    fn check(&self, msg: &IncomingMessage) -> bool;
}

impl Filter for Arc<dyn Filter> {
    fn check(&self, msg: &IncomingMessage) -> bool {
        (**self).check(msg)
    }
}

// BoxFilter

/// A heap-allocated, cloneable, composable filter.
///
/// Returned by every built-in filter constructor. Supports `&`, `|`, and `!`
/// operators for building compound expressions.
#[derive(Clone)]
pub struct BoxFilter(Arc<dyn Filter>);

impl BoxFilter {
    fn new<F: Filter>(f: F) -> Self {
        BoxFilter(Arc::new(f))
    }
}

impl Filter for BoxFilter {
    fn check(&self, msg: &IncomingMessage) -> bool {
        self.0.check(msg)
    }
}

impl std::ops::BitAnd for BoxFilter {
    type Output = BoxFilter;
    fn bitand(self, rhs: BoxFilter) -> BoxFilter {
        BoxFilter::new(AndFilter(self, rhs))
    }
}

impl std::ops::BitOr for BoxFilter {
    type Output = BoxFilter;
    fn bitor(self, rhs: BoxFilter) -> BoxFilter {
        BoxFilter::new(OrFilter(self, rhs))
    }
}

impl std::ops::Not for BoxFilter {
    type Output = BoxFilter;
    fn not(self) -> BoxFilter {
        BoxFilter::new(NotFilter(self))
    }
}

struct AndFilter(BoxFilter, BoxFilter);
impl Filter for AndFilter {
    fn check(&self, m: &IncomingMessage) -> bool {
        self.0.check(m) && self.1.check(m)
    }
}

struct OrFilter(BoxFilter, BoxFilter);
impl Filter for OrFilter {
    fn check(&self, m: &IncomingMessage) -> bool {
        self.0.check(m) || self.1.check(m)
    }
}

struct NotFilter(BoxFilter);
impl Filter for NotFilter {
    fn check(&self, m: &IncomingMessage) -> bool {
        !self.0.check(m)
    }
}

struct FnFilter(Arc<dyn Fn(&IncomingMessage) -> bool + Send + Sync + 'static>);
impl Filter for FnFilter {
    fn check(&self, m: &IncomingMessage) -> bool {
        (self.0)(m)
    }
}

fn make<F>(f: F) -> BoxFilter
where
    F: Fn(&IncomingMessage) -> bool + Send + Sync + 'static,
{
    BoxFilter::new(FnFilter(Arc::new(f)))
}

// Built-in filter constructors

/// Passes every message (wildcard / fallback handler).
pub fn all() -> BoxFilter {
    make(|_| true)
}

/// Never passes (disabled handler placeholder).
pub fn none() -> BoxFilter {
    make(|_| false)
}

/// Private (1-on-1) chats only.
pub fn private() -> BoxFilter {
    make(|m| m.is_private())
}

/// Basic group chats only.
pub fn group() -> BoxFilter {
    make(|m| m.is_group())
}

/// Channels and supergroups only.
pub fn channel() -> BoxFilter {
    make(|m| m.is_channel())
}

/// Any non-empty text message.
pub fn text() -> BoxFilter {
    make(|m| m.text().is_some())
}

/// Messages with any media attachment.
pub fn media() -> BoxFilter {
    make(|m| m.has_media())
}

/// Messages with a photo.
pub fn photo() -> BoxFilter {
    make(|m| m.has_photo())
}

/// Messages with a document (file, video, audio, sticker …).
pub fn document() -> BoxFilter {
    make(|m| m.has_document())
}

/// Forwarded messages.
pub fn forwarded() -> BoxFilter {
    make(|m| m.is_forwarded())
}

/// Reply messages.
pub fn reply() -> BoxFilter {
    make(|m| m.is_reply())
}

/// Album / grouped-media messages.
pub fn album() -> BoxFilter {
    make(|m| m.album_id().is_some())
}

/// Any bot command (`/something`).
pub fn any_command() -> BoxFilter {
    make(|m| m.is_bot_command())
}

/// A specific bot command (case-insensitive, strips `@BotName` suffix).
///
/// # Example
/// ```rust,no_run
/// use ferogram::filters::command;
/// let start = command("start");
/// let help  = command("help");
/// ```
pub fn command(name: impl Into<String>) -> BoxFilter {
    let name = name.into();
    make(move |m| m.is_command_named(&name))
}

/// Text contains a substring (case-sensitive).
pub fn text_contains(needle: impl Into<String>) -> BoxFilter {
    let needle = needle.into();
    make(move |m| m.text().is_some_and(|t| t.contains(needle.as_str())))
}

/// Text starts with a prefix (case-sensitive).
pub fn text_starts_with(prefix: impl Into<String>) -> BoxFilter {
    let prefix = prefix.into();
    make(move |m| m.text().is_some_and(|t| t.starts_with(prefix.as_str())))
}

/// Message is from a specific user ID.
pub fn from_user(id: i64) -> BoxFilter {
    make(move |m| m.sender_user_id() == Some(id))
}

/// Message is in a specific chat.
pub fn in_chat(id: i64) -> BoxFilter {
    make(move |m| m.chat_id() == id)
}

/// Filter from an arbitrary closure.
///
/// # Example
/// ```rust,no_run
/// use ferogram::filters::custom;
/// let long_text = custom(|msg| msg.text().map_or(false, |t| t.len() > 200));
/// ```
pub fn custom<F>(f: F) -> BoxFilter
where
    F: Fn(&IncomingMessage) -> bool + Send + Sync + 'static,
{
    make(f)
}

// Internal handler types

type MsgFuture = Pin<Box<dyn Future<Output = ()> + Send + 'static>>;
type HandlerFn = Arc<dyn Fn(IncomingMessage) -> MsgFuture + Send + Sync + 'static>;
type FsmHandlerFn = Arc<dyn Fn(IncomingMessage, StateContext) -> MsgFuture + Send + Sync + 'static>;

#[derive(Clone)]
pub(crate) struct MessageHandler {
    filter: BoxFilter,
    handler: HandlerFn,
}

#[derive(Clone)]
pub(crate) struct FsmMessageHandler {
    filter: BoxFilter,
    /// Serialized via [`FsmState::as_key`]; matched against the live state.
    expected_state: String,
    handler: FsmHandlerFn,
}

// Router

/// A handler registry that can be attached to a Dispatcher.
///
/// Routers have the same handler registration API as [`Dispatcher`].
/// Include them in a dispatcher (or in a parent router) via
/// [`Dispatcher::include`] / [`Router::include`].
///
/// # Scoped routers
///
/// Apply a filter to every handler in a router and its children:
///
/// ```rust,no_run
/// use ferogram::filters::{Router, command, private};
///
/// pub fn user_router() -> Router {
///     // Handlers only fire in private chats.
///     let mut r = Router::new().scope(private());
///     r.on_message(command("profile"),  |msg| async move { /* … */ });
///     r.on_message(command("settings"), |msg| async move { /* … */ });
///     r
/// }
/// ```
///
/// # Nested routers
///
/// ```rust,no_run
/// use ferogram::filters::{Router, command, private, group};
///
/// pub fn root_router() -> Router {
///     let mut root = Router::new();
///     root.include(private_router());
///     root.include(group_router());
///     root
/// }
///
/// fn private_router() -> Router {
///     let mut r = Router::new().scope(private());
///     r.on_message(command("help"), |msg| async move { /* … */ });
///     r
/// }
///
/// fn group_router() -> Router {
///     let mut r = Router::new().scope(group());
///     r.on_message(command("rules"), |msg| async move { /* … */ });
///     r
/// }
/// ```
pub struct Router {
    scope: Option<BoxFilter>,
    new_msg: Vec<MessageHandler>,
    edited_msg: Vec<MessageHandler>,
    fsm_new_msg: Vec<FsmMessageHandler>,
    fsm_edited_msg: Vec<FsmMessageHandler>,
    children: Vec<Router>,
}

impl Router {
    /// Create a new, empty router.
    pub fn new() -> Self {
        Self {
            scope: None,
            new_msg: Vec::new(),
            edited_msg: Vec::new(),
            fsm_new_msg: Vec::new(),
            fsm_edited_msg: Vec::new(),
            children: Vec::new(),
        }
    }

    /// Restrict all handlers in this router (and its children) to messages
    /// that also pass `filter`. Scopes compose when routers are nested.
    pub fn scope(mut self, filter: BoxFilter) -> Self {
        self.scope = Some(filter);
        self
    }

    /// Register a handler for `NewMessage` updates whose message passes `filter`.
    pub fn on_message<H, Fut>(&mut self, filter: BoxFilter, handler: H)
    where
        H: Fn(IncomingMessage) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        let hfn: HandlerFn = Arc::new(move |msg| Box::pin(handler(msg)) as MsgFuture);
        self.new_msg.push(MessageHandler {
            filter,
            handler: hfn,
        });
    }

    /// Register a handler for `MessageEdited` updates whose message passes `filter`.
    pub fn on_edit<H, Fut>(&mut self, filter: BoxFilter, handler: H)
    where
        H: Fn(IncomingMessage) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        let hfn: HandlerFn = Arc::new(move |msg| Box::pin(handler(msg)) as MsgFuture);
        self.edited_msg.push(MessageHandler {
            filter,
            handler: hfn,
        });
    }

    /// Register an FSM handler for `NewMessage` updates that fires only when the
    /// stored state for the conversation slot matches `state` and `filter` passes.
    pub fn on_message_fsm<S, H, Fut>(&mut self, filter: BoxFilter, state: S, handler: H)
    where
        S: FsmState,
        H: Fn(IncomingMessage, StateContext) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        let expected_state = state.as_key();
        let hfn: FsmHandlerFn = Arc::new(move |msg, ctx| Box::pin(handler(msg, ctx)) as MsgFuture);
        self.fsm_new_msg.push(FsmMessageHandler {
            filter,
            expected_state,
            handler: hfn,
        });
    }

    /// Register an FSM handler for `MessageEdited` updates.
    pub fn on_edit_fsm<S, H, Fut>(&mut self, filter: BoxFilter, state: S, handler: H)
    where
        S: FsmState,
        H: Fn(IncomingMessage, StateContext) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        let expected_state = state.as_key();
        let hfn: FsmHandlerFn = Arc::new(move |msg, ctx| Box::pin(handler(msg, ctx)) as MsgFuture);
        self.fsm_edited_msg.push(FsmMessageHandler {
            filter,
            expected_state,
            handler: hfn,
        });
    }

    /// Include a child router. Handlers from the child are merged on
    /// flattening, with scopes composes correctly.
    pub fn include(&mut self, router: Router) {
        self.children.push(router);
    }

    /// Flatten the router tree into four sorted handler lists, applying scope
    /// filters along the way. `parent_scope` is ANDed with this router's scope.
    pub(crate) fn flatten(self, parent_scope: Option<BoxFilter>) -> FlatHandlers {
        let combined_scope = combine_scopes(parent_scope, self.scope);
        let mut flat = FlatHandlers::default();

        for h in self.new_msg {
            flat.new_msg.push(scoped(h, combined_scope.as_ref()));
        }
        for h in self.edited_msg {
            flat.edited_msg.push(scoped(h, combined_scope.as_ref()));
        }
        for h in self.fsm_new_msg {
            flat.fsm_new_msg
                .push(scoped_fsm(h, combined_scope.as_ref()));
        }
        for h in self.fsm_edited_msg {
            flat.fsm_edited_msg
                .push(scoped_fsm(h, combined_scope.as_ref()));
        }

        for child in self.children {
            let child_flat = child.flatten(combined_scope.clone());
            flat.new_msg.extend(child_flat.new_msg);
            flat.edited_msg.extend(child_flat.edited_msg);
            flat.fsm_new_msg.extend(child_flat.fsm_new_msg);
            flat.fsm_edited_msg.extend(child_flat.fsm_edited_msg);
        }

        flat
    }
}

fn combine_scopes(parent: Option<BoxFilter>, own: Option<BoxFilter>) -> Option<BoxFilter> {
    match (parent, own) {
        (Some(p), Some(s)) => Some(p & s),
        (Some(p), None) | (None, Some(p)) => Some(p),
        (None, None) => None,
    }
}

fn scoped(h: MessageHandler, scope: Option<&BoxFilter>) -> MessageHandler {
    match scope {
        Some(s) => MessageHandler {
            filter: s.clone() & h.filter,
            handler: h.handler,
        },
        None => h,
    }
}

fn scoped_fsm(h: FsmMessageHandler, scope: Option<&BoxFilter>) -> FsmMessageHandler {
    match scope {
        Some(s) => FsmMessageHandler {
            filter: s.clone() & h.filter,
            ..h
        },
        None => h,
    }
}

impl Default for Router {
    fn default() -> Self {
        Self::new()
    }
}

/// Flattened handler lists from [`Router::flatten`].
#[derive(Default)]
pub(crate) struct FlatHandlers {
    pub new_msg: Vec<MessageHandler>,
    pub edited_msg: Vec<MessageHandler>,
    pub fsm_new_msg: Vec<FsmMessageHandler>,
    pub fsm_edited_msg: Vec<FsmMessageHandler>,
}

// Dispatcher

/// Central dispatcher: routes updates through middleware and into handlers.
///
/// # Handler dispatch order
///
/// 1. All middleware runs in registration order.
/// 2. For message updates, if a [`StateStorage`] is configured and the current
///    conversation has an active state, FSM handlers are checked **first**.
/// 3. Regular handlers are checked in registration order.
/// 4. First match wins; no further handlers are tried.
///
/// # Concurrent dispatch
///
/// `dispatch` takes `&self`, so you can wrap the dispatcher in an `Arc` and
/// spawn each update as an independent `tokio::task`:
///
/// ```rust,no_run
/// use ferogram::filters::Dispatcher;
/// use std::sync::Arc;
///
/// # async fn example(mut stream: ferogram::UpdateStream) {
/// let dp = Arc::new(Dispatcher::new()); // populated earlier
///
/// while let Some(upd) = stream.next().await {
///     let dp = Arc::clone(&dp);
///     tokio::spawn(async move { dp.dispatch(upd).await });
/// }
/// # }
/// ```
pub struct Dispatcher {
    new_msg: Vec<MessageHandler>,
    edited_msg: Vec<MessageHandler>,
    fsm_new_msg: Vec<FsmMessageHandler>,
    fsm_edited_msg: Vec<FsmMessageHandler>,
    middlewares: Vec<Arc<dyn Middleware>>,
    state_storage: Option<Arc<dyn StateStorage>>,
    key_strategy: StateKeyStrategy,
}

impl Dispatcher {
    /// Create an empty dispatcher.
    pub fn new() -> Self {
        // Install PanicRecoveryMiddleware as the outermost layer by default.
        // This wraps every handler invocation in a tokio::task::spawn so that
        // panics (including those across .await points) are caught and logged
        // rather than killing the reader task or the whole process.
        // Users can still prepend additional middleware via dp.middleware().
        Self {
            new_msg: Vec::new(),
            edited_msg: Vec::new(),
            fsm_new_msg: Vec::new(),
            fsm_edited_msg: Vec::new(),
            middlewares: vec![Arc::new(PanicRecoveryMiddleware::new())],
            state_storage: None,
            key_strategy: StateKeyStrategy::default(),
        }
    }

    // Middleware

    /// Add a middleware layer. Closures implement [`Middleware`] automatically.
    ///
    /// ```rust,no_run
    /// # use ferogram::filters::Dispatcher;
    /// # let mut dp = Dispatcher::new();
    /// dp.middleware(|upd, next| async move {
    ///     let r = next.run(upd).await;
    ///     r
    /// });
    /// ```
    pub fn middleware(&mut self, mw: impl Middleware) {
        self.middlewares.push(Arc::new(mw));
    }

    // FSM configuration

    /// Configure the [`StateStorage`] backend for FSM handlers.
    ///
    /// Must be called before any `on_message_fsm` / `on_edit_fsm` registrations
    /// take effect. Without storage, FSM handlers are silently skipped and a
    /// `WARN` is emitted at registration time.
    pub fn with_state_storage(&mut self, storage: Arc<dyn StateStorage>) {
        self.state_storage = Some(storage);
    }

    /// Override the default [`StateKeyStrategy`] (`PerUserPerChat`).
    pub fn with_key_strategy(&mut self, strategy: StateKeyStrategy) {
        self.key_strategy = strategy;
    }

    // Handler registration

    /// Register a handler for `NewMessage` updates matching `filter`.
    ///
    /// ```rust,no_run
    /// # use ferogram::filters::{Dispatcher, command};
    /// # let mut dp = Dispatcher::new();
    /// dp.on_message(command("start"), |msg| async move {
    ///     msg.reply("Welcome!").await.ok();
    /// });
    /// ```
    pub fn on_message<H, Fut>(&mut self, filter: BoxFilter, handler: H)
    where
        H: Fn(IncomingMessage) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        let hfn: HandlerFn = Arc::new(move |msg| Box::pin(handler(msg)) as MsgFuture);
        self.new_msg.push(MessageHandler {
            filter,
            handler: hfn,
        });
    }

    /// Register a handler for `MessageEdited` updates matching `filter`.
    pub fn on_edit<H, Fut>(&mut self, filter: BoxFilter, handler: H)
    where
        H: Fn(IncomingMessage) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        let hfn: HandlerFn = Arc::new(move |msg| Box::pin(handler(msg)) as MsgFuture);
        self.edited_msg.push(MessageHandler {
            filter,
            handler: hfn,
        });
    }

    /// Register an FSM handler for `NewMessage` updates.
    ///
    /// Fires only when **both** conditions are met:
    ///
    /// 1. The stored state for the current conversation slot equals `state`.
    /// 2. `filter` passes.
    ///
    /// FSM handlers shadow regular handlers: if a state match is found, no
    /// regular handlers are checked for that update.
    ///
    /// # Panics (at registration)
    ///
    /// Emits a `WARN` trace message if called before [`with_state_storage`];
    /// does not panic.
    ///
    /// [`with_state_storage`]: Dispatcher::with_state_storage
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// use ferogram::{FsmState, fsm::{StateContext, MemoryStorage}};
    /// use ferogram::filters::{Dispatcher, command, text};
    /// use std::sync::Arc;
    ///
    /// #[derive(FsmState, Clone, Debug, PartialEq)]
    /// enum OrderState { WaitingProduct, WaitingQuantity, WaitingAddress }
    ///
    /// # async fn example() {
    /// let mut dp = Dispatcher::new();
    /// dp.with_state_storage(Arc::new(MemoryStorage::new()));
    ///
    /// dp.on_message(command("order"), |msg| async move {
    ///     msg.reply("Which product?").await.ok();
    ///     // State is set by the first FSM handler below on next message.
    /// });
    ///
    /// dp.on_message_fsm(text(), OrderState::WaitingProduct, |msg, state| async move {
    ///     state.set_data("product", msg.text().unwrap()).await.ok();
    ///     state.transition(OrderState::WaitingQuantity).await.ok();
    ///     msg.reply("How many?").await.ok();
    /// });
    ///
    /// dp.on_message_fsm(text(), OrderState::WaitingQuantity, |msg, state| async move {
    ///     state.set_data("qty", msg.text().unwrap()).await.ok();
    ///     state.transition(OrderState::WaitingAddress).await.ok();
    ///     msg.reply("Ship to?").await.ok();
    /// });
    ///
    /// dp.on_message_fsm(text(), OrderState::WaitingAddress, |msg, state| async move {
    ///     let product: Option<String> = state.get_data("product").await.unwrap_or(None);
    ///     let qty: Option<String>     = state.get_data("qty").await.unwrap_or(None);
    ///     msg.reply(format!("Order confirmed: {:?} x {:?}", product, qty)).await.ok();
    ///     state.clear_all().await.ok();
    /// });
    /// # }
    /// ```
    pub fn on_message_fsm<S, H, Fut>(&mut self, filter: BoxFilter, state: S, handler: H)
    where
        S: FsmState,
        H: Fn(IncomingMessage, StateContext) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        if self.state_storage.is_none() {
            tracing::warn!(
                "on_message_fsm registered without a StateStorage - \
                 this handler will never fire. Call dp.with_state_storage(storage) first."
            );
        }
        let expected_state = state.as_key();
        let hfn: FsmHandlerFn = Arc::new(move |msg, ctx| Box::pin(handler(msg, ctx)) as MsgFuture);
        self.fsm_new_msg.push(FsmMessageHandler {
            filter,
            expected_state,
            handler: hfn,
        });
    }

    /// Register an FSM handler for `MessageEdited` updates.
    pub fn on_edit_fsm<S, H, Fut>(&mut self, filter: BoxFilter, state: S, handler: H)
    where
        S: FsmState,
        H: Fn(IncomingMessage, StateContext) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        if self.state_storage.is_none() {
            tracing::warn!(
                "on_edit_fsm registered without a StateStorage - \
                 this handler will never fire. Call dp.with_state_storage(storage) first."
            );
        }
        let expected_state = state.as_key();
        let hfn: FsmHandlerFn = Arc::new(move |msg, ctx| Box::pin(handler(msg, ctx)) as MsgFuture);
        self.fsm_edited_msg.push(FsmMessageHandler {
            filter,
            expected_state,
            handler: hfn,
        });
    }

    // Router inclusion

    /// Merge a [`Router`] (and all its nested children) into this dispatcher.
    ///
    /// Handlers are appended after existing handlers in registration order.
    /// Router scopes are applied at this point.
    ///
    /// ```rust,no_run
    /// use ferogram::filters::{Dispatcher, Router, command};
    ///
    /// fn payment_router() -> Router {
    ///     let mut r = Router::new();
    ///     r.on_message(command("pay"), |msg| async move { /* … */ });
    ///     r
    /// }
    ///
    /// # async fn main_fn() {
    /// let mut dp = Dispatcher::new();
    /// dp.include(payment_router());
    /// # }
    /// ```
    pub fn include(&mut self, router: Router) {
        let flat = router.flatten(None);
        self.new_msg.extend(flat.new_msg);
        self.edited_msg.extend(flat.edited_msg);
        self.fsm_new_msg.extend(flat.fsm_new_msg);
        self.fsm_edited_msg.extend(flat.fsm_edited_msg);
    }

    // Dispatch

    /// Dispatch a single update through the middleware chain and into the first
    /// matching handler.
    ///
    /// Errors returned by middleware are logged at `ERROR` level and swallowed
    /// so that the update loop is never interrupted by a single bad update.
    ///
    /// For concurrent per-update processing, wrap the dispatcher in an `Arc`
    /// and call `tokio::spawn(async move { dp.dispatch(upd).await })`.
    pub async fn dispatch(&self, update: Update) {
        // Build cheap Arc snapshots of the handler lists.
        // Each field is a `Vec<T>` where T contains only `Arc`-backed fields,
        // clone() here only bumps Arc reference counts, no heap allocation.
        let new_msg = Arc::new(self.new_msg.clone());
        let edited_msg = Arc::new(self.edited_msg.clone());
        let fsm_new = Arc::new(self.fsm_new_msg.clone());
        let fsm_edited = Arc::new(self.fsm_edited_msg.clone());
        let storage = self.state_storage.clone(); // Option<Arc<dyn StateStorage>>
        let strategy = self.key_strategy;

        // The endpoint is the final step of the middleware chain: run the
        // matching handler. It is Arc'd so the chain can share it cheaply.
        let endpoint: Arc<dyn Fn(Update) -> BoxFuture + Send + Sync> =
            Arc::new(move |upd: Update| {
                // Clone the Arc snapshots.
                let new_msg = Arc::clone(&new_msg);
                let edited_msg = Arc::clone(&edited_msg);
                let fsm_new = Arc::clone(&fsm_new);
                let fsm_edited = Arc::clone(&fsm_edited);
                let storage = storage.clone();

                Box::pin(async move {
                    dispatch_to_handlers(
                        upd,
                        &new_msg,
                        &edited_msg,
                        &fsm_new,
                        &fsm_edited,
                        storage,
                        strategy,
                    )
                    .await;
                    Ok(()) as DispatchResult
                })
            });

        if self.middlewares.is_empty() {
            // Fast path: skip chain construction overhead.
            if let Err(e) = (endpoint)(update).await {
                tracing::error!(error = %e, "dispatch error");
            }
            return;
        }

        let chain: Arc<[Arc<dyn Middleware>]> = self.middlewares.clone().into();
        let next = Next::new(chain, endpoint);
        if let Err(e) = next.run(update).await {
            tracing::error!(error = %e, "dispatch error");
        }
    }
}

impl Default for Dispatcher {
    fn default() -> Self {
        Self::new()
    }
}

// dispatch_to_handlers

/// Route a single update to the first matching handler.
///
/// Runs the handler chain for a single update.
async fn dispatch_to_handlers(
    update: Update,
    new_msg: &[MessageHandler],
    edited_msg: &[MessageHandler],
    fsm_new: &[FsmMessageHandler],
    fsm_edited: &[FsmMessageHandler],
    storage: Option<Arc<dyn StateStorage>>,
    strategy: StateKeyStrategy,
) {
    match update {
        Update::NewMessage(msg) => {
            run_message(msg, new_msg, fsm_new, storage, strategy).await;
        }
        Update::MessageEdited(msg) => {
            run_message(msg, edited_msg, fsm_edited, storage, strategy).await;
        }
        _ => {
            // Future: add per-variant handler lists for CallbackQuery, InlineQuery, …
        }
    }
}

/// Inner dispatch for `IncomingMessage` updates.
///
/// Priority:
/// 1. FSM handlers - if storage is set and the conversation has an active
///    state that matches, the **first** matching FSM handler fires and we stop.
/// 2. Regular handlers - first match fires.
async fn run_message(
    msg: IncomingMessage,
    regular: &[MessageHandler],
    fsm: &[FsmMessageHandler],
    storage: Option<Arc<dyn StateStorage>>,
    strategy: StateKeyStrategy,
) {
    // Phase 1: FSM handlers.
    if let Some(ref arc_storage) = storage
        && !fsm.is_empty()
    {
        let key = StateKey::from_message(&msg, strategy);

        let current_state = match arc_storage.get_state(key.clone()).await {
            Ok(s) => s,
            Err(e) => {
                tracing::error!(error = %e, "FSM: failed to read state");
                None
            }
        };

        if let Some(ref current) = current_state {
            // Find the first FSM handler whose expected state and filter both match.
            // We only borrow `msg` here for the filter check; the actual call moves it.
            let matched_idx = fsm
                .iter()
                .position(|h| h.expected_state == *current && h.filter.check(&msg));

            if let Some(idx) = matched_idx {
                let ctx = StateContext::new(Arc::clone(arc_storage), key, current.clone());
                (fsm[idx].handler)(msg, ctx).await;
                return;
            }
        }
    }

    // Phase 2: Regular handlers.
    let matched_idx = regular.iter().position(|h| h.filter.check(&msg));
    if let Some(idx) = matched_idx {
        (regular[idx].handler)(msg).await;
    }
}
