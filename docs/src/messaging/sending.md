# Sending Messages

## Basic send

```rust
// By username
client.send_message("@username", "Hello!").await?;

// To yourself (Saved Messages)
client.send_message("me", "Note to self").await?;
client.send_to_self("Quick note").await?;

// By numeric ID (string form)
client.send_message("123456789", "Hi").await?;
```

## Send to a resolved peer

```rust
let peer = client.resolve_peer("@username").await?;
client.send_message_to_peer(peer, "Hello!").await?;
```

## Rich messages with InputMessage

`InputMessage` gives you full control over formatting, entities, reply markup, and more:

```rust
use ferogram::{InputMessage, parsers::parse_markdown};

// Markdown formatting
let (text, entities) = parse_markdown("**Bold** and _italic_ and `code`");
let msg = InputMessage::text(text)
    .entities(entities);

client.send_message_to_peer_ex(peer, &msg).await?;
```

## Reply to a message

```rust
let msg = InputMessage::text("This is a reply")
    .reply_to(Some(original_message_id));

client.send_message_to_peer_ex(peer, &msg).await?;
```

## With inline keyboard

```rust
use ferogram_tl_types as tl;

let keyboard = tl::enums::ReplyMarkup::ReplyInlineMarkup(
    tl::types::ReplyInlineMarkup {
        rows: vec![
            tl::enums::KeyboardButtonRow::KeyboardButtonRow(
                tl::types::KeyboardButtonRow {
                    buttons: vec![
                        tl::enums::KeyboardButton::Callback(
                            tl::types::KeyboardButtonCallback {
                                requires_password: false,
                                style: None,
                                text: "Click me!".into(),
                                data: b"my_data".to_vec(),
                            }
                        ),
                    ]
                }
            )
        ]
    }
);

let msg = InputMessage::text("Choose an option:")
    .reply_markup(keyboard);

client.send_message_to_peer_ex(peer, &msg).await?;
```

## Delete messages

```rust
// revoke = true removes for everyone, false removes only for you
client.delete_messages(vec![msg_id_1, msg_id_2], true).await?;
```

## Fetch message history

```rust
// (peer, limit, offset_id)
// offset_id = 0 means start from the newest
let messages = client.get_messages(peer, 50, 0).await?;

for msg in messages {
    if let tl::enums::Message::Message(m) = msg {
        println!("{}: {}", m.id, m.message);
    }
}
```
