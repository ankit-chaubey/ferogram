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

//! Schedule a message to be sent at a future time.
//!
//! Telegram stores the message server-side and delivers it at the scheduled
//! Unix timestamp - no process needs to stay running. Works for any peer:
//! Saved Messages, DMs, groups, or channels you have posting rights in.
//!
//! Run:
//!   cargo run --example schedule_message
//!
//! Fill in API_ID, API_HASH, PHONE, PEER, TEXT and SEND_IN_SECONDS below.

use chrono::Utc;
use ferogram::{Client, InputMessage, TransportKind};

const API_ID: i32 = 0; // from https://my.telegram.org
const API_HASH: &str = ""; // from https://my.telegram.org
const PHONE: &str = ""; // e.g. "+15551234567"

/// Where to send the scheduled message.
/// "me" sends to your own Saved Messages.
const PEER: &str = "me";

/// The message text.
const TEXT: &str = "This message was scheduled with ferogram 🦀";

/// How many seconds from now to deliver the message.
const SEND_IN_SECONDS: i64 = 60;

#[tokio::main]
async fn main() {
    if let Err(e) = run().await {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}

async fn run() -> Result<(), Box<dyn std::error::Error>> {
    if API_ID == 0 || API_HASH.is_empty() || PHONE.is_empty() {
        eprintln!("Fill in API_ID, API_HASH and PHONE at the top of schedule_message.rs");
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

    let deliver_at = Utc::now().timestamp() + SEND_IN_SECONDS;
    let deliver_str = chrono::DateTime::from_timestamp(deliver_at, 0)
        .map(|d| d.format("%Y-%m-%d %H:%M:%S UTC").to_string())
        .unwrap_or_else(|| deliver_at.to_string());

    println!("Scheduling message to {PEER}");
    println!("  Text    : {TEXT}");
    println!("  Deliver : {deliver_str} (in {SEND_IN_SECONDS}s)\n");

    // schedule_date() takes an i32 Unix timestamp.
    // Telegram stores the message and fires it at that time server-side.
    // You can close your app immediately after this call.
    let msg = client
        .send_message(
            PEER,
            InputMessage::text(TEXT).schedule_date(Some(deliver_at as i32)),
        )
        .await?;

    println!(
        "Scheduled. Message ID = {}  (appears in chat at {})",
        msg.id(),
        deliver_str,
    );
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
