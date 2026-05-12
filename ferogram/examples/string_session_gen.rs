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

//! Generates a string session you can use for serverless deployments.
//!
//! A string session encodes your auth into a compact string so you can store
//! it in an env var or secret manager instead of a session file on disk.
//!
//! Run:
//!   cargo run --example string_session_gen
//!
//! After login, your session string is printed. Copy it and store it as the
//! SESSION_STRING env var for use with the serverless_userbot example.
//!
//! Keep the string private. It gives full access to your account.

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
        eprintln!("Fill in API_ID and API_HASH at the top of string_session_gen.rs");
        std::process::exit(1);
    }

    let (client, _shutdown) = Client::quick_connect("gen.session", API_ID, API_HASH).await?;

    let me = client.get_me().await?;
    println!(
        "Logged in as {} ({})",
        me.first_name.as_deref().unwrap_or("?"),
        me.id
    );

    let session_string = client.export_session_string().await?;

    println!("\nYour session string:\n");
    println!("{session_string}");
    println!("\nStore this as SESSION_STRING and use it with the serverless_userbot example.");
    println!("Keep it private. It gives full access to your account.");

    Ok(())
}
