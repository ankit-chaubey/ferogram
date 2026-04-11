# Message Formatting

Telegram supports rich text formatting through **message entities**: positional markers that indicate bold, italic, code, links, and more.

## Using parse_markdown

The easiest way is `parse_markdown`, which converts a Markdown-like syntax into a `(String, Vec<MessageEntity>)` tuple:

```rust
use ferogram::parsers::parse_markdown;
use ferogram::InputMessage;

let (plain, entities) = parse_markdown(
    "**Bold text**, _italic text_, `inline code`\n\
     and a [clickable link](https://example.com)"
);

let msg = InputMessage::text(plain).entities(entities);
client.send_message_to_peer_ex(peer, &msg).await?;
```

## Supported syntax

| Markdown | Entity type | Example |
|---|---|---|
| `**text**` | Bold | **Hello** |
| `_text_` | Italic | _Hello_ |
| `*text*` | Italic | *Hello* |
| `__text__` | Underline | Hello |
| `~~text~~` | Strikethrough | ~~Hello~~ |
| `` `text` `` | Code (inline) | `Hello` |
| ```` ```text``` ```` | Pre (code block) | block |
| `\|\|text\|\|` | Spoiler | ▓▓▓▓▓ |
| `[label](url)` | Text link | clickable |

## Building entities manually

For full control, construct `MessageEntity` values directly:

```rust
use ferogram_tl_types as tl;

let text = "Hello world";
let entities = vec![
    // Bold "Hello"
    tl::enums::MessageEntity::Bold(tl::types::MessageEntityBold {
        offset: 0,
        length: 5,
    }),
    // Code "world"
    tl::enums::MessageEntity::Code(tl::types::MessageEntityCode {
        offset: 6,
        length: 5,
    }),
];

let msg = InputMessage::text(text).entities(entities);
```

## All entity types (Layer 224)

| Enum variant | Description |
|---|---|
| `Bold` | **Bold text** |
| `Italic` | _Italic text_ |
| `Underline` | Underlined |
| `Strike` | ~~Strikethrough~~ |
| `Spoiler` | Hidden until tapped |
| `Code` | `Monospace inline` |
| `Pre` | Code block (optional language) |
| `TextUrl` | Hyperlink with custom label |
| `Url` | Auto-detected URL |
| `Email` | Auto-detected email |
| `Phone` | Auto-detected phone number |
| `Mention` | @username mention |
| `MentionName` | Inline mention by user ID |
| `Hashtag` | #hashtag |
| `Cashtag` | $TICKER |
| `BotCommand` | /command |
| `BankCard` | Bank card number |
| `BlockquoteCollapsible` | Collapsible quote block |
| `CustomEmoji` | Custom emoji by document ID |
| `FormattedDate` | ✨ New in Layer 223: displays a date in local time |

## Pre block with language

```rust
tl::enums::MessageEntity::Pre(tl::types::MessageEntityPre {
    offset:   0,
    length:   code_text.len() as i32,
    language: "rust".into(),
})
```

## Mention by user ID (no @username needed)

```rust
tl::enums::MessageEntity::MentionName(tl::types::MessageEntityMentionName {
    offset:  0,
    length:  5,   // length of the label text
    user_id: 123456789,
})
```

## FormattedDate: Layer 224

A new entity that automatically formats a unix timestamp into the user's local timezone and locale:

```rust
tl::enums::MessageEntity::FormattedDate(tl::types::MessageEntityFormattedDate {
    flags:    0,
    relative:    false, // "yesterday", "2 days ago"
    short_time:  false, // "14:30"
    long_time:   false, // "2:30 PM"
    short_date:  true,  // "Jan 5"
    long_date:   false, // "January 5, 2026"
    day_of_week: false, // "Monday"
    offset:      0,
    length:      text.len() as i32,
    date:        1736000000, // unix timestamp
})
```
