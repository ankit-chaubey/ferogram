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

//! Search for messages inside any chat by keyword.
//!
//! `messages.Search` runs server-side full-text search across the entire
//! history of a chat. The Bot API has no search endpoint at all - bots can
//! only see messages as they arrive. With MTProto you can search years of
//! history in one call.
//!
//! Run:
//!   cargo run --example search_messages
//!
//! Fill in API_ID, API_HASH, PHONE, PEER and QUERY below.

use ferogram::{Client, TransportKind};

const API_ID: i32 = 0; // from https://my.telegram.org
const API_HASH: &str = ""; // from https://my.telegram.org
const PHONE: &str = ""; // e.g. "+15551234567"

/// Chat to search in. "me" searches Saved Messages.
/// Any username, phone number, or integer peer ID works.
const PEER: &str = "me";

/// Keyword to search for.
const QUERY: &str = "hello";

/// Maximum results to return.
const LIMIT: i32 = 20;

#[tokio::main]
async fn main() {
    if let Err(e) = run().await {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}

async fn run() -> Result<(), Box<dyn std::error::Error>> {
    if API_ID == 0 || API_HASH.is_empty() || PHONE.is_empty() {
        eprintln!("Fill in API_ID, API_HASH and PHONE at the top of search_messages.rs");
        std::process::exit(1);
    }

    println!("Connecting...");
    let (client, _shutdown) = Client::builder()
        .api_id(API_ID)
        .api_hash(API_HASH)
        .transport(TransportKind::Abridged)
        .connect()
        .await?;

    if !client.is_authorized().await? {
        login(&client).await?;
        client.save_session().await?;
        println!("Session saved.");
    }

    let me = client.get_me().await?;
    let display = me
        .first_name
        .as_deref()
        .unwrap_or(me.username.as_deref().unwrap_or("?"));
    println!("Logged in as {display}\n");

    println!(
        "Searching {:?} in {PEER} (limit {LIMIT}):\n{}",
        QUERY,
        "-".repeat(60)
    );

    // search() returns a fluent builder; chain .limit() before .fetch().
    // The server does full-text indexing - this is instant even on huge chats.
    let results = client
        .search(PEER, QUERY)
        .limit(LIMIT)
        .fetch(&client)
        .await?;

    if results.is_empty() {
        println!("No messages found.");
        return Ok(());
    }

    for msg in &results {
        let ts = msg
            .date_utc()
            .map(|d| d.format("%Y-%m-%d %H:%M:%S").to_string())
            .unwrap_or_else(|| format!("unix={}", msg.date()));
        let body = msg.text().unwrap_or("").trim();
        let snippet = &body[..body.floor_char_boundary(100)];
        println!("[msg={:>8}] {ts}  {snippet}", msg.id());
    }

    println!("{}\nFound {} result(s).", "-".repeat(60), results.len());
    Ok(())
}

async fn login(client: &Client) -> Result<(), Box<dyn std::error::Error>> {
    let name = client.interactive_sign_in(PHONE).await?;
    println!("Signed in as {name}");
    Ok(())
}
