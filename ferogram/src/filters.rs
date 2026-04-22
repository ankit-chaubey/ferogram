// Copyright (c) Ankit Chaubey <ankitchaubey.dev@gmail.com>
// SPDX-License-Identifier: MIT OR Apache-2.0
//
// ferogram: async Telegram MTProto client in Rust
// https://github.com/ankit-chaubey/ferogram
//
//
// If you use or modify this code, keep this notice at the top of your file
// and include the LICENSE-MIT or LICENSE-APACHE file from this repository:
// https://github.com/ankit-chaubey/ferogram

//! Handler/filter ergonomics for ferogram.
//!
//! # Quick start
//! ```rust,no_run
//! use ferogram::{Client, Update};
//! use ferogram::filters::{Dispatcher, command, private, text_contains};
//!
//! # async fn example(client: Client) {
//! let mut stream = client.stream_updates();
//! let mut dp = Dispatcher::new();
//!
//! // Register handlers (checked in order; first match wins)
//! dp.on_message(command("start"), |msg| async move {
//!     msg.reply("Welcome!").await.ok();
//! });
//! dp.on_message(private() & text_contains("hello"), |msg| async move {
//!     msg.reply("Hello to you too!").await.ok();
//! });
//!
//! while let Some(upd) = stream.next().await {
//!     dp.dispatch(upd).await;
//! }
//! # }
//! ```

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use crate::update::{IncomingMessage, Update};

/// A composable predicate over an [`IncomingMessage`].
///
/// All built-in constructors (e.g. [`private()`], [`command()`]) return a
/// [`BoxFilter`] which supports `&`, `|`, and `!` operators.
pub trait Filter: Send + Sync + 'static {
    /// Returns `true` if the message passes this filter.
    fn check(&self, msg: &IncomingMessage) -> bool;
}

impl Filter for Arc<dyn Filter> {
    fn check(&self, msg: &IncomingMessage) -> bool {
        (**self).check(msg)
    }
}

/// A heap-allocated, clone-able, composable filter.
///
/// Returned by every built-in filter constructor. Supports `&`, `|`, and `!`
/// operators to build compound filters without needing generic type parameters.
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

/// `filter_a & filter_b` - both must pass.
impl std::ops::BitAnd for BoxFilter {
    type Output = BoxFilter;
    fn bitand(self, rhs: BoxFilter) -> BoxFilter {
        BoxFilter::new(AndFilter(self, rhs))
    }
}

/// `filter_a | filter_b` - at least one must pass.
impl std::ops::BitOr for BoxFilter {
    type Output = BoxFilter;
    fn bitor(self, rhs: BoxFilter) -> BoxFilter {
        BoxFilter::new(OrFilter(self, rhs))
    }
}

/// `!filter` - negation.
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

/// Passes every message (wildcard / fallback handler).
pub fn all() -> BoxFilter {
    make(|_| true)
}

/// Never passes (useful as a disabled-handler placeholder).
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

/// Channels / supergroups only.
pub fn channel() -> BoxFilter {
    make(|m| m.is_channel())
}

/// Any non-empty text message.
pub fn text() -> BoxFilter {
    make(|m| m.text().is_some())
}

/// Messages that carry any media attachment.
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

/// Reply messages (replies to another message).
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
///
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

/// Message is in a specific chat (by numeric ID).
pub fn in_chat(id: i64) -> BoxFilter {
    make(move |m| m.chat_id() == id)
}

/// Build a filter from an arbitrary closure.
///
/// # Example
/// ```rust,no_run
/// use ferogram::filters::custom;
///
/// let long_text = custom(|msg| msg.text().map_or(false, |t| t.len() > 200));
/// ```
pub fn custom<F>(f: F) -> BoxFilter
where
    F: Fn(&IncomingMessage) -> bool + Send + Sync + 'static,
{
    make(f)
}

type BoxFuture = Pin<Box<dyn Future<Output = ()> + Send + 'static>>;
type HandlerFn = Arc<dyn Fn(IncomingMessage) -> BoxFuture + Send + Sync + 'static>;

struct MessageHandler {
    filter: BoxFilter,
    handler: HandlerFn,
}

/// Dispatches [`Update`]s to the **first** matching async handler.
///
/// Handlers are checked in registration order. As soon as one filter passes,
/// its handler runs and no further handlers are tried (first-match-wins).
///
/// Separate handler lists exist for `NewMessage` (registered via
/// [`on_message`]) and `MessageEdited` (registered via [`on_edit`]).
///
/// # Example
/// ```rust,no_run
/// use ferogram::filters::{Dispatcher, command, private, group, channel, text_contains, forwarded};
///
/// # async fn ex(mut stream: ferogram::UpdateStream) {
/// let mut dp = Dispatcher::new();
///
/// dp.on_message(command("start"), |msg| async move {
///     msg.reply("Hello!").await.ok();
/// });
/// dp.on_message(private() & text_contains("hi"), |msg| async move {
///     msg.reply("Hey there!").await.ok();
/// });
/// dp.on_message(group() | channel(), |msg| async move {
///     // fallback for any group or channel message
/// });
/// dp.on_edit(private(), |msg| async move {
///     msg.reply("You edited a message!").await.ok();
/// });
///
/// while let Some(upd) = stream.next().await {
///     dp.dispatch(upd).await;
/// }
/// # }
/// ```
pub struct Dispatcher {
    new_msg: Vec<MessageHandler>,
    edited_msg: Vec<MessageHandler>,
}

impl Dispatcher {
    /// Create an empty dispatcher.
    pub fn new() -> Self {
        Self {
            new_msg: Vec::new(),
            edited_msg: Vec::new(),
        }
    }

    /// Register an async handler for `NewMessage` updates whose message passes
    /// `filter`.
    ///
    /// # Example
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
        let hfn: HandlerFn = Arc::new(move |msg| Box::pin(handler(msg)) as BoxFuture);
        self.new_msg.push(MessageHandler {
            filter,
            handler: hfn,
        });
    }

    /// Register an async handler for `MessageEdited` updates whose message
    /// passes `filter`.
    ///
    /// # Example
    /// ```rust,no_run
    /// # use ferogram::filters::{Dispatcher, private};
    /// # let mut dp = Dispatcher::new();
    /// dp.on_edit(private(), |msg| async move {
    ///     msg.reply("You edited that!").await.ok();
    /// });
    /// ```
    pub fn on_edit<H, Fut>(&mut self, filter: BoxFilter, handler: H)
    where
        H: Fn(IncomingMessage) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        let hfn: HandlerFn = Arc::new(move |msg| Box::pin(handler(msg)) as BoxFuture);
        self.edited_msg.push(MessageHandler {
            filter,
            handler: hfn,
        });
    }

    /// Dispatch one [`Update`] to the first matching registered handler.
    ///
    /// - `NewMessage`    → checked against [`on_message`] handlers.
    /// - `MessageEdited` → checked against [`on_edit`] handlers.
    /// - All other variants are silently ignored.
    pub async fn dispatch(&self, update: Update) {
        match update {
            Update::NewMessage(msg) => self.run(&self.new_msg, msg).await,
            Update::MessageEdited(msg) => self.run(&self.edited_msg, msg).await,
            _ => {}
        }
    }

    async fn run(&self, handlers: &[MessageHandler], msg: IncomingMessage) {
        for h in handlers {
            if h.filter.check(&msg) {
                (h.handler)(msg).await;
                return; // first match wins
            }
        }
    }
}

impl Default for Dispatcher {
    fn default() -> Self {
        Self::new()
    }
}
