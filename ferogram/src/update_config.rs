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

//! Configuration for user-facing update buffering.
//!
//! Internal MTProto state (pts, qts, gap detection, getDifference) always
//! runs inside the reader task and is never interrupted. Only the high-level
//! [`Update`] queue that your application reads from [`stream_updates()`]
//! is controlled by these types.
//!
//! [`Update`]: crate::update::Update
//! [`stream_updates()`]: crate::Client::stream_updates

/// What to do when the user-dispatch queue is at capacity and a new update arrives.
///
/// Regardless of which strategy is chosen, internal MTProto state (pts, qts,
/// gap detection) is always processed and never dropped.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum OverflowStrategy {
    /// Drop the oldest (stalest) update to make room for the incoming one.
    ///
    /// Recommended for most bots and userbots: a typing indicator from 30
    /// seconds ago is useless, whereas an incoming message right now matters.
    /// The internal dispatch task maintains a ring buffer of `queue_capacity`
    /// items; when it is full the front element is evicted first.
    ///
    /// Ephemeral updates (online status, typing) are evicted before Normal
    /// ones when the queue contains a mix.
    #[default]
    DropOldest,

    /// Drop the arriving update instead of displacing an existing one.
    ///
    /// Keeps a strict FIFO queue using a plain `tokio::mpsc` channel.
    /// Useful when you would rather miss a burst of new events than lose
    /// something already enqueued (rare; `DropOldest` is usually better).
    DropNewest,
}

/// Coarse priority used to decide which buffered update to evict first when
/// `OverflowStrategy::DropOldest` is active and the queue is full.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum UpdatePriority {
    /// Ephemeral presence signals: typing indicators, online status.
    /// Safe to drop first under memory pressure.
    Ephemeral = 0,
    /// Everything else: messages, queries, reactions, participant events.
    Normal = 1,
}

/// Classify a high-level [`Update`] by eviction priority.
///
/// [`Update`]: crate::update::Update
pub(crate) fn update_priority(upd: &crate::update::Update) -> UpdatePriority {
    use crate::update::Update::*;
    match upd {
        // Pure presence signals: safe to drop first.
        UserStatus(_) | UserTyping(_) => UpdatePriority::Ephemeral,
        // Everything else: messages, queries, participants, reactions, etc.
        _ => UpdatePriority::Normal,
    }
}

/// Configuration for the user-facing update dispatch queue.
///
/// Pass this via [`ClientBuilder::update_queue_capacity`] /
/// [`ClientBuilder::update_overflow_strategy`] or set
/// `Config::update_config` directly.
///
/// # Example
///
/// ```rust,no_run
/// use ferogram::{Client, update_config::{UpdateConfig, OverflowStrategy}};
///
/// # #[tokio::main] async fn main() -> anyhow::Result<()> {
/// let (client, _) = Client::builder()
///     .api_id(12345)
///     .api_hash("abc")
///     .session("bot.session")
///     .update_queue_capacity(512)
///     .update_overflow_strategy(OverflowStrategy::DropOldest)
///     .connect().await?;
/// # Ok(()) }
/// ```
#[derive(Debug, Clone)]
pub struct UpdateConfig {
    /// Maximum number of high-level updates held in the dispatch buffer.
    ///
    /// A smaller value uses less RAM; a larger value absorbs bigger bursts
    /// before any eviction occurs.
    ///
    /// Default: `2048`.
    pub queue_capacity: usize,

    /// What happens when the buffer is full and a new update arrives.
    ///
    /// Default: [`OverflowStrategy::DropOldest`].
    pub overflow_strategy: OverflowStrategy,
}

impl Default for UpdateConfig {
    fn default() -> Self {
        Self {
            queue_capacity: 2048,
            overflow_strategy: OverflowStrategy::DropOldest,
        }
    }
}

impl UpdateConfig {
    /// Drops the queue to 256 slots and sets `DropOldest` eviction.
    ///
    /// Good for Termux, small VPS, or any host where RAM is tight.
    /// Prefer [`ClientBuilder::low_memory_mode`] over calling this directly.
    ///
    /// ```rust,no_run
    /// use ferogram::{Client, update_config::UpdateConfig};
    ///
    /// # #[tokio::main] async fn main() -> anyhow::Result<()> {
    /// let (client, _) = Client::builder()
    ///     .api_id(12345)
    ///     .api_hash("abc")
    ///     .session("bot.session")
    ///     .low_memory_mode(true)
    ///     .connect().await?;
    /// # Ok(()) }
    /// ```
    pub fn low_memory() -> Self {
        Self {
            queue_capacity: 256,
            overflow_strategy: OverflowStrategy::DropOldest,
        }
    }
}
