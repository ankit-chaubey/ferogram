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

use crate::filters::core::Filter;

use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, LazyLock};

use super::core::BoxFilter;
#[cfg(feature = "fsm")]
use crate::fsm::{FsmState, StateContext, StateKey, StateKeyStrategy, StateStorage};
use crate::middleware::{BoxFuture, DispatchResult, Middleware, Next, PanicRecoveryMiddleware};
use crate::update::{CallbackQuery, IncomingMessage, InlineQuery, InlineSend, Update};

// Internal handler types

type MsgFuture = Pin<Box<dyn Future<Output = ()> + Send + 'static>>;
type HandlerFn = Arc<dyn Fn(IncomingMessage) -> MsgFuture + Send + Sync + 'static>;
type CallbackHandlerFn = Arc<dyn Fn(CallbackQuery) -> MsgFuture + Send + Sync + 'static>;
type InlineQueryHandlerFn = Arc<dyn Fn(InlineQuery) -> MsgFuture + Send + Sync + 'static>;
type InlineSendHandlerFn = Arc<dyn Fn(InlineSend) -> MsgFuture + Send + Sync + 'static>;
#[cfg(feature = "fsm")]
type FsmHandlerFn = Arc<dyn Fn(IncomingMessage, StateContext) -> MsgFuture + Send + Sync + 'static>;
#[cfg(feature = "fsm")]
type FsmCallbackHandlerFn =
    Arc<dyn Fn(CallbackQuery, StateContext) -> MsgFuture + Send + Sync + 'static>;

#[derive(Clone)]
pub(crate) struct MessageHandler {
    filter: BoxFilter,
    handler: HandlerFn,
}

#[derive(Clone)]
pub(crate) struct CallbackHandler {
    filter: BoxFilter<CallbackQuery>,
    handler: CallbackHandlerFn,
}

#[derive(Clone)]
pub(crate) struct InlineQueryHandler {
    filter: BoxFilter<InlineQuery>,
    handler: InlineQueryHandlerFn,
}

#[derive(Clone)]
pub(crate) struct InlineSendHandler {
    filter: BoxFilter<InlineSend>,
    handler: InlineSendHandlerFn,
}

#[cfg(feature = "fsm")]
#[derive(Clone)]
pub(crate) struct FsmMessageHandler {
    filter: BoxFilter,
    /// Serialized via [`FsmState::as_key`]; matched against the live state.
    expected_state: String,
    handler: FsmHandlerFn,
}

#[cfg(feature = "fsm")]
#[derive(Clone)]
pub(crate) struct FsmCallbackHandler {
    filter: BoxFilter<CallbackQuery>,
    /// Serialized via [`FsmState::as_key`]; matched against the live state.
    expected_state: String,
    handler: FsmCallbackHandlerFn,
}

// Shared, zero-allocation stand-ins for "this handler category is empty".
// `dispatch()` snapshots every handler list on every call (see its doc
// comment); for a bot that never registers callback/inline handlers, that
// would otherwise mean a wasted `Arc::new(Vec::new())` heap allocation per
// category, per dispatched update, for handler kinds it never uses. Cloning
// one of these statics is just an atomic refcount bump instead.
static EMPTY_CALLBACK: LazyLock<Arc<Vec<CallbackHandler>>> = LazyLock::new(|| Arc::new(Vec::new()));
static EMPTY_INLINE_QUERY: LazyLock<Arc<Vec<InlineQueryHandler>>> =
    LazyLock::new(|| Arc::new(Vec::new()));
static EMPTY_INLINE_SEND: LazyLock<Arc<Vec<InlineSendHandler>>> =
    LazyLock::new(|| Arc::new(Vec::new()));
#[cfg(feature = "fsm")]
static EMPTY_FSM_CALLBACK: LazyLock<Arc<Vec<FsmCallbackHandler>>> =
    LazyLock::new(|| Arc::new(Vec::new()));

