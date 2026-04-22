# Middleware & Dispatcher

ferogram ships a `Dispatcher` that routes incoming updates to typed handlers, and a middleware chain that intercepts every update before it reaches any handler.

---

## Dispatcher basics

```rust
use ferogram::filters::{self, Dispatcher};

let mut dp = Dispatcher::new();

// Handle /start command (bots)
dp.on_message(filters::command("start"), |msg| async move {
    msg.reply("Hello!").await.ok();
});

// Handle any private text
dp.on_message(filters::private() & filters::text(), |msg| async move {
    let echo = msg.text().unwrap_or_default().to_string();
    msg.reply(echo).await.ok();
});

// Drive the dispatcher from the update stream
let mut stream = client.stream_updates();
while let Some(upd) = stream.next().await {
    dp.dispatch(upd).await;
}
```

`dispatch` is `async` and runs handlers serially per update. For concurrent handling, spawn each `dispatch` call:

```rust
use std::sync::Arc;

let dp = Arc::new(dp);
let mut stream = client.stream_updates();
while let Some(upd) = stream.next().await {
    let dp = dp.clone();
    tokio::spawn(async move { dp.dispatch(upd).await; });
}
```

---

## Built-in filters

All filters return a `BoxFilter` which supports `&` (AND), `|` (OR), and `!` (NOT).

| Filter | Description |
|---|---|
| `all()` | Always matches |
| `none()` | Never matches |
| `private()` | Message is in a private chat (DM) |
| `group()` | Message is in a basic group |
| `channel()` | Message is in a channel |
| `text()` | Message has non-empty text |
| `media()` | Message has any media attachment |
| `photo()` | Message contains a photo |
| `document()` | Message contains a document / file |
| `forwarded()` | Message was forwarded |
| `reply()` | Message is a reply to another message |
| `album()` | Message is part of a media album |
| `any_command()` | Message starts with any `/command` |
| `command("start")` | Message is exactly the `/start` command |
| `text_contains("word")` | Message text contains the substring |
| `text_starts_with("!")` | Message text starts with the prefix |
| `from_user(user_id)` | Message was sent by a specific user ID |
| `in_chat(chat_id)` | Message is from a specific chat ID |
| `custom(|msg| bool)` | Arbitrary predicate closure |

### Combining filters

```rust
// Both conditions must be true
let f = filters::private() & filters::text();

// Either condition must be true
let f = filters::command("help") | filters::command("start");

// Negate a filter
let f = !filters::forwarded();

// Complex expressions
let f = (filters::group() | filters::channel()) & !filters::forwarded();
```

---

## Dispatcher methods

| Method | Description |
|---|---|
| `dp.on_message(filter, handler)` | Register a handler for new messages |
| `dp.on_edit(filter, handler)` | Register a handler for edited messages |
| `dp.on_message_fsm(filter, state, handler)` | FSM-gated message handler (see [FSM](./fsm.md)) |
| `dp.on_edit_fsm(filter, state, handler)` | FSM-gated edit handler |
| `dp.middleware(mw)` | Prepend middleware to the chain |
| `dp.with_state_storage(storage)` | Set the FSM storage backend |
| `dp.with_key_strategy(strategy)` | Set the FSM key strategy |
| `dp.include(router)` | Mount a `Router` sub-tree |
| `dp.dispatch(update).await` | Route an update through the chain |

---

## Routers

`Router` lets you split a large bot into feature modules with their own handlers and optional scoped filters.

```rust
use ferogram::filters::{Router, command, private};

pub fn admin_router() -> Router {
    let mut r = Router::new();
    // Only handle /ban and /kick from private chats
    r.scope(private());
    r.on_message(command("ban"),  handle_ban);
    r.on_message(command("kick"), handle_kick);
    r
}

// In main:
dp.include(admin_router());
```

`scope(filter)` adds a guard that runs before any handler in the router. If the filter does not match, the router is skipped entirely.

---

## Middleware

Middleware intercepts every `Update` before it reaches a handler. Common use cases include logging, rate-limiting, authentication, and metrics.

### Implementing `Middleware`

```rust
use ferogram::middleware::{Middleware, Next, BoxFuture, DispatchResult};
use ferogram::update::Update;

struct LoggingMiddleware;

impl Middleware for LoggingMiddleware {
    fn call(&self, update: Update, next: Next) -> BoxFuture {
        Box::pin(async move {
            println!("Update: {:?}", update);
            let result = next.run(update).await;
            if let Err(ref e) = result {
                println!("Handler error: {e}");
            }
            result
        })
    }
}
```

### Registering middleware

```rust
dp.middleware(LoggingMiddleware);
```

Middleware runs in **registration order**: the first `middleware()` call wraps the outermost layer.

### `Next`

`Next` represents the remainder of the chain. Call `next.run(update).await` to pass control forward. If you do not call it, all remaining middleware and the handler are skipped.

```rust
impl Middleware for AuthMiddleware {
    fn call(&self, update: Update, next: Next) -> BoxFuture {
        Box::pin(async move {
            if let Update::NewMessage(ref msg) = update {
                if !is_allowed(msg.sender_user_id()) {
                    return Ok(()); // block: do not call next
                }
            }
            next.run(update).await
        })
    }
}
```

### Closure middleware

For simple cases, pass a closure directly via the `middleware_fn` helper (if available) or wrap in a newtype:

```rust
struct TimerMiddleware;

impl Middleware for TimerMiddleware {
    fn call(&self, update: Update, next: Next) -> BoxFuture {
        Box::pin(async move {
            let start = std::time::Instant::now();
            let result = next.run(update).await;
            println!("Handled in {:?}", start.elapsed());
            result
        })
    }
}

dp.middleware(TimerMiddleware);
```

---

## Full example: rate-limiter middleware

```rust
use dashmap::DashMap;
use ferogram::middleware::{BoxFuture, DispatchResult, Middleware, Next};
use ferogram::update::Update;
use std::sync::Arc;
use std::time::{Duration, Instant};

struct RateLimiter {
    last_seen: Arc<DashMap<i64, Instant>>,
    period: Duration,
}

impl RateLimiter {
    fn new(period: Duration) -> Self {
        Self { last_seen: Arc::new(DashMap::new()), period }
    }
}

impl Middleware for RateLimiter {
    fn call(&self, update: Update, next: Next) -> BoxFuture {
        let last_seen = self.last_seen.clone();
        let period = self.period;
        Box::pin(async move {
            if let Update::NewMessage(ref msg) = update {
                if let Some(uid) = msg.sender_user_id() {
                    let now = Instant::now();
                    if let Some(t) = last_seen.get(&uid) {
                        if now.duration_since(*t) < period {
                            return Ok(()); // drop: too fast
                        }
                    }
                    last_seen.insert(uid, now);
                }
            }
            next.run(update).await
        })
    }
}

// Register: limit each user to one message per second
dp.middleware(RateLimiter::new(Duration::from_secs(1)));
```

---

## Handler signatures

All handlers must be `async fn` or closures returning a `Future`:

```rust
// No arguments (ignores the message)
dp.on_message(filters::command("ping"), || async {
    // nothing
});

// Message only
dp.on_message(filters::text(), |msg| async move {
    println!("{:?}", msg.text());
});
```

The dispatcher calls handlers with `msg.clone()` so handlers can be registered multiple times. Handlers do not need to return a value; errors should be handled internally (e.g. `.ok()` on send results).
