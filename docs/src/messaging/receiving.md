# Receiving Updates

## The update stream

`stream_updates()` returns an async stream of typed `Update` events:

```rust
use ferogram::update::Update;

let mut updates = client.stream_updates();

while let Some(update) = updates.next().await {
    match update {
        Update::NewMessage(msg)     => { /* new message arrived */ }
        Update::MessageEdited(msg)  => { /* message was edited */ }
        Update::MessageDeleted(del) => { /* message was deleted */ }
        Update::CallbackQuery(cb)   => { /* inline button pressed */ }
        Update::InlineQuery(iq)     => { /* @bot query in another chat */ }
        Update::InlineSend(is)      => { /* inline result was chosen */ }
        Update::Raw(raw)            => { /* any other update by constructor ID */ }
        _ => {}
    }
}
```

## Concurrent update handling

For bots under load, spawn each update into its own task so the receive loop never blocks:

```rust
use std::sync::Arc;

let client = Arc::new(client);
let mut updates = client.stream_updates();

while let Some(update) = updates.next().await {
    let client = client.clone();
    tokio::spawn(async move {
        handle(update, client).await;
    });
}
```

## Filtering outgoing messages

In user accounts, your own sent messages come back as updates with `out = true`. Filter them:

```rust
Update::NewMessage(msg) if !msg.outgoing() => {
    // only incoming messages
}
```

## MessageDeleted

Deleted message updates only contain the message IDs, not the content:

```rust
Update::MessageDeleted(del) => {
    println!("Deleted IDs: {:?}", del.messages());
    // del.channel_id(): Some if deleted from a channel
}
```
