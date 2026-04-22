// Copyright (c) Ankit Chaubey <ankitchaubey.dev@gmail.com>
// SPDX-License-Identifier: MIT OR Apache-2.0
//
// ferogram: async Telegram MTProto client in Rust
// https://github.com/ankit-chaubey/ferogram
//
// If you use or modify this code, keep this notice at the top of your file
// and include the LICENSE-MIT or LICENSE-APACHE file from this repository:
// https://github.com/ankit-chaubey/ferogram

use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;

use async_trait::async_trait;
use dashmap::DashMap;
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::update::IncomingMessage;

// FsmState trait

/// A type that can be used as an FSM state.
///
/// Implement this trait on an enum to use it with [`StateContext`] and
/// [`Dispatcher::on_message_fsm`].
///
/// In practice you will derive this via `#[derive(FsmState)]` (requires the
/// `derive` feature):
///
/// ```rust,no_run
/// use ferogram::FsmState;
///
/// #[derive(FsmState, Clone, Debug, PartialEq)]
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

// StateKey

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
    pub fn from_message(msg: &IncomingMessage, strategy: StateKeyStrategy) -> Self {
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

// StorageError

/// An error from a [`StateStorage`] backend.
#[derive(Debug)]
pub struct StorageError {
    message: String,
    source: Option<Box<dyn std::error::Error + Send + Sync>>,
}

impl StorageError {
    /// Create a storage error with a plain message.
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            source: None,
        }
    }

    /// Create a storage error wrapping an underlying cause.
    pub fn with_source(
        message: impl Into<String>,
        source: impl std::error::Error + Send + Sync + 'static,
    ) -> Self {
        Self {
            message: message.into(),
            source: Some(Box::new(source)),
        }
    }
}

impl fmt::Display for StorageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "state storage error: {}", self.message)?;
        if let Some(ref src) = self.source {
            write!(f, ": {src}")?;
        }
        Ok(())
    }
}

impl std::error::Error for StorageError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.source.as_ref().map(|e| e.as_ref() as _)
    }
}

// StateStorage trait

/// Persistent storage backend for FSM state.
///
/// All methods are async and return `Result<_, StorageError>`.
/// Implement this trait to add custom backends (database, Redis, etc.).
///
/// Built-in implementations:
/// - [`MemoryStorage`] - in-process `DashMap`, zero setup, no persistence.
///
/// # Example - custom backend
///
/// ```rust,no_run
/// use ferogram::fsm::{StateStorage, StateKey, StorageError};
/// use async_trait::async_trait;
/// use std::collections::HashMap;
///
/// struct MyStorage;
///
/// #[async_trait]
/// impl StateStorage for MyStorage {
///     async fn get_state(&self, key: StateKey) -> Result<Option<String>, StorageError> {
///         // fetch from your DB
///         todo!()
///     }
///     // ... (implement all methods)
/// #   async fn set_state(&self, _: StateKey, _: String) -> Result<(), StorageError> { todo!() }
/// #   async fn clear_state(&self, _: StateKey) -> Result<(), StorageError> { todo!() }
/// #   async fn get_data(&self, _: StateKey, _: &str) -> Result<Option<serde_json::Value>, StorageError> { todo!() }
/// #   async fn set_data(&self, _: StateKey, _: &str, _: serde_json::Value) -> Result<(), StorageError> { todo!() }
/// #   async fn get_all_data(&self, _: StateKey) -> Result<HashMap<String, serde_json::Value>, StorageError> { todo!() }
/// #   async fn clear_data(&self, _: StateKey) -> Result<(), StorageError> { todo!() }
/// #   async fn clear_all(&self, _: StateKey) -> Result<(), StorageError> { todo!() }
/// }
/// ```
#[async_trait]
pub trait StateStorage: Send + Sync + 'static {
    /// Return the current state key for this slot, or `None` if no state is
    /// set (i.e. the conversation has not started or has been cleared).
    async fn get_state(&self, key: StateKey) -> Result<Option<String>, StorageError>;

    /// Persist a new state. Overwrites any previously set state.
    async fn set_state(&self, key: StateKey, state: String) -> Result<(), StorageError>;

    /// Clear the state for this slot. The conversation data is NOT cleared;
    /// use [`clear_all`] to reset both.
    async fn clear_state(&self, key: StateKey) -> Result<(), StorageError>;

    /// Retrieve a single data field as a raw JSON value.
    async fn get_data(
        &self,
        key: StateKey,
        field: &str,
    ) -> Result<Option<serde_json::Value>, StorageError>;

    /// Persist a single data field as a raw JSON value. Existing fields are
    /// not affected.
    async fn set_data(
        &self,
        key: StateKey,
        field: &str,
        value: serde_json::Value,
    ) -> Result<(), StorageError>;

    /// Return all data fields stored for this slot.
    async fn get_all_data(
        &self,
        key: StateKey,
    ) -> Result<HashMap<String, serde_json::Value>, StorageError>;

    /// Remove all data fields for this slot. The state is NOT cleared.
    async fn clear_data(&self, key: StateKey) -> Result<(), StorageError>;

    /// Clear both state and all data for this slot (full reset).
    async fn clear_all(&self, key: StateKey) -> Result<(), StorageError>;
}

