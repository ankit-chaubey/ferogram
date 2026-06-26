# ferogram-parsers

Telegram HTML and Markdown entity parsers for ferogram.

[![Crates.io](https://img.shields.io/crates/v/ferogram-parsers?color=fc8d62)](https://crates.io/crates/ferogram-parsers)
[![Telegram](https://img.shields.io/badge/community-%40FerogramChat-2CA5E0?logo=telegram)](https://t.me/FerogramChat) [![Channel](https://img.shields.io/badge/channel-%40Ferogram-2CA5E0?logo=telegram)](https://t.me/Ferogram)
[![docs.rs](https://img.shields.io/badge/docs.rs-ferogram--parsers-5865F2)](https://docs.rs/ferogram-parsers)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](#license)

Converts Telegram-flavoured Markdown and HTML to and from `MessageEntity` vectors. `ferogram` re-exports everything via `ferogram::parsers`, so you don't need to add this separately unless you're working with formatted text outside the full client.

For installation instructions see the [ferogram README](https://github.com/ankit-chaubey/ferogram).

---

## Usage

### Markdown

```rust
use ferogram_parsers::{parse_markdown, generate_markdown};

let (text, entities) = parse_markdown("**bold** and _italic_");
// text     = "bold and italic"
// entities = [Bold(0..4), Italic(9..15)]

let md = generate_markdown(&text, &entities);
// md = "**bold** and __italic__"
```

Supported syntax:

| Syntax | Entity |
|---|---|
| `**bold**` or `*bold*` | Bold |
| `__italic__` or `_italic_` | Italic |
| `~~strike~~` | Strikethrough |
| `\|\|spoiler\|\|` | Spoiler |
| `` `code` `` | Code |
| ` ```lang\npre\n``` ` | Pre (code block) |
| `[text](url)` | TextUrl |
| `[text](tg://user?id=123)` | MentionName |
| `![text](tg://emoji?id=123)` | CustomEmoji |
| `\*`, `\_`, `\~` ... | Escaped literal char |

### HTML

```rust
use ferogram_parsers::{parse_html, generate_html};

let (text, entities) = parse_html("<b>Hello</b> <i>world</i>");
// text     = "Hello world"
// entities = [Bold(0..5), Italic(6..11)]

let html = generate_html(&text, &entities);
// html = "<b>Hello</b> <i>world</i>"
```

Supported tags: `<b>`, `<strong>`, `<i>`, `<em>`, `<u>`, `<s>`, `<del>`, `<code>`, `<pre>`, `<tg-spoiler>`, `<a href="url">`, `<tg-emoji emoji-id="id">`

### Rich Messages

Rich messages use `PageBlock` / `RichText` trees instead of flat entity lists. Both Markdown and HTML rich formats are supported.

```rust
use ferogram_parsers::{parse_rich_markdown, parse_rich_html, parse_rich_html_inline};

// Rich Markdown → Vec<PageBlock>
let blocks = parse_rich_markdown("# Heading\n\n**bold** text with $math$");

// Rich HTML → Vec<PageBlock>
let blocks = parse_rich_html("<h1>Heading</h1><p><b>bold</b> text</p>");

// Inline HTML → RichText (for table cells, captions, etc.)
let rt = parse_rich_html_inline("<b>bold</b> and <tg-spoiler>secret</tg-spoiler>");
```

Pass the resulting `Vec<PageBlock>` to `inputRichMessage` (or `inputRichMessageHTML` / `inputRichMessageMarkdown`) when calling `messages.sendMessage` with a rich message.

**Supported block types:** Headings H1-H6, Paragraph, Preformatted, Divider, Unordered/Ordered/Task lists, Blockquote, Pullquote/Aside, Table, Details/Summary, Media (Photo/Video/Audio/Voice/Animation), Collage, Slideshow, Map, Math block, Footnotes, Footer, Anchor.

**Supported inline (RichText) types:** Bold, Italic, Underline, Strike, Fixed, Marked, Spoiler, Subscript, Superscript, Inline Math, Url, Email, Phone, MentionName, CustomEmoji, Date/Time, Anchor, Concat.

---

## Feature flags

| Flag | What it enables |
|---|---|
| `html5ever` | Replaces `parse_html` with a spec-compliant html5ever tokenizer |

By default `parse_html` uses the built-in hand-rolled parser (zero extra dependencies). Enable `html5ever` for strict HTML5 conformance.

---

## Stack position

```
ferogram
└ ferogram-parsers  <-- here
  └ ferogram-tl-types (tl-api feature)
```

---

## License

MIT or Apache-2.0, at your option. See [LICENSE-MIT](../LICENSE-MIT) and [LICENSE-APACHE](../LICENSE-APACHE).

**Ankit Chaubey** - [github.com/ankit-chaubey](https://github.com/ankit-chaubey)
