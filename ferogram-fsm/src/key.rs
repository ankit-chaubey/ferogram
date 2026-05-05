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

/// Minimal view of an incoming message needed to build a [`StateKey`].
///
/// Implement this for your message type in the consumer crate so that
/// `StateKey::from_message` can work without a direct dependency on
/// `ferogram` itself.
pub trait MessageLike {
    fn sender_user_id(&self) -> Option<i64>;
    fn chat_id(&self) -> i64;
}

/// Identifies which conversation slot to read/write state for.
///
/// The canonical strategy is per-user-per-chat so that the same user can
/// have independent sessions in different chats simultaneously.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct StateKey {
    /// The Telegram user ID, if applicable.
    pub user_id: Option<i64>,
    /// The Telegram chat ID.
    pub chat_id: i64,
}

impl StateKey {
    /// Construct a key from an incoming message using the given strategy.
    pub fn from_message(msg: &impl MessageLike, strategy: StateKeyStrategy) -> Self {
        match strategy {
            StateKeyStrategy::PerUserPerChat => Self {
                user_id: msg.sender_user_id(),
                chat_id: msg.chat_id(),
            },
            StateKeyStrategy::PerUser => Self {
                user_id: msg.sender_user_id(),
                chat_id: 0,
            },
            StateKeyStrategy::PerChat => Self {
                user_id: None,
                chat_id: msg.chat_id(),
            },
        }
    }
}

/// How the FSM key is composed from an incoming message.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum StateKeyStrategy {
    /// Track state per user per chat (recommended for most bots). Default.
    #[default]
    PerUserPerChat,
    /// Track state per user across all chats (global user session).
    PerUser,
    /// Track state per chat, regardless of sender (e.g. group games).
    PerChat,
}
