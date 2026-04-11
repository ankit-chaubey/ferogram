# Typing Guard

`TypingGuard` is a RAII wrapper that keeps a "typing…" or "uploading…" indicator alive for the duration of an operation and **automatically cancels it when dropped**. You never need to call `SetTyping` with `CancelAction` by hand.

The guard re-sends the action every **4 seconds** (Telegram drops indicators after ~5 s) until the guard is dropped or `.cancel()` is called.

---

## Setup

`TypingGuard` is re-exported from `ferogram`: no extra import needed beyond `use ferogram::TypingGuard;`.

---

## Convenience methods on `Client`

These are the recommended entry points:

```rust
use ferogram::{Client, TypingGuard};

// "typing…"
let _typing = client.typing(peer.clone()).await?;

// "uploading document…"
let _typing = client.uploading_document(peer.clone()).await?;

// "recording video…"
let _typing = client.recording_video(peer.clone()).await?;

// typing inside a forum topic (top_msg_id)
let _typing = client.typing_in_topic(peer.clone(), topic_id).await?;
```

The guard auto-cancels when it goes out of scope.

---

## Using `TypingGuard::start` directly

For any `SendMessageAction` variant: including ones that don't have a convenience method:

```rust
use ferogram::TypingGuard;
use ferogram_tl_types as tl;

// Record audio / voice message
let _guard = TypingGuard::start(
    client,
    peer.clone(),
    tl::enums::SendMessageAction::SendMessageRecordAudioAction,
).await?;

// Upload photo
let _guard = TypingGuard::start(
    client,
    peer.clone(),
    tl::enums::SendMessageAction::SendMessageUploadPhotoAction(
        tl::types::SendMessageUploadPhotoAction { progress: 0 },
    ),
).await?;

// Choose sticker
let _guard = TypingGuard::start(
    client,
    peer.clone(),
    tl::enums::SendMessageAction::SendMessageChooseStickerAction,
).await?;
```

---

## `TypingGuard::start_ex`: forum topics + custom delay

```rust
use std::time::Duration;

let _guard = TypingGuard::start_ex(
    client,
    peer,                   // tl::enums::Peer (already resolved)
    tl::enums::SendMessageAction::SendMessageTypingAction,
    Some(topic_msg_id),     // top_msg_id: None for normal chats
    Duration::from_secs(4), // repeat delay (≤ 4 s recommended)
).await?;
```

---

## Manual `.cancel()`

Call `.cancel()` to stop the indicator immediately without waiting for the guard to drop:

```rust
let mut guard = client.typing(peer.clone()).await?;

do_some_work().await;

guard.cancel(); // indicator stops here

// guard still lives, but the task is already stopped
send_reply(client, peer).await?;
```

---

## One-shot `send_chat_action` (no guard)

If you don't need the automatic renewal, fire a single action:

```rust
client.send_chat_action(
    peer.clone(),
    tl::enums::SendMessageAction::SendMessageTypingAction,
    None, // top_msg_id: Some(id) for forum topics
).await?;
```

Telegram shows the indicator for ~5 seconds and then removes it automatically.

---

## Complete example: long task with typing

```rust
use ferogram::{Client, InvocationError};
use ferogram_tl_types as tl;

async fn handle_command(
    client: &Client,
    peer: tl::enums::Peer,
) -> Result<(), InvocationError> {
    // Indicator starts immediately, renewed every 4 s
    let _typing = client.typing(peer.clone()).await?;

    // Simulate a slow operation
    let result = compute_answer().await;

    // _typing drops here → indicator cancelled
    client
        .send_message_to_peer(peer, &result)
        .await?;

    Ok(())
}

async fn compute_answer() -> String {
    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    "Here is your answer!".into()
}
```

---

## Example: upload with "uploading document…" indicator

```rust
async fn send_document(
    client: &Client,
    peer: tl::enums::Peer,
    bytes: Vec<u8>,
    filename: &str,
) -> Result<(), InvocationError> {
    // Show "uploading document…" while the upload runs
    let _guard = client.uploading_document(peer.clone()).await?;

    let uploaded = client.upload_file(filename, &bytes).await?;

    drop(_guard); // cancel the indicator before sending

    client.send_file(peer, uploaded, false).await?;
    Ok(())
}
```

---

## How it works internally

1. `start()` calls `send_chat_action_ex(peer, action, topic_id)` immediately.
2. A `tokio::spawn` loop wakes every `repeat_delay` (default 4 s) and re-sends the action.
3. A `tokio::sync::Notify` signals the loop to stop when the guard is dropped or `.cancel()` is called.
4. On loop exit, `SendMessageCancelAction` is sent to immediately clear the indicator.

---

## API reference

| Symbol | Kind | Description |
|---|---|---|
| `TypingGuard` | struct | RAII guard; drop to cancel |
| `TypingGuard::start(client, peer, action)` | `async fn` | Start any `SendMessageAction` |
| `TypingGuard::start_ex(client, peer, action, topic_id, delay)` | `async fn` | Full control: topic support + custom repeat delay |
| `guard.cancel()` | `fn` | Stop the indicator immediately (guard stays alive) |
| `client.typing(peer)` | `async fn` | Shorthand for `TypingAction` |
| `client.uploading_document(peer)` | `async fn` | Shorthand for `UploadDocumentAction` |
| `client.recording_video(peer)` | `async fn` | Shorthand for `RecordVideoAction` |
| `client.typing_in_topic(peer, topic_id)` | `async fn` | Typing inside a forum topic thread |
