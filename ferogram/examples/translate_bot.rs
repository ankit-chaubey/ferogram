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

//! Translation bot using Telegram's built-in translation API.
//!
//! Reply to any message with `/tr <lang>` and the bot translates it.
//! The translation is done server-side by Telegram - no external API key needed.
//!
//! Examples:
//!   /tr en   - translate the replied message to English
//!   /tr es   - translate to Spanish
//!   /tr ja   - translate to Japanese
//!
//! Run:
//!   cargo run --example translate_bot
//!
//! Fill in API_ID, API_HASH and BOT_TOKEN below (bot must be in the chat).

use ferogram::{Client, InputMessage, update::Update};

const API_ID: i32 = 0; // from https://my.telegram.org
const API_HASH: &str = ""; // from https://my.telegram.org
const BOT_TOKEN: &str = ""; // from @BotFather

#[tokio::main]
async fn main() {
    if let Err(e) = run().await {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}

async fn run() -> Result<(), Box<dyn std::error::Error>> {
    if API_ID == 0 || API_HASH.is_empty() || BOT_TOKEN.is_empty() {
        eprintln!("Fill in API_ID, API_HASH and BOT_TOKEN at the top of translate_bot.rs");
        std::process::exit(1);
    }

    let (client, _shutdown) =
        Client::quick_connect("translate_bot.session", API_ID, API_HASH).await?;

    let me = client.get_me().await?;
    println!(
        "Running as @{}\nReply to any message with /tr <lang>  (e.g. /tr en)\n",
        me.username.as_deref().unwrap_or("?")
    );

    let mut stream = client.stream_updates();
    while let Some(upd) = stream.next().await {
        if let Update::NewMessage(msg) = upd {
            if msg.outgoing() {
                continue;
            }

            let text = msg.text().unwrap_or("").trim().to_string();
            if !text.starts_with("/tr") {
                continue;
            }

            // Parse: /tr <lang>
            let lang = text
                .split_whitespace()
                .nth(1)
                .unwrap_or("")
                .to_ascii_lowercase();

            if lang.is_empty() || lang.len() > 10 {
                let _ = msg
                    .reply(InputMessage::text(
                        "Usage: reply to a message with /tr <lang>\nExample: /tr en",
                    ))
                    .await;
                continue;
            }

            // We need the peer and the ID of the message we're replying to.
            let Some(reply_id) = msg.reply_to_message_id() else {
                let _ = msg
                    .reply(InputMessage::text(
                        "Reply to a message you want translated.",
                    ))
                    .await;
                continue;
            };

            let Some(peer) = msg.peer_id() else { continue };

            println!(
                "Translating msg={reply_id} to lang={lang:?} in chat {:?}",
                peer
            );

            // translate_messages calls messages.translateText over MTProto.
            // No external service, no API key - Telegram Premium's translation
            // engine runs server-side.
            match client
                .translate_messages(peer.clone(), vec![reply_id], &lang)
                .await
            {
                Ok(results) => {
                    let translated = results
                        .into_iter()
                        .map(|t| t.text)
                        .collect::<Vec<_>>()
                        .join("\n");

                    if translated.is_empty() {
                        let _ = msg
                            .reply(InputMessage::text(
                                "Translation returned empty. Try a different language code.",
                            ))
                            .await;
                    } else {
                        let reply_text = format!("({lang})\n{translated}");
                        let _ = msg.reply(InputMessage::text(&reply_text)).await;
                    }
                }
                Err(e) => {
                    let err_text = format!("Translation failed: {e}");
                    eprintln!("{err_text}");
                    let _ = msg.reply(InputMessage::text(&err_text)).await;
                }
            }
        }
    }

    Ok(())
}
