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
use std::sync::Arc;

use async_trait::async_trait;
use dashmap::DashMap;

use crate::error::StorageError;
use crate::key::StateKey;

/// Persistent storage backend for FSM state.
///
/// All methods are async and return `Result<_, StorageError>`.
/// Implement this trait to add custom backends (database, Redis, etc.).
///
/// Built-in implementations:
/// - [`MemoryStorage`] - in-process `DashMap`, zero setup, no persistence.
#[async_trait]
pub trait StateStorage: Send + Sync + 'static {
    /// Return the current state key for this slot, or `None` if no state is set.
    async fn get_state(&self, key: StateKey) -> Result<Option<String>, StorageError>;

    /// Persist a new state. Overwrites any previously set state.
    async fn set_state(&self, key: StateKey, state: String) -> Result<(), StorageError>;

    /// Clear the state for this slot. Data is NOT cleared.
    async fn clear_state(&self, key: StateKey) -> Result<(), StorageError>;

    /// Retrieve a single data field as a raw JSON value.
    async fn get_data(
        &self,
        key: StateKey,
        field: &str,
    ) -> Result<Option<serde_json::Value>, StorageError>;

    /// Persist a single data field as a raw JSON value.
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

    /// Remove all data fields for this slot. State is NOT cleared.
    async fn clear_data(&self, key: StateKey) -> Result<(), StorageError>;

    /// Clear both state and all data for this slot (full reset).
    async fn clear_all(&self, key: StateKey) -> Result<(), StorageError>;
}

/// An in-process, non-persistent [`StateStorage`] backed by `DashMap`.
///
/// State is lost on process restart. Suitable for development and bots that
/// do not need persistence.
///
/// `MemoryStorage` is `Send + Sync + Clone` - each clone shares the same
/// underlying map, so you can hold an `Arc<MemoryStorage>` or clone freely.
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

    /// Returns the number of active conversation slots.
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
