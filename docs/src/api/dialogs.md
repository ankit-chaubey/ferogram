# Dialogs & History

---

## Fetch dialogs

```rust
// Fetch up to N dialogs (returns the most recent first)
let dialogs = client.get_dialogs(50).await?;

for d in &dialogs {
    println!("{}: {} unread: top msg: {}",
        d.title(), d.unread_count(), d.top_message());
}
```

### `Dialog` accessors

| Method | Return | Description |
|---|---|---|
| `d.title()` | `String` | Chat name |
| `d.peer()` | `Option<&tl::enums::Peer>` | The peer for this dialog |
| `d.unread_count()` | `i32` | Unread message count |
| `d.top_message()` | `i32` | ID of the latest message |

---

## `DialogIter`: lazy paginated iterator

```rust
let mut iter = client.iter_dialogs();

// Total count (available after first page is fetched)
if let Some(total) = iter.total() {
    println!("Total dialogs: {total}");
}

while let Some(dialog) = iter.next(&client).await? {
    println!("{}", dialog.title());
}
```

| Method | Description |
|---|---|
| `client.iter_dialogs()` | Create iterator |
| `iter.total()` | `Option<i32>`: total count after first fetch |
| `iter.next(&client)` | `async → Option<Dialog>` |

---

## `MessageIter`: lazy message history

```rust
let mut iter = client.iter_messages(peer.clone());

// Total count of messages in this chat
if let Some(total) = iter.total() {
    println!("Total messages: {total}");
}

while let Some(msg) = iter.next(&client).await? {
    println!("[{}] {}", msg.id, msg.message);
}
```

| Method | Description |
|---|---|
| `client.iter_messages(peer)` | Create iterator (newest first) |
| `iter.total()` | `Option<i32>`: total message count after first fetch |
| `iter.next(&client)` | `async → Option<tl::types::Message>` |

---

## Fetch messages directly

```rust
// Latest N messages from a peer
let messages = client.get_message_history(peer.clone(), 20).await?;

// Specific message IDs
let messages = client.get_messages_by_id(peer.clone(), &[100, 101, 102]).await?;
// Returns Vec<Option<tl::enums::Message>>: None if not found

// Pinned message
let pinned = client.get_pinned_message(peer.clone()).await?;

// The message a given message replies to
let parent = client.get_reply_to_message(peer.clone(), msg_id).await?;
```

---

## Scheduled messages

```rust
// List all scheduled messages
let scheduled = client.get_scheduled_messages(peer.clone()).await?;

// Send a scheduled message immediately
client.send_scheduled_now(peer.clone(), &[scheduled_id]).await?;

// Cancel a scheduled message
client.delete_scheduled_messages(peer.clone(), &[scheduled_id]).await?;
```

---

## Dialog management

```rust
// Mark all messages as read
client.mark_as_read(peer.clone()).await?;

// Clear @mention badges
client.clear_mentions(peer.clone()).await?;

// Leave and remove from dialog list
client.delete_dialog(peer.clone()).await?;

// Pin / unpin a dialog
client.pin_dialog(peer.clone()).await?;
client.unpin_dialog(peer.clone()).await?;

// Get pinned dialogs (folder_id: 0 = main, 1 = archived)
let pinned = client.get_pinned_dialogs(0).await?;

// Set manual unread flag on a dialog
client.mark_dialog_unread(peer.clone(), true).await?;

// Clear the manual unread flag
client.mark_dialog_read(peer.clone()).await?;

// Join a public group/channel
client.join_chat("@somegroup").await?;

// Accept a private invite link
client.accept_invite_link("https://t.me/joinchat/AbCdEfG").await?;

// Parse invite hash from any link format
let hash = Client::parse_invite_hash("https://t.me/+AbCdEfG12345");
```

---

## Archive & unarchive

Move a dialog into the Telegram archive (folder 1) or back to the main list (folder 0):

```rust
// Move to archive - chat disappears from the main dialog list
client.archive_chat("@somebot").await?;

// Move back to main list
client.unarchive_chat("@somebot").await?;
```

Archived chats still receive messages; they are simply hidden from the main list and muted by default until the user opens them.

### Low-level folder move

For finer control (e.g. custom folders), use `move_to_folder`:

```rust
// folder_id 0 = main list, 1 = archive
client.move_to_folder("@somebot", 0).await?;
```