/// Snapshot `handlers` for a single `dispatch()` call: a cheap `Arc::clone`
/// of a shared empty singleton if there's nothing registered, otherwise a
/// real clone of the (non-empty) list.
fn snapshot<T: Clone>(handlers: &[T], empty: &Arc<Vec<T>>) -> Arc<Vec<T>> {
    if handlers.is_empty() {
        Arc::clone(empty)
    } else {
        Arc::new(handlers.to_vec())
    }
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
///     let mut r = Router::new().scope(private());
///     r.on_message(command("profile"),  |msg| async move { /* ... */ });
///     r.on_message(command("settings"), |msg| async move { /* ... */ });
///     r
/// }
/// ```
///
/// `scope` only restricts `on_message`/`on_edit` handlers (it filters over
/// [`IncomingMessage`]). Callback/inline handlers registered on a scoped
/// router are still included, unscoped, since a button press or inline
/// query isn't an `IncomingMessage` to filter over.
pub struct Router {
    scope: Option<BoxFilter>,
    new_msg: Vec<MessageHandler>,
    edited_msg: Vec<MessageHandler>,
    callback_query: Vec<CallbackHandler>,
    inline_query: Vec<InlineQueryHandler>,
    inline_send: Vec<InlineSendHandler>,
    #[cfg(feature = "fsm")]
    fsm_new_msg: Vec<FsmMessageHandler>,
    #[cfg(feature = "fsm")]
    fsm_edited_msg: Vec<FsmMessageHandler>,
    #[cfg(feature = "fsm")]
    fsm_callback_query: Vec<FsmCallbackHandler>,
    children: Vec<Router>,
}

impl Router {
    /// Create a new, empty router.
    pub fn new() -> Self {
        Self {
            scope: None,
            new_msg: Vec::new(),
            edited_msg: Vec::new(),
            callback_query: Vec::new(),
            inline_query: Vec::new(),
            inline_send: Vec::new(),
            #[cfg(feature = "fsm")]
            fsm_new_msg: Vec::new(),
            #[cfg(feature = "fsm")]
            fsm_edited_msg: Vec::new(),
            #[cfg(feature = "fsm")]
            fsm_callback_query: Vec::new(),
            children: Vec::new(),
        }
    }

    /// Restrict all `on_message`/`on_edit` handlers in this router (and its
    /// children) to messages that also pass `filter`. Scopes compose when
    /// routers are nested. Does not affect callback/inline handlers.
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

    /// Register a handler for `CallbackQuery` updates (inline keyboard button
    /// presses) whose query passes `filter`.
    ///
    /// # Example
    /// ```rust,no_run
    /// use ferogram::filters::{Router, data};
    ///
    /// let mut r = Router::new();
    /// r.on_callback_query(data("close"), |cb| async move {
    ///     // cb.answer().text("Closed!").send(&client).await?;
    /// });
    /// ```
    pub fn on_callback_query<H, Fut>(&mut self, filter: BoxFilter<CallbackQuery>, handler: H)
    where
        H: Fn(CallbackQuery) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        let hfn: CallbackHandlerFn = Arc::new(move |cb| Box::pin(handler(cb)) as MsgFuture);
        self.callback_query.push(CallbackHandler {
            filter,
            handler: hfn,
        });
    }

    /// Register a handler for `InlineQuery` updates (`@bot something`) whose
    /// query passes `filter`.
    pub fn on_inline_query<H, Fut>(&mut self, filter: BoxFilter<InlineQuery>, handler: H)
    where
        H: Fn(InlineQuery) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        let hfn: InlineQueryHandlerFn = Arc::new(move |iq| Box::pin(handler(iq)) as MsgFuture);
        self.inline_query.push(InlineQueryHandler {
            filter,
            handler: hfn,
        });
    }

    /// Register a handler for `InlineSend` updates (a user picked one of your
    /// inline results) whose payload passes `filter`.
    pub fn on_inline_send<H, Fut>(&mut self, filter: BoxFilter<InlineSend>, handler: H)
    where
        H: Fn(InlineSend) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        let hfn: InlineSendHandlerFn = Arc::new(move |is| Box::pin(handler(is)) as MsgFuture);
        self.inline_send.push(InlineSendHandler {
            filter,
            handler: hfn,
        });
    }

    /// Register an FSM handler for `NewMessage` updates that fires only when the
    /// stored state for the conversation slot matches `state` and `filter` passes.
    #[cfg(feature = "fsm")]
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
    #[cfg(feature = "fsm")]
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

    /// Register an FSM handler for `CallbackQuery` updates that fires only
    /// when the stored state for the conversation slot matches `state` and
    /// `filter` passes. The FSM key is derived the same way as for messages
    /// (via [`crate::update::CallbackQuery::chat_id`] and the query's
    /// `user_id`), so a button press can advance the same conversation a
    /// text message would.
    #[cfg(feature = "fsm")]
    pub fn on_callback_query_fsm<S, H, Fut>(
        &mut self,
        filter: BoxFilter<CallbackQuery>,
        state: S,
        handler: H,
    ) where
        S: FsmState,
        H: Fn(CallbackQuery, StateContext) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        let expected_state = state.as_key();
        let hfn: FsmCallbackHandlerFn =
            Arc::new(move |cb, ctx| Box::pin(handler(cb, ctx)) as MsgFuture);
        self.fsm_callback_query.push(FsmCallbackHandler {
            filter,
            expected_state,
            handler: hfn,
        });
    }

    /// Include a child router. Handlers from the child are merged on
    /// flattening, with scopes composed correctly.
    pub fn include(&mut self, router: Router) {
        self.children.push(router);
    }

    /// Flatten the router tree into sorted handler lists, applying scope
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
        // Callback/inline handlers aren't restricted by a message-typed
        // scope; carry them through unchanged.
        flat.callback_query.extend(self.callback_query);
        flat.inline_query.extend(self.inline_query);
        flat.inline_send.extend(self.inline_send);
        #[cfg(feature = "fsm")]
        for h in self.fsm_new_msg {
            flat.fsm_new_msg
                .push(scoped_fsm(h, combined_scope.as_ref()));
        }
        #[cfg(feature = "fsm")]
        for h in self.fsm_edited_msg {
            flat.fsm_edited_msg
                .push(scoped_fsm(h, combined_scope.as_ref()));
        }
        #[cfg(feature = "fsm")]
        flat.fsm_callback_query.extend(self.fsm_callback_query);

        for child in self.children {
            let child_flat = child.flatten(combined_scope.clone());
            flat.new_msg.extend(child_flat.new_msg);
            flat.edited_msg.extend(child_flat.edited_msg);
            flat.callback_query.extend(child_flat.callback_query);
            flat.inline_query.extend(child_flat.inline_query);
            flat.inline_send.extend(child_flat.inline_send);
            #[cfg(feature = "fsm")]
            flat.fsm_new_msg.extend(child_flat.fsm_new_msg);
            #[cfg(feature = "fsm")]
            flat.fsm_edited_msg.extend(child_flat.fsm_edited_msg);
            #[cfg(feature = "fsm")]
            flat.fsm_callback_query
                .extend(child_flat.fsm_callback_query);
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

#[cfg(feature = "fsm")]
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
    pub callback_query: Vec<CallbackHandler>,
    pub inline_query: Vec<InlineQueryHandler>,
    pub inline_send: Vec<InlineSendHandler>,
    #[cfg(feature = "fsm")]
    pub fsm_new_msg: Vec<FsmMessageHandler>,
    #[cfg(feature = "fsm")]
    pub fsm_edited_msg: Vec<FsmMessageHandler>,
    #[cfg(feature = "fsm")]
    pub fsm_callback_query: Vec<FsmCallbackHandler>,
}

