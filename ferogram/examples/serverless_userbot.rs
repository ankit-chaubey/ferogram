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

//! Userbot using a string session instead of a session file.
//!
//! Useful for serverless environments, tiny VPS setups, or anywhere you
//! can't or don't want to persist files to disk. The session string is
//! loaded from an env var at startup.
//!
//! On connect it sends a message to your Saved Messages to confirm the
//! session is working, then exits. Good sanity check before wiring up
//! more logic.
//!
//! Run:
//!   SESSION_STRING="..." cargo run --example serverless_userbot
//!
//! To generate a session string, run the string_session_gen example first:
//!   cargo run --example string_session_gen

use ferogram::Client;

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
        eprintln!("Fill in API_ID and API_HASH at the top of serverless_userbot.rs");
        std::process::exit(1);
    }

    let session_string = std::env::var("SESSION_STRING")
        .map_err(|_| "SESSION_STRING env var not set. Run string_session_gen first.")?;

    let (client, _shutdown) = Client::builder()
        .api_id(API_ID)
        .api_hash(API_HASH)
        .session_string(session_string)
        .connect()
        .await?;

    let me = client.get_me().await?;
    println!(
        "Logged in as {} ({})",
        me.first_name.as_deref().unwrap_or("?"),
        me.id
    );

    client
        .send_message("me", "Hello from ferogram! String session is working.")
        .await?;
    println!("Sent a message to Saved Messages. Check your Telegram app.");

    Ok(())
}
