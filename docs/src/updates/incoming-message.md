# IncomingMessage

`IncomingMessage` is the type of `Update::NewMessage` and `Update::MessageEdited`. It wraps a raw `tl::enums::Message` and provides typed accessors plus a full suite of **convenience action methods** that let you act on a message without passing the `Client` around explicitly (when the message was received from the update stream it already carries a client reference).

---

## Basic accessors

```rust
Update::NewMessage(msg) => {
    msg.id()                  // i32: unique message ID in this chat
    msg.text()                // Option<&str>: text or media caption
    msg.peer_id()             // Option<&tl::enums::Peer>: chat this is in
    msg.sender_id()           // Option<&tl::enums::Peer>: who sent it
    msg.outgoing()            // bool: sent by us?
    msg.date()                // i32: Unix timestamp
    msg.edit_date()           // Option<i32>: last edit timestamp
    msg.mentioned()           // bool: are we @mentioned?
    msg.silent()              // bool: no notification
    msg.pinned()              // bool: currently pinned
    msg.post()                // bool: channel post (no sender)
    msg.noforwards()          // bool: forwarding disabled
    msg.from_scheduled()      // bool: was a scheduled message
    msg.edit_hide()           // bool: edit not shown in UI
    msg.media_unread()        // bool: media not yet viewed
    msg.raw                   // tl::enums::Message: full TL object
}
```

---

## Extended accessors

```rust
// Counters
msg.forward_count()          // Option<i32>: number of times forwarded
msg.view_count()             // Option<i32>: view count (channels)
msg.reply_count()            // Option<i32>: number of replies in thread
msg.reaction_count()         // i32: total reactions
msg.reply_to_message_id()    // Option<i32>: ID of the message replied to

// Typed timestamps
msg.date_utc()               // Option<DateTime<Utc>>
msg.edit_date_utc()          // Option<DateTime<Utc>>

// Rich content
msg.media()                  // Option<&tl::enums::MessageMedia>
msg.entities()               // Option<&Vec<tl::enums::MessageEntity>>
msg.action()                 // Option<&tl::enums::MessageAction>: service messages
msg.reply_markup()           // Option<&tl::enums::ReplyMarkup>
msg.forward_header()         // Option<&tl::enums::MessageFwdHeader>
msg.grouped_id()             // Option<i64>: album group ID
msg.via_bot_id()             // Option<i64>: inline bot that sent this
msg.post_author()            // Option<&str>: channel post author signature
msg.restriction_reason()     // Option<&Vec<tl::enums::RestrictionReason>>

// Formatted text helpers (requires html feature for html_text)
msg.markdown_text()          // Option<String>: entities rendered as Markdown
msg.html_text()              // Option<String>: entities rendered as HTML

// Typed media extraction
msg.photo()                  // Option<Photo>
msg.document()               // Option<Document>

// Sender details
msg.sender_user_id()         // Option<i64>: sender's user ID
msg.sender_chat_id()         // Option<i64>: sender's chat ID
msg.sender_user()            // async → Option<User> (fetches from cache/API)
```

---

## Convenience action methods

These methods work without passing a `&Client` when the message came from `stream_updates()` (the client reference is embedded). Use the `_with(client)` variants when you hold the message outside an update handler.

### Reply

```rust
// Reply to this message (reply_to set automatically)
msg.reply("Got it! ✅").await?;

// Reply with full InputMessage control
msg.reply_ex(
    InputMessage::text("Here you go")
        .silent(true)
        .no_webpage(true)
).await?;

// Explicit client variants
msg.reply_with(&client, "Hi").await?;
msg.reply_ex_with(&client, input_msg).await?;
```

### Respond (no reply thread)

```rust
// Send to the same chat without quoting
msg.respond("Hello everyone!").await?;
msg.respond_ex(InputMessage::text("...").keyboard(kb)).await?;

// Explicit client variants
msg.respond_with(&client, "Hi").await?;
msg.respond_ex_with(&client, input_msg).await?;
```

### Edit

```rust
// Edit this message's text
msg.edit("Updated content").await?;
msg.edit_with(&client, "Updated content").await?;
```

### Delete

```rust
msg.delete().await?;
msg.delete_with(&client).await?;
```

### Mark as read

```rust
msg.mark_as_read().await?;
msg.mark_as_read_with(&client).await?;
```

### Pin / Unpin

```rust
msg.pin().await?;          // pins with notification
msg.pin_with(&client).await?;

msg.unpin().await?;
msg.unpin_with(&client).await?;
```

### React

```rust
use ferogram::reactions::InputReactions;

msg.react("👍").await?;
msg.react(InputReactions::emoticon("🔥").big()).await?;
msg.react_with(&client, "❤️").await?;
```

### Forward

```rust
// Forward to another peer
msg.forward_to("@someuser").await?;
msg.forward_to_with(&client, peer).await?;
```

### Download media

```rust
// Download attached media to a file path, returns true if media existed
let downloaded = msg.download_media("output.jpg").await?;
msg.download_media_with(&client, "output.jpg").await?;
```

### Fetch replied-to message

```rust
// Get the message this one replies to
if let Some(parent) = msg.get_reply().await? {
    println!("Replying to: {}", parent.text().unwrap_or(""));
}
msg.get_reply_with(&client).await?;
```

### Refetch

```rust
// Re-fetch this message from the server (update its state)
msg.refetch().await?;
msg.refetch_with(&client).await?;
```

### Reply-to-message (via Client)

```rust
// Fetch the replied-to message via client
let parent = client.get_reply_to_message(peer.clone(), msg.id()).await?;
```

---

## Full handler example

```rust
use ferogram::{Client, update::Update};

let mut stream = client.stream_updates();
while let Some(update) = stream.next().await {
    match update {
        Update::NewMessage(msg) if !msg.outgoing() => {
            let text = msg.text().unwrap_or("");

            if text == "/start" {
                msg.reply("Welcome! 👋").await.ok();

            } else if text == "/me" {
                if let Ok(Some(user)) = msg.sender_user().await {
                    msg.reply(&format!("You are: {}", user.full_name())).await.ok();
                }

            } else if text.starts_with("/echo ") {
                let echo = &text[6..];
                msg.respond(echo).await.ok();

            } else if let Some(media) = msg.media() {
                msg.reply("Downloading…").await.ok();
                msg.download_media("received_file").await.ok();
                msg.react("✅").await.ok();
            }
        }
        _ => {}
    }
}
```
