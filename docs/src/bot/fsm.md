# Finite State Machine (FSM)

The FSM module lets bots track per-user conversation state across multiple messages without managing a global `HashMap` manually. State is stored in a pluggable `StateStorage` backend and keyed by user ID + chat ID.

---

## Quick start

Add the `derive` feature to your `Cargo.toml`:

```toml
ferogram = { version = "0.3", features = ["derive"] }
```

Define a state enum and derive `FsmState`:

```rust
use ferogram::FsmState;

#[derive(FsmState, Clone, Debug, PartialEq)]
enum OrderState {
    AwaitingProduct,
    AwaitingQuantity,
    AwaitingConfirmation,
}
```

Wire it into the dispatcher:

```rust
use std::sync::Arc;
use ferogram::{Client, filters, fsm::MemoryStorage};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let (client, _sd) = Client::builder()
        .api_id(12345)
        .api_hash("your_hash")
        .session("bot.session")
        .connect()
        .await?;

    client.bot_sign_in("TOKEN").await?;
    client.save_session().await?;

    let mut dp = filters::Dispatcher::new();
    dp.with_state_storage(Arc::new(MemoryStorage::new()));

    // Entry point - no active state yet
    dp.on_message(filters::command("order"), |msg| async move {
        msg.reply("What product would you like?").await.ok();
    });

    // Triggered only when the user is in AwaitingProduct
    dp.on_message_fsm(filters::text(), OrderState::AwaitingProduct, |msg, state| async move {
        let product = msg.text().unwrap_or_default().to_string();
        state.set_data("product", &product).await.ok();
        state.transition(OrderState::AwaitingQuantity).await.ok();
        msg.reply("How many?").await.ok();
    });

    dp.on_message_fsm(filters::text(), OrderState::AwaitingQuantity, |msg, state| async move {
        let qty = msg.text().unwrap_or_default().to_string();
        let product: String = state.get_data("product").await.unwrap_or_default();
        state.transition(OrderState::AwaitingConfirmation).await.ok();
        msg.reply(format!("Order {} × {}. Confirm? (yes/no)", qty, product)).await.ok();
    });

    dp.on_message_fsm(filters::text(), OrderState::AwaitingConfirmation, |msg, state| async move {
        match msg.text().unwrap_or("").trim() {
            "yes" => {
                state.clear_state().await.ok();
                msg.reply("Order placed!").await.ok();
            }
            _ => {
                state.clear_state().await.ok();
                msg.reply("Order cancelled.").await.ok();
            }
        }
    });

    let mut stream = client.stream_updates();
    while let Some(upd) = stream.next().await {
        dp.dispatch(upd).await;
    }
    Ok(())
}
```

---

## `FsmState` trait

Any enum that derives `FsmState` can be used as a state discriminant:

```rust
use ferogram::FsmState;

#[derive(FsmState, Clone, Debug, PartialEq)]
enum Form {
    Name,
    Email,
    Done,
}
```

You can also implement `FsmState` manually if you need custom serialization:

```rust
impl ferogram::fsm::FsmState for Form {
    fn as_key(&self) -> String {
        format!("{:?}", self)
    }
    fn from_key(key: &str) -> Option<Self> {
        match key {
            "Name" => Some(Self::Name),
            "Email" => Some(Self::Email),
            "Done" => Some(Self::Done),
            _ => None,
        }
    }
}
```

---

## `StateContext`

Handler functions registered via `on_message_fsm` receive a `StateContext` as the second argument. It provides the following methods:

| Method | Description |
|---|---|
| `state.transition(new_state).await` | Move to a new FSM state |
| `state.clear_state().await` | Reset to no state (end the flow) |
| `state.set_data(field, value).await` | Store a serializable value |
| `state.get_data::<T>(field).await` | Retrieve a stored value |
| `state.get_all_data().await` | All stored data as `HashMap<String, Value>` |
| `state.clear_data().await` | Delete all data, keep current state |
| `state.clear_all().await` | Delete state and all associated data |
| `state.key()` | Inspect the active `StateKey` |

`set_data` / `get_data` use `serde_json` internally, so any `Serialize + DeserializeOwned` type works.

---

## State key strategies

By default, state is tracked **per user per chat**. Change the strategy on the dispatcher:

```rust
use ferogram::fsm::StateKeyStrategy;

dp.with_key_strategy(StateKeyStrategy::PerUser);    // one session per user across all chats
dp.with_key_strategy(StateKeyStrategy::PerChat);    // shared state per chat (e.g. group games)
dp.with_key_strategy(StateKeyStrategy::PerUserPerChat); // default
```

---

## Storage backends

### `MemoryStorage` (default for testing)

In-process `DashMap`-backed storage. State is lost on restart.

```rust
use ferogram::fsm::MemoryStorage;
use std::sync::Arc;

dp.with_state_storage(Arc::new(MemoryStorage::new()));
```

### Custom backend

Implement the `StateStorage` trait to persist state in Redis, a database, or any store:

```rust
use ferogram::fsm::{StateStorage, StateKey, StorageError};
use async_trait::async_trait;

struct RedisStorage { /* ... */ }

#[async_trait]
impl StateStorage for RedisStorage {
    async fn get_state(&self, key: &StateKey) -> Result<Option<String>, StorageError> { todo!() }
    async fn set_state(&self, key: &StateKey, state: &str) -> Result<(), StorageError> { todo!() }
    async fn clear_state(&self, key: &StateKey) -> Result<(), StorageError> { todo!() }
    async fn get_data(&self, key: &StateKey, field: &str) -> Result<Option<String>, StorageError> { todo!() }
    async fn set_data(&self, key: &StateKey, field: &str, value: &str) -> Result<(), StorageError> { todo!() }
    async fn clear_data(&self, key: &StateKey, field: &str) -> Result<(), StorageError> { todo!() }
    async fn clear_all_data(&self, key: &StateKey) -> Result<(), StorageError> { todo!() }
}
```

---

## Handler signature

FSM handlers take two arguments: the message and the state context:

```rust
dp.on_message_fsm(filter, MyState::SomeVariant, |msg, state| async move {
    // msg  : ferogram::update::IncomingMessage
    // state: ferogram::fsm::StateContext
});
```

For edited messages use `on_edit_fsm` with the same signature.

---

## Routers with FSM

Routers support FSM handlers too, which is useful for splitting a bot into feature modules:

```rust
use ferogram::filters::Router;
use std::sync::Arc;

fn order_router() -> Router {
    let mut r = Router::new();
    r.on_message_fsm(filters::text(), OrderState::AwaitingProduct, handle_product);
    r.on_message_fsm(filters::text(), OrderState::AwaitingQuantity, handle_qty);
    r
}

// Then include in the dispatcher:
dp.include(order_router());
```
