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

#![cfg_attr(docsrs, feature(doc_cfg))]
#![doc(html_root_url = "https://docs.rs/ferogram-fsm/0.6.2")]
//! FSM state management for ferogram bots.
//!
//! This crate is part of [ferogram](https://crates.io/crates/ferogram), an async Rust
//! MTProto client built by [Ankit Chaubey](https://github.com/ankit-chaubey).
//!
//! - Channel: [t.me/Ferogram](https://t.me/Ferogram)
//! - Chat: [t.me/FerogramChat](https://t.me/FerogramChat)
//!
//! Provides a finite-state machine layer for multi-step bot conversations.
//! Each user/chat slot holds an optional state string and an arbitrary
//! key-value data bag. Handlers are gated on the current state and receive
//! a [`StateContext`] to transition to the next state or read/write data.
//!
//! Most users reach this through the `ferogram` crate's handler builder
//! (`.state::<MyState>(MyState::WaitingName)`). Use `ferogram-fsm` directly
//! only when building a custom dispatcher or storage backend.
//!
//! # What's in here
//!
//! - **[`FsmState`]**: Trait that state enums must implement. Serialises a
//!   variant to a string key and deserialises it back. Derived automatically
//!   via `#[derive(FsmState)]` from `ferogram-derive`.
//! - **[`StateContext`]**: Injected into state-matched handlers. Exposes
//!   [`StateContext::transition`] to move to the next state, [`StateContext::clear_state`]
//!   to finish the conversation, and typed [`StateContext::set_data`] /
//!   [`StateContext::get_data`] for per-slot JSON-serialised fields.
//! - **[`StateStorage`]**: Async trait for the persistence backend. Implement
//!   it to add Redis, SQLite, or any other store.
//! - **[`MemoryStorage`]**: Built-in in-process backend backed by `DashMap`.
//!   Zero setup; state is lost on restart.
//! - **[`StateKey`]** / **[`StateKeyStrategy`]**: Controls how the storage
//!   slot is keyed. The default strategy keys by `(chat_id, user_id)` so
//!   each user in a group has independent state.
//! - **[`StorageError`]**: Error type returned by all storage operations.
//!
//! # Example
//!
//! ```rust,no_run
//! use ferogram_fsm::{FsmState, MemoryStorage, StateContext};
//!
//! #[derive(Clone, Debug, PartialEq)]
//! enum OrderState { WaitingItem, WaitingQty, Done }
//!
//! impl FsmState for OrderState {
//!     fn as_key(&self) -> String {
//!         match self {
//!             Self::WaitingItem => "WaitingItem".into(),
//!             Self::WaitingQty  => "WaitingQty".into(),
//!             Self::Done        => "Done".into(),
//!         }
//!     }
//!     fn from_key(key: &str) -> Option<Self> {
//!         match key {
//!             "WaitingItem" => Some(Self::WaitingItem),
//!             "WaitingQty"  => Some(Self::WaitingQty),
//!             "Done"        => Some(Self::Done),
//!             _             => None,
//!         }
//!     }
//! }
//!
//! // In practice, use #[derive(FsmState)] from ferogram-derive instead.
//! ```

#![deny(unsafe_code)]

mod context;
mod error;
mod key;
mod storage;

pub use context::StateContext;
pub use error::StorageError;
pub use key::{MessageLike, StateKey, StateKeyStrategy};
pub use storage::{MemoryStorage, StateStorage};

/// A type that can be used as an FSM state.
///
/// Implement this trait on an enum to use it with [`StateContext`] and
/// the FSM dispatcher.
///
/// In practice you will derive this via `#[derive(FsmState)]`:
///
/// ```rust,no_run
/// #[derive(Clone, Debug, PartialEq)]
/// enum CheckoutState {
///     Cart,
///     Address,
///     Payment,
///     Confirmation,
/// }
/// ```
pub trait FsmState: Send + Sync + 'static {
    /// Serialize this state variant to a string key (e.g. `"WaitingProduct"`).
    fn as_key(&self) -> String;

    /// Deserialize a state variant from a key string. Returns `None` if the
    /// key does not match any variant.
    fn from_key(key: &str) -> Option<Self>
    where
        Self: Sized;
}
