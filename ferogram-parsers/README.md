# ferogram-parsers

Telegram HTML and Markdown entity parsers for ferogram.

[![Crates.io](https://img.shields.io/crates/v/ferogram-parsers?color=fc8d62)](https://crates.io/crates/ferogram-parsers)
[![Telegram](https://img.shields.io/badge/community-%40FerogramChat-2CA5E0?logo=telegram)](https://t.me/FerogramChat) [![Channel](https://img.shields.io/badge/channel-%40Ferogram-2CA5E0?logo=telegram)](https://t.me/Ferogram)
[![docs.rs](https://img.shields.io/badge/docs.rs-ferogram--parsers-5865F2)](https://docs.rs/ferogram-parsers)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](#license)

Converts Telegram-flavoured Markdown and HTML to and from `MessageEntity` vectors. Extracted in v0.3.0; `ferogram` re-exports everything via `ferogram::parsers`.

Can be used standalone by any crate that works with Telegram formatted text.

---

## Installation

```toml
[dependencies]
ferogram-parsers = "0.3.0"
```

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

---

## Feature flags

| Flag | What it enables |
|---|---|
| `html5ever` | Replaces `parse_html` with a spec-compliant html5ever tokenizer |

By default `parse_html` uses the built-in hand-rolled parser (zero extra dependencies). Enable `html5ever` for strict HTML5 conformance.

```toml
ferogram-parsers = { version = "0.3.0", features = ["html5ever"] }
```

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
