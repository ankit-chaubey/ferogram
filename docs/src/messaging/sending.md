# Sending Messages

## Basic send

```rust
// By username
client.send_message("@username", "Hello!").await?;

// To yourself (Saved Messages)
client.send_message("me", "Note to self").await?;
client.send_to_self("Quick note").await?;

// By numeric ID
client.send_message(123456789_i64, "Hi").await?;
```

`send_message` accepts anything that implements `Into<PeerRef>` as the peer (username string, numeric ID, or resolved `tl::enums::Peer`). The second argument accepts anything that implements `Into<InputMessage>`, including a bare `&str` or `String` for plain text.

---

## Rich messages with InputMessage

`InputMessage` gives full control over formatting, entities, reply markup, and more:

```rust
use ferogram::InputMessage;

// Plain text
client.send_message("@peer", InputMessage::text("Hello!")).await?;

// Markdown-formatted
client.send_message("@peer", InputMessage::markdown("**Bold** and _italic_ and `code`")).await?;

// HTML-formatted (requires `html` feature)
client.send_message("@peer", InputMessage::html("Hello <b>world</b>")).await?;

// Reply to a message
let msg = InputMessage::text("This is a reply").reply_to(Some(original_id));
client.send_message("@peer", msg).await?;

// Silent message (no notification)
let msg = InputMessage::text("Quiet update").silent(true);
client.send_message("@peer", msg).await?;
```

---

## Message shorthands on `IncomingMessage`

Every received message exposes shorthand methods that embed the client:

```rust
// Quote-reply in the same chat
msg.reply(InputMessage::text("Got it!")).await?;

// Send to the same chat without quoting
msg.respond("Noted.").await?;

// Edit the message
msg.edit(InputMessage::text("Updated!")).await?;

// Forward to another chat
msg.forward_to("@other_chat").await?;

// Delete
msg.delete().await?;

// Pin / unpin
msg.pin().await?;
msg.unpin().await?;

// Mark as read
msg.mark_as_read().await?;

// Reload from server
msg.refetch().await?;

// Fetch the message being replied to
if let Some(parent) = msg.get_reply().await? {
    println!("Replied to: {}", parent.text().unwrap_or(""));
}
```

All of these have explicit `_with(client, ...)` variants for use outside handler closures.

---

## Inline keyboards with InputMessage

```rust
use ferogram::{InputMessage, InlineKeyboard, Button};

let kb = InlineKeyboard::new()
    .row([
        Button::callback("✅ Yes", b"yes"),
        Button::callback("❌ No",  b"no"),
    ]);

let msg = InputMessage::text("Confirm?").reply_markup(kb);
client.send_message("@peer", msg).await?;
```

---

## Clicking inline buttons

Three targeting modes via `ButtonFilter`:

```rust
use ferogram::update::ButtonFilter;

// By position (row, col - 0-based)
msg.click_button(ButtonFilter::Pos(0, 0)).await?;

// By exact button label
msg.click_button(ButtonFilter::Text("✅ Yes")).await?;

// By callback data bytes
msg.click_button(ButtonFilter::Data(b"action:buy")).await?;

// By arbitrary predicate
msg.click_button_where(|text, data| text.starts_with("✅")).await?;
```

`find_button` and `find_button_where` return `Option<(row, col)>` without sending anything.

---

## Edit, forward, delete, pin (standalone client methods)

```rust
// Edit by message ID
client.edit_message("@peer", msg_id, InputMessage::text("New text")).await?;

// Forward messages
client.forward_messages("@source", &[id1, id2], "@dest").await?;

// Delete messages (revoke = remove for everyone)
client.delete_messages(&[msg_id_1, msg_id_2], true).await?;

// Pin / unpin
client.pin_message("@peer", msg_id).await?;
client.unpin_message("@peer", msg_id).await?;
client.unpin_all_messages("@peer").await?;

// Get pinned message
let pinned = client.get_pinned_message("@peer").await?;

// Mark as read
client.mark_as_read("@peer").await?;

// Export a permanent message link
let link = client.export_message_link("@peer", msg_id).await?;

// Who has read a message (groups/channels with read receipts)
let readers = client.get_message_read_participants("@peer", msg_id).await?;
```

---

## Fetch message history

```rust
// get_message_history(peer, limit, offset_id)
// offset_id = 0 starts from the newest
let messages = client.get_message_history("@peer", 50, 0).await?;

for msg in messages {
    println!("{}: {}", msg.id(), msg.text().unwrap_or(""));
}

// Lazy iterator (auto-paginating)
let mut iter = client.iter_messages("@peer");
while let Some(msg) = iter.next(&client).await {
    println!("{}", msg.text().unwrap_or(""));
}
```

---

## Scheduled messages

```rust
use ferogram::InputMessage;

// Schedule at a Unix timestamp
let msg = InputMessage::text("Happy New Year!").schedule_date(Some(1735689600));
client.send_message("@peer", msg).await?;

// Schedule to send when the recipient comes online
let msg = InputMessage::text("Hey!").schedule_once_online();
client.send_message("@peer", msg).await?;

// List scheduled messages
let scheduled = client.get_scheduled_messages("@peer").await?;

// Send a scheduled message immediately
client.send_scheduled_now("@peer", &[scheduled_id]).await?;

// Cancel a scheduled message
client.delete_scheduled_messages("@peer", &[scheduled_id]).await?;
```

---

## Drafts

```rust
// Save or update a draft
client.save_draft("@peer", "Draft text").await?;

// Trigger server push of all drafts as update events
client.sync_drafts().await?;

// Delete all drafts
client.clear_all_drafts().await?;
```
