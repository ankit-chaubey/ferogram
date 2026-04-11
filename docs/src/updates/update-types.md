# Update Types

`stream.next().await` yields `Option<Update>`. `Update` is `#[non_exhaustive]`: always include `_ => {}`.

```rust
use ferogram::update::Update;

while let Some(update) = stream.next().await {
    match update {
        // Messages
        Update::NewMessage(msg)     => { /* IncomingMessage */ }
        Update::MessageEdited(msg)  => { /* IncomingMessage */ }
        Update::MessageDeleted(del) => { /* MessageDeletion */ }

        // Bot interactions
        Update::CallbackQuery(cb)   => { /* CallbackQuery */ }
        Update::InlineQuery(iq)     => { /* InlineQuery */ }
        Update::InlineSend(is)      => { /* InlineSend */ }

        // Presence
        Update::UserTyping(action)  => { /* ChatActionUpdate */ }
        Update::UserStatus(status)  => { /* UserStatusUpdate */ }

        // Raw passthrough
        Update::Raw(raw)            => { /* RawUpdate */ }

        _ => {}  // required: Update is #[non_exhaustive]
    }
}
```

---

## `MessageDeletion`

```rust
Update::MessageDeleted(del) => {
    let ids: Vec<i32> = del.into_messages();
}
```

| Method | Return | Description |
|---|---|---|
| `del.into_messages()` | `Vec<i32>` | IDs of deleted messages |

---

## `CallbackQuery`

See the full [Callback Queries](./callbacks.md) page.

```rust
cb.query_id      // i64
cb.user_id       // i64
cb.msg_id        // Option<i32>
cb.data()        // Option<&str>
cb.answer()      // → Answer builder
cb.answer_flat(&client, text)
cb.answer_alert(&client, text)
```

---

## `InlineQuery`

```rust
iq.query_id      // i64
iq.user_id       // i64
iq.query()       // &str: the typed query
iq.offset        // String: pagination offset
```

Answer with `client.answer_inline_query(...)`. See [Inline Mode](./inline-mode.md).

---

## `InlineSend`

Fires when a user picks a result from your bot's inline mode.

```rust
is.result_id     // String: which result was chosen
is.user_id       // i64
is.query         // String: original query

// Edit the message the inline result was sent as
is.edit_message(&client, updated_input_msg).await?;
```

---

## `ChatActionUpdate` (UserTyping)

```rust
Update::UserTyping(action) => {
    action.peer      // tl::enums::Peer: the chat
    action.user_id   // Option<i64>
    action.action    // tl::enums::SendMessageAction
}
```

---

## `UserStatusUpdate`

```rust
Update::UserStatus(status) => {
    status.user_id  // i64
    status.status   // tl::enums::UserStatus
    // variants: UserStatusOnline, UserStatusOffline, UserStatusRecently, etc.
}
```

---

## `RawUpdate`

Any TL update that doesn't map to a typed variant:

```rust
Update::Raw(raw) => {
    raw.update   // tl::enums::Update: the raw TL object
}
```

---

## Raw update stream

If you need all updates unfiltered:

```rust
let mut stream = client.stream_updates();
while let Some(raw) = stream.next_raw().await {
    println!("{:?}", raw.update);
}
```