// Dispatcher

/// Central dispatcher: routes updates through middleware and into handlers.
///
/// # Handler dispatch order
///
/// 1. All middleware runs in registration order.
/// 2. For message and callback-query updates, if a `StateStorage` is
///    configured and the current conversation has an active state, FSM
///    handlers are checked **first**; the first matching FSM handler wins
///    and no regular handlers run for that update.
/// 3. Regular handlers are checked in registration order; **every** handler
///    whose filter matches runs (match-all, not first-match).
///
/// Because callback-query handlers can match-all, [`CallbackQuery::answer`]
/// guards against being called more than once for the same query -- only
/// the first `.answer().send(...)` reaches Telegram; later calls from other
/// matching handlers log a warning and no-op.
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
/// let dp = Arc::new(Dispatcher::new());
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
    callback_query: Vec<CallbackHandler>,
    inline_query: Vec<InlineQueryHandler>,
    inline_send: Vec<InlineSendHandler>,
    #[cfg(feature = "fsm")]
    fsm_new_msg: Vec<FsmMessageHandler>,
    #[cfg(feature = "fsm")]
    fsm_edited_msg: Vec<FsmMessageHandler>,
    #[cfg(feature = "fsm")]
    fsm_callback_query: Vec<FsmCallbackHandler>,
    middlewares: Vec<Arc<dyn Middleware>>,
    #[cfg(feature = "fsm")]
    state_storage: Option<Arc<dyn StateStorage>>,
    #[cfg(feature = "fsm")]
    key_strategy: StateKeyStrategy,
}

