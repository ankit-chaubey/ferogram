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

//! Inline query bot. Users type `@your_bot <text>` anywhere in Telegram and
//! get instant results they can tap to send.
//!
//! Run:
//!   cargo run --example inline_query_bot
//!
//! Prerequisites:
//!   1. Create a bot with @BotFather and copy the token into BOT_TOKEN.
//!   2. Enable inline mode in @BotFather: /setinline -> choose your bot.
//!   3. Fill in API_ID and API_HASH from https://my.telegram.org.
//!
//! Once running, open any chat, type "@your_bot hello" and tap a result.

use ferogram::tl;
use ferogram::{Client, update::Update};

const API_ID: i32 = 0; // from https://my.telegram.org
const API_HASH: &str = ""; // from https://my.telegram.org
const BOT_TOKEN: &str = ""; // from @BotFather, e.g. "123456:ABC-DEF..."

#[tokio::main]
async fn main() {
    if let Err(e) = run().await {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}

async fn run() -> Result<(), Box<dyn std::error::Error>> {
    if API_ID == 0 || API_HASH.is_empty() || BOT_TOKEN.is_empty() {
        eprintln!("Fill in API_ID, API_HASH and BOT_TOKEN at the top of inline_query_bot.rs");
        std::process::exit(1);
    }

    let (client, _shutdown) =
        Client::quick_connect("inline_query_bot.session", API_ID, API_HASH).await?;

    let me = client.get_me().await?;
    println!(
        "Running as @{}\nType @{} <text> in any chat.\n",
        me.username.as_deref().unwrap_or("?"),
        me.username.as_deref().unwrap_or("your_bot"),
    );

    let mut stream = client.stream_updates();
    while let Some(upd) = stream.next().await {
        if let Update::InlineQuery(iq) = upd {
            let q = iq.query().trim().to_string();
            let qid = iq.query_id;
            println!("Inline query [qid={qid}]: {q:?}");

            let results = build_results(&q);

            // answer_inline_query(query_id, results, cache_time_secs, is_personal, next_offset)
            let _ = client
                .answer_inline_query(qid, results, 0, false, None)
                .await;
        }
    }

    Ok(())
}

/// Build the list of inline result articles for a given query.
fn build_results(q: &str) -> Vec<tl::enums::InputBotInlineResult> {
    if q.is_empty() {
        // Show a help card when the user has not typed anything yet.
        return vec![article(
            "help",
            "Type something to transform it",
            "Usage: @bot <text>  -  get UPPER, reversed, word count, and more.",
        )];
    }

    let upper = q.to_uppercase();
    let lower = q.to_lowercase();
    let rev: String = q.chars().rev().collect();
    let words = q.split_whitespace().count();
    let chars = q.chars().count();
    let bytes = q.len();

    vec![
        article("upper", &format!("UPPER: {upper}"), &upper),
        article("lower", &format!("lower: {lower}"), &lower),
        article("rev", &format!("Reversed: {rev}"), &rev),
        article(
            "stats",
            &format!("{chars} chars, {words} words, {bytes} bytes"),
            &format!("{chars} characters | {words} words | {bytes} bytes"),
        ),
    ]
}

/// Build a single article result that sends plain text when tapped.
fn article(id: &str, title: &str, content: &str) -> tl::enums::InputBotInlineResult {
    tl::enums::InputBotInlineResult::InputBotInlineResult(tl::types::InputBotInlineResult {
        id: id.to_string(),
        r#type: "article".to_string(),
        title: Some(title.to_string()),
        description: Some(content.to_string()),
        url: None,
        thumb: None,
        content: None,
        send_message: tl::enums::InputBotInlineMessage::Text(
            tl::types::InputBotInlineMessageText {
                no_webpage: false,
                invert_media: false,
                message: content.to_string(),
                entities: None,
                reply_markup: None,
            },
        ),
    })
}
