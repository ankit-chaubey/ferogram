# Callback Queries

When a user taps an inline keyboard button, the bot receives `Update::CallbackQuery`. Always answer it: Telegram shows a loading spinner until you do.

---

## Basic handling

```rust
Update::CallbackQuery(cb) => {
    let data = cb.data().unwrap_or("");

    match data {
        "vote:yes" => cb.answer().text("✅ Voted yes!").send(&client).await?,
        "vote:no"  => cb.answer().text("❌ Voted no").send(&client).await?,
        _          => cb.answer().send(&client).await?,  // empty answer
    }
}
```

---

## `CallbackQuery` fields

```rust
cb.query_id       // i64: must be passed to answer_callback_query
cb.user_id        // i64: who pressed the button
cb.msg_id         // Option<i32>: message the button was on
cb.data()         // Option<&str>: the callback data string
cb.inline_msg_id  // Option<tl::enums::InputBotInlineMessageID>: for inline messages
```

---

## `Answer` builder (fluent API)

`cb.answer()` returns an `Answer` builder. Chain modifiers then call `.send(&client)`.

```rust
// Toast notification (default)
cb.answer()
    .text("✅ Done!")
    .send(&client)
    .await?;

// Alert popup (modal dialog)
cb.answer()
    .alert("⛔ You don't have permission for this action.")
    .send(&client)
    .await?;

// Open a URL (Telegram shows a confirmation first)
cb.answer()
    .url("https://example.com/auth")
    .send(&client)
    .await?;

// Silent answer (no notification, just clears the spinner)
cb.answer().send(&client).await?;
```

### `Answer` methods

| Method | Description |
|---|---|
| `cb.answer()` | Start building an answer |
| `.text(str)` | Toast message shown to the user |
| `.alert(str)` | Modal popup shown to the user |
| `.url(str)` | URL to open (with user confirmation) |
| `.cache_time(Duration)` | How long Telegram caches this answer client-side (default: 0) |
| `.send(&client)` | Execute: always call this |

---

## Convenience shortcuts

```rust
// Flat answer with optional text
cb.answer_flat(&client, Some("✅ Done")).await?;
cb.answer_flat(&client, None).await?;  // silent

// Alert shortcut
cb.answer_alert(&client, "⛔ Access denied").await?;
```

---

## Via `Client` directly

```rust
// answer_callback_query(query_id, text, alert)
client.answer_callback_query(cb.query_id, Some("✅ Done!"), false).await?;
client.answer_callback_query(cb.query_id, Some("⛔ No!"), true).await?;  // alert
client.answer_callback_query(cb.query_id, None, false).await?;  // silent
```

---

## Edit the message on callback

```rust
Update::CallbackQuery(cb) => {
    // Answer first (clears spinner)
    cb.answer().text("Loading…").send(&client).await?;

    // Then edit the original message
    if let Some(msg_id) = cb.msg_id {
        client.edit_message(
            // peer from the callback: resolve from context
            peer.clone(),
            msg_id,
            "Updated content",
        ).await?;
    }
}
```

---

## Full example: vote bot

```rust
Update::CallbackQuery(cb) => {
    match cb.data().unwrap_or("") {
        "vote:yes" => {
            cb.answer().text("Thanks for voting Yes! 👍").send(&client).await?;
        }
        "vote:no" => {
            cb.answer().text("Thanks for voting No! 👎").send(&client).await?;
        }
        "vote:info" => {
            cb.answer()
                .url("https://example.com/vote-info")
                .send(&client)
                .await?;
        }
        _ => {
            cb.answer().send(&client).await?; // always answer
        }
    }
}
```
