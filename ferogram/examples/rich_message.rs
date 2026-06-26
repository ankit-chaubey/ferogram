// Copyright (c) Ankit Chaubey <ankitchaubey.dev@gmail.com>
//
// ferogram: async Telegram MTProto client in Rust
// https://github.com/ankit-chaubey/ferogram
//
// Licensed under either the MIT License or the Apache License 2.0.
// See the LICENSE-MIT or LICENSE-APACHE file in this repository:
// https://github.com/ankit-chaubey/ferogram
//
// Feel free to use, modify, and share this code.
// Please keep this notice when redistributing.

//! Rich message demo. Sends a structured article with headings, a table,
//! a code block, a collapsible section, and a divider to a chat.
//!
//! Rich messages render as full documents inside Telegram — not flat text.
//! They use `PageBlock` rather than `MessageEntity`.
//!
//! Run:
//!   cargo run --example rich_message
//!
//! Fill in API_ID, API_HASH, BOT_TOKEN and TARGET_USERNAME below.
//! Get API credentials from https://my.telegram.org
//! Get a bot token from @BotFather on Telegram.

use ferogram::{Client, InputMessage, parsers::parse_rich_markdown};

const API_ID: i32 = 0; // from https://my.telegram.org
const API_HASH: &str = ""; // from https://my.telegram.org
const BOT_TOKEN: &str = ""; // from @BotFather
const TARGET_USERNAME: &str = ""; // e.g. "username" (without @)

#[tokio::main]
async fn main() {
    if let Err(e) = run().await {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}

async fn run() -> Result<(), Box<dyn std::error::Error>> {
    if API_ID == 0 || API_HASH.is_empty() || BOT_TOKEN.is_empty() || TARGET_USERNAME.is_empty() {
        eprintln!(
            "Fill in API_ID, API_HASH, BOT_TOKEN and TARGET_USERNAME at the top of rich_message.rs"
        );
        std::process::exit(1);
    }

    let (client, _shutdown) = Client::builder()
        .api_id(API_ID)
        .api_hash(API_HASH)
        .connect()
        .await?;

    if !client.is_authorized().await? {
        client.bot_sign_in(BOT_TOKEN).await?;
        client.save_session().await?;
    }

    let article = r##"
# ferogram Rich Message Demo

This message was sent using `parse_rich_markdown` from **ferogram-parsers**.
It renders as a structured document, not flat text.

---

## Inline formatting

You can use **bold**, _italic_, ~~strikethrough~~, ||spoiler||, and `inline code`.

Math works inline too: $E = mc^2$

---

## Code block

```rust
async fn run() -> Result<(), Box<dyn std::error::Error>> {
    let blocks = parse_rich_markdown("# Hello");
    let msg = InputMessage::text("").rich_text(blocks);
    client.send_message(peer, msg).await?;
    Ok(())
}
```

---

## Table

| Feature        | Support |
|----------------|---------|
| Headings       | yes     |
| Tables         | yes     |
| Math           | yes     |
| Collapsible    | yes     |
| Maps           | yes     |

---

## Collapsible section

<details>
<summary>Click to expand</summary>
This content is hidden until the user taps the summary.
You can put anything here: text, lists, code.
</details>

---

## Ordered list

1. Parse markdown with `parse_rich_markdown`
2. Pass the `Vec<PageBlock>` to `.rich_text()`
3. Send

---

## Math block

$$
\int_0^\infty e^{-x^2} dx = \frac{\sqrt{\pi}}{2}
$$

---

_Sent by ferogram. See docs/src/messaging/rich-messages.md for the full syntax reference._
"##;

    let blocks = parse_rich_markdown(article.trim());
    let msg = InputMessage::text("").rich_text(blocks);

    client.send_message(TARGET_USERNAME, msg).await?;

    println!("Rich message sent to @{TARGET_USERNAME}");

    Ok(())
}
