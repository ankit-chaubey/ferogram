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
use std::sync::Arc;

use super::core::BoxFilter;
#[cfg(feature = "fsm")]
use crate::fsm::{FsmState, StateContext, StateKey, StateKeyStrategy, StateStorage};
use crate::middleware::{BoxFuture, DispatchResult, Middleware, Next, PanicRecoveryMiddleware};
use crate::update::{IncomingMessage, Update};

// Internal handler types

type MsgFuture = Pin<Box<dyn Future<Output = ()> + Send + 'static>>;
type HandlerFn = Arc<dyn Fn(IncomingMessage) -> MsgFuture + Send + Sync + 'static>;
#[cfg(feature = "fsm")]
type FsmHandlerFn = Arc<dyn Fn(IncomingMessage, StateContext) -> MsgFuture + Send + Sync + 'static>;

#[derive(Clone)]
pub(crate) struct MessageHandler {
    filter: BoxFilter,
    handler: HandlerFn,
}

#[cfg(feature = "fsm")]
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
///     let mut r = Router::new().scope(private());
///     r.on_message(command("profile"),  |msg| async move { /* ... */ });
///     r.on_message(command("settings"), |msg| async move { /* ... */ });
///     r
/// }
/// ```
pub struct Router {
    scope: Option<BoxFilter>,
    new_msg: Vec<MessageHandler>,
    edited_msg: Vec<MessageHandler>,
    #[cfg(feature = "fsm")]
    fsm_new_msg: Vec<FsmMessageHandler>,
    #[cfg(feature = "fsm")]
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
            #[cfg(feature = "fsm")]
            fsm_new_msg: Vec::new(),
            #[cfg(feature = "fsm")]
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

    /// Include a child router. Handlers from the child are merged on
    /// flattening, with scopes composed correctly.
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

        for child in self.children {
            let child_flat = child.flatten(combined_scope.clone());
            flat.new_msg.extend(child_flat.new_msg);
            flat.edited_msg.extend(child_flat.edited_msg);
            #[cfg(feature = "fsm")]
            flat.fsm_new_msg.extend(child_flat.fsm_new_msg);
            #[cfg(feature = "fsm")]
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
    #[cfg(feature = "fsm")]
    pub fsm_new_msg: Vec<FsmMessageHandler>,
    #[cfg(feature = "fsm")]
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
    #[cfg(feature = "fsm")]
    fsm_new_msg: Vec<FsmMessageHandler>,
    #[cfg(feature = "fsm")]
    fsm_edited_msg: Vec<FsmMessageHandler>,
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
            #[cfg(feature = "fsm")]
            fsm_new_msg: Vec::new(),
            #[cfg(feature = "fsm")]
            fsm_edited_msg: Vec::new(),
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

    /// Configure the [`StateStorage`] backend for FSM handlers.
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
    #[cfg(feature = "fsm")]
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

    /// Merge a [`Router`] (and all its nested children) into this dispatcher.
    pub fn include(&mut self, router: Router) {
        let flat = router.flatten(None);
        self.new_msg.extend(flat.new_msg);
        self.edited_msg.extend(flat.edited_msg);
        #[cfg(feature = "fsm")]
        self.fsm_new_msg.extend(flat.fsm_new_msg);
        #[cfg(feature = "fsm")]
        self.fsm_edited_msg.extend(flat.fsm_edited_msg);
    }

    /// Dispatch a single update through the middleware chain and into the first
    /// matching handler.
    pub async fn dispatch(&self, update: Update) {
        let new_msg = Arc::new(self.new_msg.clone());
        let edited_msg = Arc::new(self.edited_msg.clone());
        #[cfg(feature = "fsm")]
        let fsm_new = Arc::new(self.fsm_new_msg.clone());
        #[cfg(feature = "fsm")]
        let fsm_edited = Arc::new(self.fsm_edited_msg.clone());
        #[cfg(feature = "fsm")]
        let storage = self.state_storage.clone();
        #[cfg(feature = "fsm")]
        let strategy = self.key_strategy;

        let endpoint: Arc<dyn Fn(Update) -> BoxFuture + Send + Sync> =
            Arc::new(move |upd: Update| {
                let new_msg = Arc::clone(&new_msg);
                let edited_msg = Arc::clone(&edited_msg);
                #[cfg(feature = "fsm")]
                let fsm_new = Arc::clone(&fsm_new);
                #[cfg(feature = "fsm")]
                let fsm_edited = Arc::clone(&fsm_edited);
                #[cfg(feature = "fsm")]
                let storage = storage.clone();

                Box::pin(async move {
                    dispatch_to_handlers(
                        upd,
                        &new_msg,
                        &edited_msg,
                        #[cfg(feature = "fsm")]
                        &fsm_new,
                        #[cfg(feature = "fsm")]
                        &fsm_edited,
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

/// Route a single update to the first matching handler.
async fn dispatch_to_handlers(
    update: Update,
    new_msg: &[MessageHandler],
    edited_msg: &[MessageHandler],
    #[cfg(feature = "fsm")] fsm_new: &[FsmMessageHandler],
    #[cfg(feature = "fsm")] fsm_edited: &[FsmMessageHandler],
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
        _ => {}
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
                tracing::error!(error = %e, "FSM: failed to read state");
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

    let matched_idx = regular.iter().position(|h| h.filter.check(&msg));
    if let Some(idx) = matched_idx {
        (regular[idx].handler)(msg).await;
    }
}
