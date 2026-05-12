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

//! List every dialog (chat) in the account, newest first, with unread counts.
//!
//! Enumerating the full dialog list is impossible over the Bot API - bots only
//! learn about chats they are added to. MTProto gives you the complete picture
//! across all DMs, groups, channels and bots in one paginated walk.
//!
//! Run:
//!   cargo run --example dialogs_list
//!
//! Fill in API_ID, API_HASH and PHONE below, then run. Session is reused on
//! subsequent runs.

use ferogram::{Client, TransportKind};

const API_ID: i32 = 0; // from https://my.telegram.org
const API_HASH: &str = ""; // from https://my.telegram.org
const PHONE: &str = ""; // e.g. "+15551234567"

/// Stop after printing this many dialogs. Set to 0 for no limit (walks the
/// entire account - may take a few seconds for large accounts).
const MAX_DIALOGS: usize = 50;

#[tokio::main]
async fn main() {
    if let Err(e) = run().await {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}

async fn run() -> Result<(), Box<dyn std::error::Error>> {
    if API_ID == 0 || API_HASH.is_empty() || PHONE.is_empty() {
        eprintln!("Fill in API_ID, API_HASH and PHONE at the top of dialogs_list.rs");
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

    let limit_label = if MAX_DIALOGS == 0 {
        "all".to_string()
    } else {
        MAX_DIALOGS.to_string()
    };
    println!("Dialogs ({limit_label}):\n{}", "-".repeat(60));

    // iter_dialogs() pages through GetDialogs automatically.
    // Each Dialog carries the chat entity and the top message already resolved.
    let mut iter = client.iter_dialogs();
    let mut n = 0usize;
    let mut total_unread = 0i32;

    while let Some(dialog) = iter.next(&client).await? {
        n += 1;
        let unread = dialog.unread_count();
        total_unread += unread;

        let badge = if unread > 0 {
            format!("  [{unread} unread]")
        } else {
            String::new()
        };

        // Print total once we get it back from the first server response.
        if n == 1 {
            if let Some(total) = iter.total() {
                println!("Total dialogs reported by server: {total}\n");
            }
        }

        println!("{n:>4}. {}{badge}", dialog.title());

        if MAX_DIALOGS > 0 && n >= MAX_DIALOGS {
            println!("... (truncated at {MAX_DIALOGS})");
            break;
        }
    }

    println!(
        "{}\nShowed {n} dialog(s), {total_unread} total unread message(s).",
        "-".repeat(60)
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
