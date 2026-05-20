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

//! Userbot that connects from a string session, no session file needed.
//!
//! Useful for serverless environments or anywhere you cannot persist files
//! to disk. The session string is read from SESSION_STRING at startup.
//!
//! Generate a session string first:
//!   cargo run --example string_session_gen
//!
//! Then run:
//!   SESSION_STRING="..." cargo run --example serverless_userbot

use ferogram::Client;

const API_ID: i32 = 0; // fill in from https://my.telegram.org
const API_HASH: &str = ""; // fill in from https://my.telegram.org

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
        .map_err(|_| "SESSION_STRING not set. Run string_session_gen first.")?;

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
        .send_message("me", "ferogram string session works.")
        .await?;

    Ok(())
}
