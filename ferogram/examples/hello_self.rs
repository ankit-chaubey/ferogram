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

//! Sends "Hello from ferogram!" to your Saved Messages (the "me" chat).
//!
//! Run:
//!   cargo run --example hello_self
//!
//! First run will prompt for your phone number or bot token,
//! then the login code, then 2FA password if you have one set.
//! After that the session is saved so subsequent runs skip all of that.

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
        eprintln!("Fill in API_ID and API_HASH at the top of hello_self.rs");
        std::process::exit(1);
    }

    let (client, _shutdown) = Client::quick_connect("hello.session", API_ID, API_HASH).await?;

    client.send_message("me", "Hello from ferogram!").await?;
    println!("Message sent to Saved Messages.");

    client.save_session().await?;
    Ok(())
}