// MemoryStorage

/// An in-process, non-persistent [`StateStorage`] backed by `DashMap`.
///
/// State is lost on process restart. Suitable for development and bots that
/// do not need persistence.
///
/// `MemoryStorage` is `Send + Sync + Clone` - each clone shares the same
/// underlying map, so you can hold an `Arc<MemoryStorage>` or clone freely.
///
/// # Example
///
/// ```rust,no_run
/// use ferogram::fsm::MemoryStorage;
/// use std::sync::Arc;
///
/// let storage = Arc::new(MemoryStorage::new());
/// ```
#[derive(Clone, Default)]
pub struct MemoryStorage {
    entries: Arc<DashMap<StateKey, StorageEntry>>,
}

#[derive(Clone, Default)]
struct StorageEntry {
    state: Option<String>,
    data: HashMap<String, serde_json::Value>,
}

impl MemoryStorage {
    /// Create a new, empty in-memory storage.
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns the number of active conversation slots (any slot with state
    /// or data set).
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Returns `true` if no slots are currently active.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

#[async_trait]
impl StateStorage for MemoryStorage {
    async fn get_state(&self, key: StateKey) -> Result<Option<String>, StorageError> {
        Ok(self.entries.get(&key).and_then(|e| e.state.clone()))
    }

    async fn set_state(&self, key: StateKey, state: String) -> Result<(), StorageError> {
        self.entries.entry(key).or_default().state = Some(state);
        Ok(())
    }

    async fn clear_state(&self, key: StateKey) -> Result<(), StorageError> {
        if let Some(mut entry) = self.entries.get_mut(&key) {
            entry.state = None;
            // Remove the entry entirely if no data remains.
            if entry.data.is_empty() {
                drop(entry);
                self.entries.remove(&key);
            }
        }
        Ok(())
    }

    async fn get_data(
        &self,
        key: StateKey,
        field: &str,
    ) -> Result<Option<serde_json::Value>, StorageError> {
        Ok(self
            .entries
            .get(&key)
            .and_then(|e| e.data.get(field).cloned()))
    }

    async fn set_data(
        &self,
        key: StateKey,
        field: &str,
        value: serde_json::Value,
    ) -> Result<(), StorageError> {
        self.entries
            .entry(key)
            .or_default()
            .data
            .insert(field.to_string(), value);
        Ok(())
    }

    async fn get_all_data(
        &self,
        key: StateKey,
    ) -> Result<HashMap<String, serde_json::Value>, StorageError> {
        Ok(self
            .entries
            .get(&key)
            .map(|e| e.data.clone())
            .unwrap_or_default())
    }

    async fn clear_data(&self, key: StateKey) -> Result<(), StorageError> {
        if let Some(mut entry) = self.entries.get_mut(&key) {
            entry.data.clear();
            if entry.state.is_none() {
                drop(entry);
                self.entries.remove(&key);
            }
        }
        Ok(())
    }

    async fn clear_all(&self, key: StateKey) -> Result<(), StorageError> {
        self.entries.remove(&key);
        Ok(())
    }
}

// StateContext

/// The FSM context injected into state-matched handlers.
///
/// Provides typed access to state transitions and arbitrary key-value data
/// associated with the current conversation slot.
///
/// # Example
///
/// ```rust,no_run
/// use ferogram::fsm::{StateContext, FsmState};
/// use ferogram::FsmState;
///
/// #[derive(FsmState, Clone, Debug, PartialEq)]
/// enum ShopState { WaitingProduct, WaitingQuantity }
///
/// async fn handle(msg: ferogram::update::IncomingMessage, state: StateContext) {
///     // Store arbitrary data
///     state.set_data("product", "Widget").await.ok();
///
///     // Transition to the next state
///     state.transition(ShopState::WaitingQuantity).await.ok();
///
///     // Read data back
///     let product: Option<String> = state.get_data("product").await.unwrap_or(None);
///
///     // Get all data as a HashMap
///     let all = state.get_all_data().await.unwrap_or_default();
///
///     // Clear everything and end the FSM
///     state.clear_all().await.ok();
/// }
/// ```
#[derive(Clone)]
pub struct StateContext {
    storage: Arc<dyn StateStorage>,
    key: StateKey,
    /// The state key that matched this handler, provided as context.
    pub current_state: String,
}

impl StateContext {
    /// Construct a new `StateContext`. Called internally by the dispatcher.
    pub(crate) fn new(
        storage: Arc<dyn StateStorage>,
        key: StateKey,
        current_state: String,
    ) -> Self {
        Self {
            storage,
            key,
            current_state,
        }
    }

    /// Transition to a new state. Overwrites the current state.
    pub async fn transition(&self, new_state: impl FsmState) -> Result<(), StorageError> {
        self.storage
            .set_state(self.key.clone(), new_state.as_key())
            .await
    }

    /// Clear the current state (set to `None`). Leaves data intact.
    ///
    /// Use this to end an FSM flow without resetting gathered data.
    pub async fn clear_state(&self) -> Result<(), StorageError> {
        self.storage.clear_state(self.key.clone()).await
    }

    /// Set a typed data value for `field`. The value is serialized to JSON.
    ///
    /// # Example
    /// ```rust,no_run
    /// # async fn ex(state: ferogram::fsm::StateContext) {
    /// state.set_data("name", "Alice").await.ok();
    /// state.set_data("age", 30u32).await.ok();
    /// # }
    /// ```
    pub async fn set_data<T: Serialize>(&self, field: &str, value: T) -> Result<(), StorageError> {
        let json = serde_json::to_value(value).map_err(|e| {
            StorageError::with_source(format!("failed to serialize field `{field}`"), e)
        })?;
        self.storage.set_data(self.key.clone(), field, json).await
    }

    /// Get a typed data value for `field`. Returns `None` if not set or if
    /// deserialization fails.
    ///
    /// # Example
    /// ```rust,no_run
    /// # async fn ex(state: ferogram::fsm::StateContext) {
    /// let name: Option<String> = state.get_data("name").await.unwrap_or(None);
    /// # }
    /// ```
    pub async fn get_data<T: DeserializeOwned>(
        &self,
        field: &str,
    ) -> Result<Option<T>, StorageError> {
        let raw = self.storage.get_data(self.key.clone(), field).await?;
        match raw {
            None => Ok(None),
            Some(val) => {
                let typed = serde_json::from_value(val).map_err(|e| {
                    StorageError::with_source(format!("failed to deserialize field `{field}`"), e)
                })?;
                Ok(Some(typed))
            }
        }
    }

    /// Return all data fields as a raw JSON map.
    pub async fn get_all_data(&self) -> Result<HashMap<String, serde_json::Value>, StorageError> {
        self.storage.get_all_data(self.key.clone()).await
    }

    /// Remove all data fields. State is unchanged.
    pub async fn clear_data(&self) -> Result<(), StorageError> {
        self.storage.clear_data(self.key.clone()).await
    }

    /// Reset both state and all data (full conversation reset).
    pub async fn clear_all(&self) -> Result<(), StorageError> {
        self.storage.clear_all(self.key.clone()).await
    }

    /// The [`StateKey`] for this conversation slot.
    pub fn key(&self) -> &StateKey {
        &self.key
    }
}

impl fmt::Debug for StateContext {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("StateContext")
            .field("key", &self.key)
            .field("current_state", &self.current_state)
            .finish()
    }
}
