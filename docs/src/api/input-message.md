# InputMessage Builder

`InputMessage` is a fluent builder for composing rich messages with full control over every parameter.

## Import

```rust
use ferogram::InputMessage;
use ferogram::parsers::parse_markdown;
```

## Builder methods

| Method | Type | Description |
|---|---|---|
| `InputMessage::text(text)` | `impl Into<String>` | Create with plain text (constructor) |
| `.set_text(text)` | `impl Into<String>` | Replace the text |
| `.entities(entities)` | `Vec<MessageEntity>` | Formatting entities from `parse_markdown` |
| `.reply_to(id)` | `Option<i32>` | Reply to a message ID |
| `.reply_markup(markup)` | `ReplyMarkup` | Inline or reply keyboard |
| `.silent(v)` | `bool` | Send without notification |
| `.background(v)` | `bool` | Send as background message |
| `.clear_draft(v)` | `bool` | Clear the chat draft on send |
| `.no_webpage(v)` | `bool` | Disable link preview |
| `.schedule_date(ts)` | `Option<i32>` | Unix timestamp to schedule the send |

## Plain text

```rust
let msg = InputMessage::text("Hello, world!");
client.send_message(peer, msg).await?;
```

## Markdown formatting

`parse_markdown` converts Markdown to plain text + entity list:

```rust
let (plain, entities) = parse_markdown(
    "**Bold**, _italic_, `inline code`, and [a link](https://example.com)"
);
let msg = InputMessage::text(plain).entities(entities);
```

Supported Markdown syntax:

| Syntax | Result |
|---|---|
| `**text**` | **Bold** |
| `_text_` or `*text*` | _Italic_ |
| `\`text\`` | `Inline code` |
| `\`\`\`text\`\`\`` | Pre-formatted block |
| `[label](url)` | Hyperlink |
| `__text__` | Underline |
| `~~text~~` | Strikethrough |
| `\|\|text\|\|` | Spoiler |

## Reply to a message

```rust
let msg = InputMessage::text("This is my reply")
    .reply_to(Some(original_msg_id));
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
                                text: "Click me".into(),
                                data: b"my_action".to_vec(),
                            }
                        )
                    ]
                }
            )
        ]
    }
);

let msg = InputMessage::text("Pick an action:").reply_markup(keyboard);
```

## Silent message (no notification)

```rust
let msg = InputMessage::text("Heads-up (no ping)").silent(true);
```

## Scheduled message

```rust
use std::time::{SystemTime, UNIX_EPOCH};

// Schedule for 1 hour from now
let in_one_hour = SystemTime::now()
    .duration_since(UNIX_EPOCH).unwrap()
    .as_secs() as i32 + 3600;

let msg = InputMessage::text("This will appear in 1 hour")
    .schedule_date(Some(in_one_hour));
```

## No link preview

```rust
let msg = InputMessage::text("https://example.com: visit it!")
    .no_webpage(true);
```

## Combining everything

```rust
let (text, entities) = parse_markdown("📢 **Announcement:** check out _this week's update_!");

let msg = InputMessage::text(text)
    .entities(entities)
    .reply_to(Some(pinned_msg_id))
    .silent(false)
    .no_webpage(true)
    .reply_markup(keyboard);

client.send_message(channel_peer, msg).await?;
```
