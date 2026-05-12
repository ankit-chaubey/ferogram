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

//! Simple echo bot. Every text message sent to the bot gets echoed back.
//!
//! This runs as a bot (not your personal account), so it only receives
//! messages that people send directly to it. Safe to leave running.
//!
//! Run:
//!   cargo run --example echo_bot
//!
//! Fill in API_ID, API_HASH and BOT_TOKEN below.
//! Get API credentials from https://my.telegram.org
//! Get a bot token from @BotFather on Telegram.

use ferogram::{Client, update::Update};

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
        eprintln!("Fill in API_ID, API_HASH and BOT_TOKEN at the top of echo_bot.rs");
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

    let me = client.get_me().await?;
    println!(
        "Running as @{}\nListening for messages...",
        me.username.as_deref().unwrap_or("?")
    );

    let mut stream = client.stream_updates();
    while let Some(upd) = stream.next().await {
        if let Update::NewMessage(msg) = upd {
            if msg.outgoing() {
                continue;
            }
            if let Some(text) = msg.text() {
                msg.reply(text).await.ok();
            }
        }
    }

    Ok(())
}
