# Rich Messages

Rich messages render as structured articles inside Telegram with real headings, tables, collapsible sections, embedded media, and more. They use `InputRichMessage` and `PageBlock` rather than the flat `MessageEntity` list used by regular messages.

## When to use

Regular formatting (`InputMessage::markdown` / `InputMessage::html`) produces inline entities on a flat text string. Rich messages produce a full document structure:

| Regular message | Rich message |
|---|---|
| `MessageEntity` offsets on plain text | `PageBlock` tree |
| Bold, italic, code, links | All of the above plus headings, tables, lists, math, maps, media blocks |
| `send_message` | `send_message` with `InputRichMessage` |

## Quickstart

```rust
use ferogram::parsers::parse_rich_markdown;

let blocks = parse_rich_markdown("# Hello\n\nThis is **bold** and _italic_.");

client
    .send_message(peer)
    .rich_text(blocks)
    .await?;
```

Or from HTML:

```rust
use ferogram::parsers::parse_rich_html;

let blocks = parse_rich_html("<h1>Hello</h1><p>This is <b>bold</b>.</p>");

client
    .send_message(peer)
    .rich_text(blocks)
    .await?;
```

## Rich Markdown syntax

### Headings

```
# Heading 1
## Heading 2
### Heading 3
#### Heading 4
##### Heading 5
###### Heading 6
```

### Inline formatting

| Syntax | Result |
|---|---|
| `**text**` | Bold |
| `_text_` or `*text*` | Italic |
| `__text__` | Bold (two underscores) |
| `~~text~~` | Strikethrough |
| `\|\|text\|\|` | Spoiler |
| `` `text` `` | Inline code |
| `$source$` | Inline math |
| `[label](url)` | Link |
| `[label](tg://user?id=123)` | Mention by user ID |
| `![alt](tg://emoji?id=N)` | Custom emoji |

### Code block

````
```rust
fn main() {
    println!("hello");
}
```
````

### Math block

```
$$
E = mc^2
$$
```

Or inline with single `$`: `$E = mc^2$`

### Blockquote

```
> This is a blockquote.
```

### Lists

Unordered:

```
- item one
- item two
- [ ] unchecked task
- [x] checked task
```

Ordered:

```
1. first
2. second
3. third
```

### Table

```
| Name  | Age |
|-------|-----|
| Alice | 30  |
| Bob   | 25  |
```

Column alignment:

```
| Left | Center | Right |
|:-----|:------:|------:|
| a    |   b    |     c |
```

### Divider

```
---
```

### Details / collapsible section

```html
<details>
<summary>Click to expand</summary>
Content here.
</details>
```

### Media

```
![caption](https://example.com/photo.jpg)
```

---

## Rich HTML syntax

### Headings

```html
<h1>Heading 1</h1>
<h2>Heading 2</h2>
<!-- h1 through h6 -->
```

### Inline formatting

| Tag | Result |
|---|---|
| `<b>`, `<strong>` | Bold |
| `<i>`, `<em>` | Italic |
| `<s>`, `<del>` | Strikethrough |
| `<u>`, `<ins>` | Underline |
| `<tg-spoiler>` | Spoiler |
| `<code>` | Inline code |
| `<mark>` | Marked/highlighted |
| `<sub>` | Subscript |
| `<sup>` | Superscript |
| `<a href="url">` | Link |
| `<a href="tg://user?id=123">` | Mention by user ID |
| `<a href="mailto:x@y.com">` | Email |
| `<a href="tel:+1234">` | Phone |

### Code block

```html
<pre><code class="language-rust">fn main() {}</code></pre>
```

### Math

```html
<tg-math-block>E = mc^2</tg-math-block>
```

Or inline: `<tg-math>E = mc^2</tg-math>`

### Lists

```html
<ul>
  <li>item one</li>
  <li>item two</li>
</ul>

<ol>
  <li>first</li>
  <li>second</li>
</ol>
```

### Table

```html
<table>
  <tr><th>Name</th><th>Age</th></tr>
  <tr><td>Alice</td><td>30</td></tr>
</table>
```

### Divider

```html
<hr/>
```

### Collapsible section

```html
<details open>
  <summary>Title</summary>
  Content here.
</details>
```

### Blockquote

```html
<blockquote>
  Quote text.
  <cite>Credit</cite>
</blockquote>
```

### Pullquote

```html
<aside>
  Pull quote text.
  <cite>Credit</cite>
</aside>
```

### Embedded map

```html
<tg-map lat="51.5074" long="-0.1278" zoom="12"/>
```

### Media collage / slideshow

```html
<tg-collage>
  <img src="https://example.com/a.jpg"/>
  <img src="https://example.com/b.jpg"/>
</tg-collage>

<tg-slideshow>
  <img src="https://example.com/a.jpg"/>
  <video src="https://example.com/b.mp4"/>
</tg-slideshow>
```

### Footer

```html
<footer>Footer text here.</footer>
```

### Anchor

```html
<a name="chapter-1"></a>
```

---

## Building PageBlocks manually

Both parsers return `Vec<tl::enums::PageBlock>`. You can also construct blocks directly:

```rust
use ferogram_tl_types as tl;

let blocks = vec![
    tl::enums::PageBlock::Heading1(tl::types::PageBlockHeading1 {
        text: tl::enums::RichText::TextPlain(tl::types::TextPlain {
            text: "My Title".into(),
        }),
    }),
    tl::enums::PageBlock::Paragraph(tl::types::PageBlockParagraph {
        text: tl::enums::RichText::TextPlain(tl::types::TextPlain {
            text: "Some content.".into(),
        }),
    }),
    tl::enums::PageBlock::Divider,
];
```

---

## PageBlock reference

| Variant | Description |
|---|---|
| `Heading1`–`Heading6` | Section headings |
| `Paragraph` | Body text |
| `Preformatted` | Code block with optional language |
| `Math` | Math expression (TeX/KaTeX source) |
| `List` | Unordered list |
| `OrderedList` | Ordered list |
| `Blockquote` | Block quote with optional caption |
| `Pullquote` | Pull quote (aside) |
| `Table` | Table with optional header row |
| `Details` | Collapsible section |
| `Divider` | Horizontal rule |
| `Map` | Embedded map with lat/long/zoom |
| `Photo` / `Video` / `Audio` | Media blocks |
| `Collage` | Side-by-side media grid |
| `Slideshow` | Swipeable media slideshow |
| `Anchor` | In-document anchor point |
| `Footer` | Footer text |
| `Embed` | Embedded external content |
