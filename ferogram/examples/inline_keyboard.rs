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
//! the Dispatcher's `on_callback_query` (see [`ferogram::filters`]).
//!
//! Run:
//!   cargo run --example inline_keyboard
//!
//! When prompted, enter your bot token (get one from @BotFather).
//! Send /start to the bot to see the keyboard.

use ferogram::filters::{Dispatcher, command, data};
use ferogram::tl;
use ferogram::{Client, InputMessage};

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

    let mut dp = Dispatcher::new();

    dp.on_message(command("start"), |msg| async move {
        let keyboard = kb(vec![
            vec![bc("👋 Hello", "cb:hello"), bc("ℹ️ About", "cb:about")],
            vec![bc("🦀 What is ferogram?", "cb:ferogram")],
            vec![bu("GitHub", "https://github.com/ankit-chaubey/ferogram")],
        ]);

        msg.reply(InputMessage::html("Welcome! Pick an option below:").reply_markup(keyboard))
            .await
            .ok();
    });

    // Each button data value gets its own handler; `answer()` shows a toast
    // (or a modal via `.alert(...)`) and is safe to call even if another
    // matching handler already answered this query.
    {
        let client = client.clone();
        dp.on_callback_query(data("cb:hello"), move |cb| {
            let client = client.clone();
            async move {
                cb.answer().text("Hello there!").send(&client).await.ok();
            }
        });
    }
    {
        let client = client.clone();
        dp.on_callback_query(data("cb:about"), move |cb| {
            let client = client.clone();
            async move {
                cb.answer()
                    .alert("Built with ferogram: pure Rust MTProto.")
                    .send(&client)
                    .await
                    .ok();
            }
        });
    }
    {
        let client = client.clone();
        dp.on_callback_query(data("cb:ferogram"), move |cb| {
            let client = client.clone();
            async move {
                cb.answer()
                    .alert("An async Rust MTProto client built from scratch.")
                    .send(&client)
                    .await
                    .ok();
            }
        });
    }
    // Fallback for any other/unrecognized callback data.
    {
        let client = client.clone();
        dp.on_callback_query(
            !(data("cb:hello") | data("cb:about") | data("cb:ferogram")),
            move |cb| {
                let client = client.clone();
                async move {
                    cb.answer().text("Unknown button.").send(&client).await.ok();
                }
            },
        );
    }

    let mut stream = client.stream_updates();
    while let Some(upd) = stream.next().await {
        dp.dispatch(upd).await;
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
