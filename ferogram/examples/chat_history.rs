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

//! Read message history from Saved Messages (or any peer).
//!
//! This is the definitive MTProto showcase: one call to
//! `get_message_history` returns real messages from any chat, including
//! Saved Messages - something the Bot API cannot do at all.
//!
//! Run:
//!   cargo run --example chat_history
//!
//! Fill in API_ID, API_HASH and PHONE below, then run. On the first run
//! you will be prompted for your login code (and 2FA password if set).
//! Subsequent runs reuse the saved session.

use ferogram::{Client, TransportKind};

const API_ID: i32 = 0; // from https://my.telegram.org
const API_HASH: &str = ""; // from https://my.telegram.org
const PHONE: &str = ""; // your Telegram phone number, e.g. "+15551234567"

/// How many messages to fetch. Increase freely - the API supports up to 100
/// per call; use `get_history_range` for paginated walks over larger histories.
const LIMIT: i32 = 20;

/// Peer to read from. "me" resolves to Saved Messages. Any username, phone
/// number, or integer peer ID works here.
const PEER: &str = "me";

#[tokio::main]
async fn main() {
    if let Err(e) = run().await {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}

async fn run() -> Result<(), Box<dyn std::error::Error>> {
    if API_ID == 0 || API_HASH.is_empty() || PHONE.is_empty() {
        eprintln!("Fill in API_ID, API_HASH and PHONE at the top of chat_history.rs");
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
    println!("Logged in as {display} (id={})\n", me.id);

    let label = if PEER == "me" {
        "Saved Messages".to_string()
    } else {
        PEER.to_string()
    };
    println!("Last {LIMIT} messages from {label}:\n{}", "-".repeat(60));

    // This single call is what no Bot API wrapper can replicate.
    // It reads real message history directly over MTProto.
    let messages = client.get_message_history(PEER, LIMIT, 0).await?;

    if messages.is_empty() {
        println!("(no messages)");
        return Ok(());
    }

    for msg in &messages {
        let id = msg.id();
        let ts = msg
            .date_utc()
            .map(|d| d.format("%Y-%m-%d %H:%M:%S UTC").to_string())
            .unwrap_or_else(|| format!("unix={}", msg.date()));
        let body = msg.text().unwrap_or("").trim();
        let snippet = if body.is_empty() {
            "(media or service message)"
        } else {
            &body[..body.floor_char_boundary(120)]
        };
        println!("[{id:>8}] {ts}  {snippet}");
    }

    println!("{}\nFetched {} message(s).", "-".repeat(60), messages.len());
    Ok(())
}

async fn login(client: &Client) -> Result<(), Box<dyn std::error::Error>> {
    use ferogram::SignInError;
    use std::io::{self, BufRead, Write};

    fn prompt(msg: &str) -> io::Result<String> {
        print!("{msg}");
        io::stdout().flush()?;
        let mut line = String::new();
        io::stdin().lock().read_line(&mut line)?;
        Ok(line.trim().to_string())
    }

    let token = client.request_login_code(PHONE).await?;
    let code = prompt("Enter the code Telegram sent you: ")?;
    match client.sign_in(&token, &code).await {
        Ok(name) => println!("Signed in as {name}"),
        Err(SignInError::PasswordRequired(pw)) => {
            let pass = prompt(&format!(
                "2FA password (hint: {}): ",
                pw.hint().unwrap_or("none")
            ))?;
            client.check_password(*pw, pass.trim()).await?;
        }
        Err(SignInError::SignUpRequired) => {
            eprintln!("Phone not registered on Telegram.");
            std::process::exit(1);
        }
        Err(e) => return Err(e.into()),
    }
    Ok(())
}
