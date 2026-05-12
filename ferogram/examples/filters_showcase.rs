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

//! Demonstrates ferogram's Dispatcher and filter system.
//!
//! Filters compose with & (and), | (or), and ! (not). The Dispatcher routes
//! each incoming message to the first matching handler.
//!
//! Run:
//!   cargo run --example filters_showcase
//!
//! When prompted, enter your bot token (get one from @BotFather).

use ferogram::Client;
use ferogram::filters::{
    Dispatcher, album, channel, command, forwarded, group, media, photo, private, reply,
    text_contains,
};

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
        eprintln!("Fill in API_ID and API_HASH at the top of filters_showcase.rs");
        std::process::exit(1);
    }

    let (client, _shutdown) = Client::quick_connect("filters.session", API_ID, API_HASH).await?;

    let me = client.get_me().await?;
    println!(
        "Bot running as @{}",
        me.username.as_deref().unwrap_or("unknown")
    );

    let mut dp = Dispatcher::new();

    // Single command filter
    dp.on_message(command("start"), |msg| async move {
        msg.reply("Hello! I'm a filter showcase bot.").await.ok();
    });

    dp.on_message(command("help"), |msg| async move {
        msg.reply(
            "Commands: /start /help\n\
             Also try: sending a photo, forwarded message, or album.",
        )
        .await
        .ok();
    });

    // Private chat only
    dp.on_message(private() & text_contains("hello"), |msg| async move {
        msg.reply("Hi! (private chat)").await.ok();
    });

    // Group chat command
    dp.on_message(group() & command("hi"), |msg| async move {
        msg.reply("Hey group!").await.ok();
    });

    // Any photo
    dp.on_message(photo(), |msg| async move {
        msg.reply("Nice photo!").await.ok();
    });

    // Any media that is not a photo (document, audio, video, etc.)
    dp.on_message(media() & !photo(), |msg| async move {
        msg.reply("Got a file!").await.ok();
    });

    // Album (multiple photos/files sent together)
    dp.on_message(album(), |msg| async move {
        msg.reply("Got an album!").await.ok();
    });

    // Forwarded messages
    dp.on_message(forwarded(), |msg| async move {
        msg.reply("That looks forwarded.").await.ok();
    });

    // Replies to another message
    dp.on_message(reply(), |msg| async move {
        msg.reply("That's a reply.").await.ok();
    });

    // Channel posts (for bots added to channels as admins)
    dp.on_message(channel(), |msg| async move {
        println!("Channel post received.");
        let _ = msg;
    });

    // Compose: private OR group (excludes channels)
    // dp.on_message(private() | group(), |msg| async move { ... });

    // Compose: group AND forwarded
    dp.on_message(group() & forwarded(), |msg| async move {
        msg.reply("Forwarded in a group.").await.ok();
    });

    let mut stream = client.stream_updates();
    while let Some(upd) = stream.next().await {
        dp.dispatch(upd).await;
    }

    Ok(())
}
