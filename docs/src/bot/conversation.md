# Conversation API

The `Conversation` type provides a high-level, blocking-style interface for multi-step bot flows: send a question, wait for the answer, send the next question, and so on - all within a single `async fn`, without manually tracking state.

---

## Overview

`Conversation` wraps a mutable reference to an `UpdateStream` and filters updates to a single peer. Updates from other peers are buffered internally and can be retrieved with `drain_buffered()`.

---

## Quick start

```rust
use std::time::Duration;
use ferogram::conversation::Conversation;

// In a handler or task that already owns the update stream:
let mut conv = Conversation::new(&client, &mut stream, "@username").await?;

conv.ask("What is your name?").await?;
let name_msg = conv.get_response(Duration::from_secs(60)).await?;
let name = name_msg.text().unwrap_or("unknown").to_string();

conv.ask(format!("Nice to meet you, {}! How old are you?", name)).await?;
let age_msg = conv.get_response(Duration::from_secs(60)).await?;
let age = age_msg.text().unwrap_or("?").to_string();

conv.respond(format!("Got it: {} is {} years old.", name, age)).await?;
```

---

## Creating a `Conversation`

```rust
use ferogram::conversation::Conversation;

let mut conv = Conversation::new(&client, &mut stream, peer).await?;
```

`peer` accepts anything that implements `Into<PeerRef>`: a `&str` username, a numeric ID, or a resolved `tl::enums::Peer`.

---

## Sending messages

| Method | Description |
|---|---|
| `conv.ask(text).await` | Send a message and return the sent `IncomingMessage` |
| `conv.respond(text).await` | Alias for `ask` |

Both accept any `Into<String>`.

---

## Waiting for responses

| Method | Description |
|---|---|
| `conv.get_response(deadline).await` | Wait for the next message from the peer |
| `conv.wait_click(deadline).await` | Wait for the peer to press an inline button |
| `conv.wait_read(deadline).await` | Wait until messages are read (any non-message update from peer) |
| `conv.ask_and_wait(text, deadline).await` | Send a message and immediately wait for the reply |

`deadline` is a `std::time::Duration`. If no response arrives within the deadline, the method returns `ConversationError::Timeout`.

Non-matching updates (from other peers, or update types other than `NewMessage` / `CallbackQuery`) are buffered. Retrieve them with:

```rust
let leftover: Vec<Update> = conv.drain_buffered();
```

---

## Error handling

```rust
use ferogram::conversation::ConversationError;

match conv.get_response(Duration::from_secs(30)).await {
    Ok(msg) => { /* process msg */ }
    Err(ConversationError::Timeout(d)) => {
        conv.respond("You took too long! Please try again.").await.ok();
    }
    Err(ConversationError::StreamClosed) => { /* bot is shutting down */ }
    Err(ConversationError::Invocation(e)) => { return Err(e.into()); }
}
```

---

## Button interaction example

```rust
use std::time::Duration;
use ferogram::{InputMessage, keyboard::{Button, InlineKeyboard}};
use ferogram::conversation::Conversation;

let mut conv = Conversation::new(&client, &mut stream, peer).await?;

let kb = InlineKeyboard::new()
    .row([
        Button::callback("✅ Yes", b"yes"),
        Button::callback("❌ No",  b"no"),
    ]);

conv.ask(InputMessage::text("Confirm your order?").keyboard(kb)).await?;

let click = conv.wait_click(Duration::from_secs(120)).await?;
match click.data().unwrap_or("") {
    "yes" => conv.respond("Order confirmed!").await?,
    _     => conv.respond("Order cancelled.").await?,
};
```

---

## Integrating with the dispatcher

Because `Conversation` borrows `&mut UpdateStream` exclusively, you typically use it in a handler that was given the stream, or spin up a dedicated task:

```rust
use std::sync::Arc;
use tokio::sync::Mutex;

// Give one user their own conversation task
let stream = Arc::new(Mutex::new(client.stream_updates()));

// … in a handler triggered by /start:
let client2 = client.clone();
let stream2 = stream.clone();
let user_peer = msg.peer_id().cloned().unwrap();

tokio::spawn(async move {
    let mut locked = stream2.lock().await;
    if let Ok(mut conv) = Conversation::new(&client2, &mut locked, user_peer).await {
        run_onboarding(&mut conv).await.ok();
    }
});
```

For multi-user bots, the FSM approach is usually a better fit - see [FSM](./fsm.md). Use `Conversation` when the flow is short and you need the simplicity of sequential `await` calls.

---

## Full example: simple registration flow

```rust
use std::time::Duration;
use ferogram::conversation::{Conversation, ConversationError};

async fn registration_flow(
    client: &ferogram::Client,
    stream: &mut ferogram::UpdateStream,
    peer: ferogram::tl::enums::Peer,
) -> Result<(), ConversationError> {
    let mut conv = Conversation::new(client, stream, peer).await?;
    let timeout = Duration::from_secs(120);

    conv.ask("Welcome! Please enter your first name:").await?;
    let first = conv.get_response(timeout).await?;
    let first_name = first.text().unwrap_or("").trim().to_string();
    if first_name.is_empty() {
        conv.respond("Name cannot be empty. Please /start again.").await?;
        return Ok(());
    }

    conv.ask("Great! Now your email address:").await?;
    let email_msg = conv.get_response(timeout).await?;
    let email = email_msg.text().unwrap_or("").trim().to_string();

    conv.respond(format!(
        "Registered: {} <{}>. Welcome aboard!", first_name, email
    )).await?;

    Ok(())
}
```
