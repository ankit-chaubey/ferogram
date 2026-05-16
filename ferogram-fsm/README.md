# ferogram-fsm

FSM state management for ferogram bots.

[![Crates.io](https://img.shields.io/crates/v/ferogram-fsm?color=fc8d62)](https://crates.io/crates/ferogram-fsm)
[![Telegram](https://img.shields.io/badge/community-%40FerogramChat-2CA5E0?logo=telegram)](https://t.me/FerogramChat) [![Channel](https://img.shields.io/badge/channel-%40Ferogram-2CA5E0?logo=telegram)](https://t.me/Ferogram)
[![docs.rs](https://img.shields.io/badge/docs.rs-ferogram--fsm-5865F2)](https://docs.rs/ferogram-fsm)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](#license)

Handles state storage and context for multi-step bot conversations. Used by `ferogram`'s dispatcher when you register FSM handlers.

`ferogram` re-exports everything from here. Existing code needs no changes.

## Installation

```toml
[dependencies]
ferogram-fsm = "0.5.0"
```

## What it does

- `StateStorage` trait so you can plug in any backend
- `MemoryStorage` built-in (in-process, no persistence)
- `StateContext` for reading, writing, and transitioning state per user/chat
- `StateKey` for scoping state by user ID, chat ID, or both
- `FsmState` trait that your state enum implements

## Usage

Define a state enum and implement `FsmState` on it (or use `#[derive(FsmState)]` from `ferogram-derive`):

```rust
use ferogram_fsm::{FsmState, MemoryStorage, StateContext};
use std::sync::Arc;

#[derive(Clone, Debug, PartialEq)]
enum Order {
    Item,
    Quantity,
    Confirm,
}

impl FsmState for Order {
    fn as_key(&self) -> String {
        match self {
            Order::Item => "Item".into(),
            Order::Quantity => "Quantity".into(),
            Order::Confirm => "Confirm".into(),
        }
    }

    fn from_key(key: &str) -> Option<Self> {
        match key {
            "Item" => Some(Order::Item),
            "Quantity" => Some(Order::Quantity),
            "Confirm" => Some(Order::Confirm),
            _ => None,
        }
    }
}
```

Use `StateContext` to read, write, and move between states:

```rust
async fn handle(ctx: StateContext) {
    // Read stored data
    let item = ctx.get_data("item").await.unwrap_or_default();

    // Store data
    ctx.set_data("item", "Widget").await.ok();

    // Move to next state
    ctx.transition(Order::Quantity).await.ok();

    // Clear everything and exit the FSM
    ctx.finish().await.ok();
}
```

## Custom storage

If you need persistence, implement `StateStorage` for Redis, SQL, or whatever backend you prefer:

```rust
use ferogram_fsm::{StateStorage, StorageError};

struct RedisStorage { /* ... */ }

#[async_trait::async_trait]
impl StateStorage for RedisStorage {
    async fn get_state(&self, key: &str) -> Result<Option<String>, StorageError> { todo!() }
    async fn set_state(&self, key: &str, state: &str) -> Result<(), StorageError> { todo!() }
    async fn del_state(&self, key: &str) -> Result<(), StorageError> { todo!() }
    async fn get_data(&self, key: &str, field: &str) -> Result<Option<String>, StorageError> { todo!() }
    async fn set_data(&self, key: &str, field: &str, value: &str) -> Result<(), StorageError> { todo!() }
    async fn clear(&self, key: &str) -> Result<(), StorageError> { todo!() }
}
```

## Stack position

```
ferogram
└ ferogram-fsm  <-- here
```

## License

MIT or Apache-2.0, at your option. See [LICENSE-MIT](../LICENSE-MIT) and [LICENSE-APACHE](../LICENSE-APACHE).

**Ankit Chaubey** - [github.com/ankit-chaubey](https://github.com/ankit-chaubey)
