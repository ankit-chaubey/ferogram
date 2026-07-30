/*
 * Copyright (c) 2026 Ankit Chaubey <ankitchaubey.dev@gmail.com>
 * https://github.com/ankit-chaubey
 *
 * Project: ferogram
 * Website: https://ferogram.dev
 *
 * Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
 * https://www.apache.org/licenses/LICENSE-2.0> or the MIT license
 * <LICENSE-MIT or https://opensource.org/licenses/MIT>, at your option.
 * This file may not be copied, modified, or distributed except according
 * to those terms.
 */

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
