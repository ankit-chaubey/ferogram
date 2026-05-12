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

//! Poll bot. Shows all poll types supported by PollBuilder.
//!
//! Commands:
//!   /poll   - regular poll
//!   /quiz   - quiz with a correct answer and explanation
//!   /multi  - multiple choice poll
//!   /timed  - poll that closes after 60 seconds
//!
//! Run:
//!   cargo run --example poll_bot
//!
//! When prompted, enter your bot token (get one from @BotFather).

use ferogram::poll::PollBuilder;
use ferogram::{Client, update::Update};

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
        eprintln!("Fill in API_ID and API_HASH at the top of poll_bot.rs");
        std::process::exit(1);
    }

    let (client, _shutdown) = Client::quick_connect("poll.session", API_ID, API_HASH).await?;

    let me = client.get_me().await?;
    println!(
        "Bot running as @{}",
        me.username.as_deref().unwrap_or("unknown")
    );

    let mut stream = client.stream_updates();
    while let Some(upd) = stream.next().await {
        if let Update::NewMessage(msg) = upd {
            if msg.outgoing() {
                continue;
            }

            let Some(peer) = msg.peer_id().cloned() else {
                continue;
            };

            let text = msg.text().unwrap_or("").trim().to_string();

            match text.as_str() {
                "/poll" => {
                    client
                        .send_poll(
                            peer,
                            PollBuilder::new("What is your favourite programming language?")
                                .answers(["Rust", "Go", "Python", "C++"]),
                        )
                        .await
                        .ok();
                }

                "/quiz" => {
                    client
                        .send_poll(
                            peer,
                            PollBuilder::new("What year was Rust 1.0 released?")
                                .answers(["2014", "2015", "2016", "2017"])
                                .quiz(true)
                                .correct_index(1)
                                .solution("Rust 1.0 was released on May 15, 2015."),
                        )
                        .await
                        .ok();
                }

                "/multi" => {
                    client
                        .send_poll(
                            peer,
                            PollBuilder::new("Which of these are systems languages?")
                                .answers(["Rust", "Go", "Python", "C", "JavaScript"])
                                .multiple_choice(true),
                        )
                        .await
                        .ok();
                }

                "/timed" => {
                    client
                        .send_poll(
                            peer,
                            PollBuilder::new("Quick: tabs or spaces?")
                                .answers(["Tabs", "Spaces"])
                                .close_period(60),
                        )
                        .await
                        .ok();
                }

                _ => {}
            }
        }
    }

    Ok(())
}