impl Dispatcher {
    /// Create an empty dispatcher.
    pub fn new() -> Self {
        Self {
            new_msg: Vec::new(),
            edited_msg: Vec::new(),
            callback_query: Vec::new(),
            inline_query: Vec::new(),
            inline_send: Vec::new(),
            #[cfg(feature = "fsm")]
            fsm_new_msg: Vec::new(),
            #[cfg(feature = "fsm")]
            fsm_edited_msg: Vec::new(),
            #[cfg(feature = "fsm")]
            fsm_callback_query: Vec::new(),
            middlewares: vec![Arc::new(PanicRecoveryMiddleware::new())],
            #[cfg(feature = "fsm")]
            state_storage: None,
            #[cfg(feature = "fsm")]
            key_strategy: StateKeyStrategy::default(),
        }
    }

    /// Add a middleware layer. Closures implement [`Middleware`] automatically.
    pub fn middleware(&mut self, mw: impl Middleware) {
        self.middlewares.push(Arc::new(mw));
    }

    /// Configure the `StateStorage` backend for FSM handlers.
    #[cfg(feature = "fsm")]
    pub fn with_state_storage(&mut self, storage: Arc<dyn StateStorage>) {
        self.state_storage = Some(storage);
    }

    /// Override the default [`StateKeyStrategy`] (`PerUserPerChat`).
    #[cfg(feature = "fsm")]
    pub fn with_key_strategy(&mut self, strategy: StateKeyStrategy) {
        self.key_strategy = strategy;
    }

    /// Register a handler for `NewMessage` updates matching `filter`.
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

