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
