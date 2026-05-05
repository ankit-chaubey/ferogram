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

use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;

use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::FsmState;
use crate::error::StorageError;
use crate::key::StateKey;
use crate::storage::StateStorage;

/// The FSM context injected into state-matched handlers.
///
/// Provides typed access to state transitions and arbitrary key-value data
/// associated with the current conversation slot.
#[derive(Clone)]
pub struct StateContext {
    storage: Arc<dyn StateStorage>,
    key: StateKey,
    /// The state key that matched this handler, provided as context.
    pub current_state: String,
}

impl StateContext {
    /// Construct a new `StateContext`. Called internally by the dispatcher.
    pub fn new(storage: Arc<dyn StateStorage>, key: StateKey, current_state: String) -> Self {
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
    pub async fn clear_state(&self) -> Result<(), StorageError> {
        self.storage.clear_state(self.key.clone()).await
    }

    /// Set a typed data value for `field`. The value is serialized to JSON.
    pub async fn set_data<T: Serialize>(&self, field: &str, value: T) -> Result<(), StorageError> {
        let json = serde_json::to_value(value).map_err(|e| {
            StorageError::with_source(format!("failed to serialize field `{field}`"), e)
        })?;
        self.storage.set_data(self.key.clone(), field, json).await
    }

    /// Get a typed data value for `field`. Returns `None` if not set.
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
