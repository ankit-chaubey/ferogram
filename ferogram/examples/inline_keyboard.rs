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

//! Inline keyboard bot. Sends a menu on /start and handles button taps via
//! callback queries.
//!
//! Run:
//!   cargo run --example inline_keyboard
//!
//! When prompted, enter your bot token (get one from @BotFather).
//! Send /start to the bot to see the keyboard.

use ferogram::tl;
use ferogram::{Client, InputMessage, update::Update};

const API_ID: i32 = 0; // fill in your api_id from https://my.telegram.org
const API_HASH: &str = ""; // fill in your api_hash from https://my.telegram.org

#[tokio::main]
async fn main() {
    if let Err(e) = run().await {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}

async fn run() -> Result<(), Box<dyn std::error::Error>> {
    if API_ID == 0 || API_HASH.is_empty() {
        eprintln!("Fill in API_ID and API_HASH at the top of inline_keyboard.rs");
        std::process::exit(1);
    }

    let (client, _shutdown) = Client::quick_connect("keyboard.session", API_ID, API_HASH).await?;

    let me = client.get_me().await?;
    println!(
        "Bot running as @{}",
        me.username.as_deref().unwrap_or("unknown")
    );

    let mut stream = client.stream_updates();
    while let Some(upd) = stream.next().await {
        match upd {
            Update::NewMessage(msg) => {
                if msg.outgoing() {
                    continue;
                }

                let text = msg.text().unwrap_or("").trim().to_string();
                if text == "/start" {
                    let keyboard = kb(vec![
                        vec![bc("👋 Hello", "cb:hello"), bc("ℹ️ About", "cb:about")],
                        vec![bc("🦀 What is ferogram?", "cb:ferogram")],
                        vec![bu("GitHub", "https://github.com/ankit-chaubey/ferogram")],
                    ]);

                    msg.reply(
                        InputMessage::html("Welcome! Pick an option below:").reply_markup(keyboard),
                    )
                    .await
                    .ok();
                }
            }

            Update::CallbackQuery(cb) => {
                let qid = cb.query_id;
                let data = cb.data().unwrap_or("").to_string();

                let _ = match data.as_str() {
                    "cb:hello" => {
                        client
                            .answer_callback_query(qid, Some("Hello there!"), false)
                            .await
                    }
                    "cb:about" => {
                        client
                            .answer_callback_query(
                                qid,
                                Some("Built with ferogram: pure Rust MTProto."),
                                true,
                            )
                            .await
                    }
                    "cb:ferogram" => {
                        client
                            .answer_callback_query(
                                qid,
                                Some("An async Rust MTProto client built from scratch."),
                                true,
                            )
                            .await
                    }
                    _ => {
                        client
                            .answer_callback_query(qid, Some("Unknown button."), false)
                            .await
                    }
                };
            }

            _ => {}
        }
    }

    Ok(())
}

fn kb(rows: Vec<Vec<tl::enums::KeyboardButton>>) -> tl::enums::ReplyMarkup {
    tl::enums::ReplyMarkup::ReplyInlineMarkup(tl::types::ReplyInlineMarkup {
        rows: rows
            .into_iter()
            .map(|row| {
                tl::enums::KeyboardButtonRow::KeyboardButtonRow(tl::types::KeyboardButtonRow {
                    buttons: row,
                })
            })
            .collect(),
    })
}

fn bc(text: &str, data: &str) -> tl::enums::KeyboardButton {
    tl::enums::KeyboardButton::Callback(tl::types::KeyboardButtonCallback {
        requires_password: false,
        style: None,
        text: text.to_string(),
        data: data.as_bytes().to_vec(),
    })
}

fn bu(text: &str, url: &str) -> tl::enums::KeyboardButton {
    tl::enums::KeyboardButton::Url(tl::types::KeyboardButtonUrl {
        style: None,
        text: text.to_string(),
        url: url.to_string(),
    })
}