    /// Register a handler for `CallbackQuery` updates (inline keyboard button
    /// presses) matching `filter`.
    ///
    /// # Example
    /// ```rust,no_run
    /// use ferogram::filters::{Dispatcher, data_prefix};
    ///
    /// let mut dp = Dispatcher::new();
    /// dp.on_callback_query(data_prefix("page:"), |cb| async move {
    ///     // cb.answer().send(&client).await?;
    /// });
    /// ```
    pub fn on_callback_query<H, Fut>(&mut self, filter: BoxFilter<CallbackQuery>, handler: H)
    where
        H: Fn(CallbackQuery) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        let hfn: CallbackHandlerFn = Arc::new(move |cb| Box::pin(handler(cb)) as MsgFuture);
        self.callback_query.push(CallbackHandler {
            filter,
            handler: hfn,
        });
    }

    /// Register a handler for `InlineQuery` updates (`@bot something`)
    /// matching `filter`.
    pub fn on_inline_query<H, Fut>(&mut self, filter: BoxFilter<InlineQuery>, handler: H)
    where
        H: Fn(InlineQuery) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        let hfn: InlineQueryHandlerFn = Arc::new(move |iq| Box::pin(handler(iq)) as MsgFuture);
        self.inline_query.push(InlineQueryHandler {
            filter,
            handler: hfn,
        });
    }

    /// Register a handler for `InlineSend` updates (a user picked one of your
    /// inline results) matching `filter`.
    pub fn on_inline_send<H, Fut>(&mut self, filter: BoxFilter<InlineSend>, handler: H)
    where
        H: Fn(InlineSend) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        let hfn: InlineSendHandlerFn = Arc::new(move |is| Box::pin(handler(is)) as MsgFuture);
        self.inline_send.push(InlineSendHandler {
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
    #[cfg(feature = "fsm")]
    pub fn on_message_fsm<S, H, Fut>(&mut self, filter: BoxFilter, state: S, handler: H)
    where
        S: FsmState,
        H: Fn(IncomingMessage, StateContext) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        if self.state_storage.is_none() {
            tracing::warn!(
                "[ferogram::router] on_message_fsm handler registered but no StateStorage is set -- \
                 this handler will never fire. Call dp.with_state_storage(storage) before dispatching."
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
    #[cfg(feature = "fsm")]
    pub fn on_edit_fsm<S, H, Fut>(&mut self, filter: BoxFilter, state: S, handler: H)
    where
        S: FsmState,
        H: Fn(IncomingMessage, StateContext) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        if self.state_storage.is_none() {
            tracing::warn!(
                "[ferogram::router] on_edit_fsm handler registered but no StateStorage is set -- \
                 this handler will never fire. Call dp.with_state_storage(storage) before dispatching."
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

    /// Register an FSM handler for `CallbackQuery` updates. See
    /// [`Router::on_callback_query_fsm`] for how the FSM key is derived.
    #[cfg(feature = "fsm")]
    pub fn on_callback_query_fsm<S, H, Fut>(
        &mut self,
        filter: BoxFilter<CallbackQuery>,
        state: S,
        handler: H,
    ) where
        S: FsmState,
        H: Fn(CallbackQuery, StateContext) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        if self.state_storage.is_none() {
            tracing::warn!(
                "[ferogram::router] on_callback_query_fsm handler registered but no StateStorage is set -- \
                 this handler will never fire. Call dp.with_state_storage(storage) before dispatching."
            );
        }
        let expected_state = state.as_key();
        let hfn: FsmCallbackHandlerFn =
            Arc::new(move |cb, ctx| Box::pin(handler(cb, ctx)) as MsgFuture);
        self.fsm_callback_query.push(FsmCallbackHandler {
            filter,
            expected_state,
            handler: hfn,
        });
    }

    /// Merge a [`Router`] (and all its nested children) into this dispatcher.
    pub fn include(&mut self, router: Router) {
        let flat = router.flatten(None);
        self.new_msg.extend(flat.new_msg);
        self.edited_msg.extend(flat.edited_msg);
        self.callback_query.extend(flat.callback_query);
        self.inline_query.extend(flat.inline_query);
        self.inline_send.extend(flat.inline_send);
        #[cfg(feature = "fsm")]
        self.fsm_new_msg.extend(flat.fsm_new_msg);
        #[cfg(feature = "fsm")]
        self.fsm_edited_msg.extend(flat.fsm_edited_msg);
        #[cfg(feature = "fsm")]
        self.fsm_callback_query.extend(flat.fsm_callback_query);
    }

    /// Dispatch a single update through the middleware chain and into every
    /// matching handler (in registration order).
    pub async fn dispatch(&self, update: Update) {
        let new_msg = Arc::new(self.new_msg.clone());
        let edited_msg = Arc::new(self.edited_msg.clone());
        let callback_query = snapshot(&self.callback_query, &EMPTY_CALLBACK);
        let inline_query = snapshot(&self.inline_query, &EMPTY_INLINE_QUERY);
        let inline_send = snapshot(&self.inline_send, &EMPTY_INLINE_SEND);
        #[cfg(feature = "fsm")]
        let fsm_new = Arc::new(self.fsm_new_msg.clone());
        #[cfg(feature = "fsm")]
        let fsm_edited = Arc::new(self.fsm_edited_msg.clone());
        #[cfg(feature = "fsm")]
        let fsm_callback = snapshot(&self.fsm_callback_query, &EMPTY_FSM_CALLBACK);
        #[cfg(feature = "fsm")]
        let storage = self.state_storage.clone();
        #[cfg(feature = "fsm")]
        let strategy = self.key_strategy;

        let endpoint: Arc<dyn Fn(Update) -> BoxFuture + Send + Sync> =
            Arc::new(move |upd: Update| {
                let new_msg = Arc::clone(&new_msg);
                let edited_msg = Arc::clone(&edited_msg);
                let callback_query = Arc::clone(&callback_query);
                let inline_query = Arc::clone(&inline_query);
                let inline_send = Arc::clone(&inline_send);
                #[cfg(feature = "fsm")]
                let fsm_new = Arc::clone(&fsm_new);
                #[cfg(feature = "fsm")]
                let fsm_edited = Arc::clone(&fsm_edited);
                #[cfg(feature = "fsm")]
                let fsm_callback = Arc::clone(&fsm_callback);
                #[cfg(feature = "fsm")]
                let storage = storage.clone();

                Box::pin(async move {
                    dispatch_to_handlers(
                        upd,
                        &new_msg,
                        &edited_msg,
                        &callback_query,
                        &inline_query,
                        &inline_send,
                        #[cfg(feature = "fsm")]
                        &fsm_new,
                        #[cfg(feature = "fsm")]
                        &fsm_edited,
                        #[cfg(feature = "fsm")]
                        &fsm_callback,
                        #[cfg(feature = "fsm")]
                        storage,
                        #[cfg(feature = "fsm")]
                        strategy,
                    )
                    .await;
                    Ok(()) as DispatchResult
                })
            });

        if self.middlewares.is_empty() {
            if let Err(e) = (endpoint)(update).await {
                tracing::error!(error = %e, "[ferogram::router] handler returned an error");
            }
            return;
        }

        let chain: Arc<[Arc<dyn Middleware>]> = self.middlewares.clone().into();
        let next = Next::new(chain, endpoint);
        if let Err(e) = next.run(update).await {
            tracing::error!(error = %e, "[ferogram::router] handler returned an error");
        }
    }
}

impl Default for Dispatcher {
    fn default() -> Self {
        Self::new()
    }
}

/// Route a single update to every matching handler (see [`run_message`]).
#[allow(clippy::too_many_arguments)]
async fn dispatch_to_handlers(
    update: Update,
    new_msg: &[MessageHandler],
    edited_msg: &[MessageHandler],
    callback_query: &[CallbackHandler],
    inline_query: &[InlineQueryHandler],
    inline_send: &[InlineSendHandler],
    #[cfg(feature = "fsm")] fsm_new: &[FsmMessageHandler],
    #[cfg(feature = "fsm")] fsm_edited: &[FsmMessageHandler],
    #[cfg(feature = "fsm")] fsm_callback: &[FsmCallbackHandler],
    #[cfg(feature = "fsm")] storage: Option<Arc<dyn StateStorage>>,
    #[cfg(feature = "fsm")] strategy: StateKeyStrategy,
) {
    match update {
        Update::NewMessage(msg) => {
            run_message(
                msg,
                new_msg,
                #[cfg(feature = "fsm")]
                fsm_new,
                #[cfg(feature = "fsm")]
                storage,
                #[cfg(feature = "fsm")]
                strategy,
            )
            .await;
        }
        Update::MessageEdited(msg) => {
            run_message(
                msg,
                edited_msg,
                #[cfg(feature = "fsm")]
                fsm_edited,
                #[cfg(feature = "fsm")]
                storage,
                #[cfg(feature = "fsm")]
                strategy,
            )
            .await;
        }
        Update::CallbackQuery(cb) => {
            run_callback(
                cb,
                callback_query,
                #[cfg(feature = "fsm")]
                fsm_callback,
                #[cfg(feature = "fsm")]
                storage,
                #[cfg(feature = "fsm")]
                strategy,
            )
            .await;
        }
        Update::InlineQuery(iq) => {
            run_inline_query(iq, inline_query).await;
        }
        Update::InlineSend(is) => {
            run_inline_send(is, inline_send).await;
        }
        _ => {}
    }
}

/// Inner dispatch for `IncomingMessage` updates.
///
/// Priority:
/// 1. FSM handlers - if storage is set and the conversation has an active
///    state that matches, the **first** matching FSM handler fires and we
///    stop; regular handlers are not run for this update.
/// 2. Regular handlers - **every** handler whose filter matches runs, in
///    registration order (this is what lets a catch-all logging handler and
///    a specific command handler both fire for the same update). Use a
///    narrower filter (or split into separate routers/middleware) if you
///    need only one handler to claim an update.
async fn run_message(
    msg: IncomingMessage,
    regular: &[MessageHandler],
    #[cfg(feature = "fsm")] fsm: &[FsmMessageHandler],
    #[cfg(feature = "fsm")] storage: Option<Arc<dyn StateStorage>>,
    #[cfg(feature = "fsm")] strategy: StateKeyStrategy,
) {
    #[cfg(feature = "fsm")]
    if let Some(ref arc_storage) = storage
        && !fsm.is_empty()
    {
        let key = StateKey::from_message(&msg, strategy);

        let current_state = match arc_storage.get_state(key.clone()).await {
            Ok(s) => s,
            Err(e) => {
                tracing::error!(error = %e, "[ferogram::router] FSM: state storage read failed; skipping FSM handlers for this update");
                None
            }
        };

        if let Some(ref current) = current_state {
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

    let matched_idxs: Vec<usize> = regular
        .iter()
        .enumerate()
        .filter(|(_, h)| h.filter.check(&msg))
        .map(|(i, _)| i)
        .collect();

    // Clone for every match but the last, so the common case (zero or one
    // match) never pays for a clone it doesn't need.
    if let Some((&last, rest)) = matched_idxs.split_last() {
        for &idx in rest {
            (regular[idx].handler)(msg.clone()).await;
        }
        (regular[last].handler)(msg).await;
    }
}

/// Inner dispatch for `CallbackQuery` updates.
///
/// Same FSM-first, then match-all priority as [`run_message`]. Every
/// matching handler receives its own clone of the `CallbackQuery`, but all
/// clones share the same `answered` flag (see
/// [`crate::update::CallbackQuery::answer`]), so only the first handler to
/// actually call `.answer().send(...)` reaches Telegram.
async fn run_callback(
    cb: CallbackQuery,
    regular: &[CallbackHandler],
    #[cfg(feature = "fsm")] fsm: &[FsmCallbackHandler],
    #[cfg(feature = "fsm")] storage: Option<Arc<dyn StateStorage>>,
    #[cfg(feature = "fsm")] strategy: StateKeyStrategy,
) {
    #[cfg(feature = "fsm")]
    if let Some(ref arc_storage) = storage
        && !fsm.is_empty()
    {
        let key = StateKey::from_message(&cb, strategy);

        let current_state = match arc_storage.get_state(key.clone()).await {
            Ok(s) => s,
            Err(e) => {
                tracing::error!(error = %e, "[ferogram::router] FSM: state storage read failed; skipping FSM handlers for this callback query");
                None
            }
        };

        if let Some(ref current) = current_state {
            let matched_idx = fsm
                .iter()
                .position(|h| h.expected_state == *current && h.filter.check(&cb));

            if let Some(idx) = matched_idx {
                let ctx = StateContext::new(Arc::clone(arc_storage), key, current.clone());
                (fsm[idx].handler)(cb, ctx).await;
                return;
            }
        }
    }

    let matched_idxs: Vec<usize> = regular
        .iter()
        .enumerate()
        .filter(|(_, h)| h.filter.check(&cb))
        .map(|(i, _)| i)
        .collect();

    if let Some((&last, rest)) = matched_idxs.split_last() {
        for &idx in rest {
            (regular[idx].handler)(cb.clone()).await;
        }
        (regular[last].handler)(cb).await;
    }
}

/// Inner dispatch for `InlineQuery` updates. Match-all, same as regular
/// message handlers; no FSM support (inline queries are stateless/transient).
async fn run_inline_query(iq: InlineQuery, regular: &[InlineQueryHandler]) {
    let matched_idxs: Vec<usize> = regular
        .iter()
        .enumerate()
        .filter(|(_, h)| h.filter.check(&iq))
        .map(|(i, _)| i)
        .collect();

    if let Some((&last, rest)) = matched_idxs.split_last() {
        for &idx in rest {
            (regular[idx].handler)(iq.clone()).await;
        }
        (regular[last].handler)(iq).await;
    }
}

/// Inner dispatch for `InlineSend` updates. Match-all, same as regular
/// message handlers; no FSM support.
async fn run_inline_send(is: InlineSend, regular: &[InlineSendHandler]) {
    let matched_idxs: Vec<usize> = regular
        .iter()
        .enumerate()
        .filter(|(_, h)| h.filter.check(&is))
        .map(|(i, _)| i)
        .collect();

    if let Some((&last, rest)) = matched_idxs.split_last() {
        for &idx in rest {
            (regular[idx].handler)(is.clone()).await;
        }
        (regular[last].handler)(is).await;
    }
}
