# Message Formatting

Telegram supports rich text formatting through **message entities**: positional markers that indicate bold, italic, code, links, and more.

## Quickest way: InputMessage constructors

The simplest approach is `InputMessage::markdown` or `InputMessage::html`, which parse the text and attach entities in one call:

```rust
use ferogram::InputMessage;

// Markdown
let msg = InputMessage::markdown("**Bold**, _italic_, `code`");

// HTML
let msg = InputMessage::html("<b>Bold</b>, <i>italic</i>, <code>code</code>");

client.send_message(peer, msg).await?;
```

If you just want to send without building an `InputMessage` first, the convenience methods do the same thing in one step:

```rust
client.send_message(peer, InputMessage::html("<b>Bold</b> and <i>italic</i>")).await?;
client.send_message(peer, InputMessage::markdown("**Bold** and _italic_")).await?;
```

## Using parse_markdown directly

If you need the `(String, Vec<MessageEntity>)` tuple for further processing:

```rust
use ferogram::parsers::parse_markdown;
use ferogram::InputMessage;

let (plain, entities) = parse_markdown("**Bold text**, _italic_, `inline code`");

let msg = InputMessage::text(plain).entities(entities);
client.send_message(peer, msg).await?;
```

## Markdown syntax

| Syntax | Entity |
|---|---|
| `**text**` | Bold |
| `*text*` | Bold |
| `_text_` | Italic |
| `__text__` | Italic |
| `~~text~~` | Strikethrough |
| `` `text` `` | Code (inline) |
| ```` ```lang\ncode\n``` ```` | Pre (code block) |
| `\|\|text\|\|` | Spoiler |
| `[label](url)` | TextUrl |
| `[label](tg://user?id=123)` | MentionName |
| `![text](tg://emoji?id=123)` | CustomEmoji |
| `\*`, `\_`, `\~` … | Escaped literal character |

> **Note:** Underline has no markdown syntax. Use HTML `<u>` if you need it.

## HTML syntax

Supported tags:

| Tag | Entity |
|---|---|
| `<b>`, `<strong>` | Bold |
| `<i>`, `<em>` | Italic |
| `<u>` | Underline |
| `<s>`, `<del>`, `<strike>` | Strikethrough |
| `<code>` | Code (inline) |
| `<pre>` | Pre (code block) |
| `<tg-spoiler>` | Spoiler |
| `<a href="url">` | TextUrl |
| `<a href="tg://user?id=123">` | MentionName |
| `<tg-emoji emoji-id="123">` | CustomEmoji |
| `<br>` | Newline |

## Building entities manually

For full control, construct `MessageEntity` values directly:

```rust
use ferogram_tl_types as tl;

let text = "Hello world";
let entities = vec![
    tl::enums::MessageEntity::Bold(tl::types::MessageEntityBold {
        offset: 0,
        length: 5,
    }),
    tl::enums::MessageEntity::Code(tl::types::MessageEntityCode {
        offset: 6,
        length: 5,
    }),
];

let msg = InputMessage::text(text).entities(entities);
```

## Pre block with language

```rust
tl::enums::MessageEntity::Pre(tl::types::MessageEntityPre {
    offset:   0,
    length:   code_text.encode_utf16().count() as i32,
    language: "rust".into(),
})
```

## Mention by user ID

```rust
tl::enums::MessageEntity::MentionName(tl::types::MessageEntityMentionName {
    offset:  0,
    length:  label.encode_utf16().count() as i32,
    user_id: 123456789,
})
```

## Blockquote

`messageEntityBlockquote` has an optional `collapsed` flag:

```rust
tl::enums::MessageEntity::Blockquote(tl::types::MessageEntityBlockquote {
    collapsed: false, // true = collapsible quote
    offset: 0,
    length: text.encode_utf16().count() as i32,
})
```

## FormattedDate

Displays a Unix timestamp in the user's local timezone and locale. Set the boolean flags you want; the rest default to false:

```rust
tl::enums::MessageEntity::FormattedDate(tl::types::MessageEntityFormattedDate {
    relative:    false,
    short_time:  false,
    long_time:   false,
    short_date:  true,  // e.g. "Jan 5"
    long_date:   false,
    day_of_week: false,
    offset: 0,
    length: placeholder_text.encode_utf16().count() as i32,
    date:   1736000000, // Unix timestamp
})
```

## Generating markup from entities

Both `generate_markdown` and `generate_html` are available if you need to serialise entities back to text:

```rust
use ferogram::parsers::{generate_markdown, generate_html};

let md  = generate_markdown(plain_text, &entities);
let htm = generate_html(plain_text, &entities);
```

> `generate_markdown` skips `Underline` (no unambiguous delimiter). Use `generate_html` if you need it.

## All entity types

| Variant | Description |
|---|---|
| `Bold` | Bold text |
| `Italic` | Italic text |
| `Underline` | Underlined (HTML only) |
| `Strike` | Strikethrough |
| `Spoiler` | Hidden until tapped |
| `Code` | Monospace inline |
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
| `Blockquote` | Block quote, optionally collapsible |
| `CustomEmoji` | Custom emoji by document ID |
| `FormattedDate` | Unix timestamp rendered in local time |
