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

//! List every member of a group or channel with their role.
//!
//! `channels.GetParticipants` returns the full member roster including
//! user IDs, names, usernames and roles (creator / admin / member / banned).
//! The Bot API exposes no equivalent - bots cannot enumerate chat members.
//! This is the foundation of every moderation tool and analytics dashboard
//! built on Telegram.
//!
//! Run:
//!   cargo run --example get_participants
//!
//! You must be a member of the target group/channel.
//! Fill in API_ID, API_HASH, PHONE and PEER below.

use ferogram::{Client, ParticipantStatus, TransportKind};

const API_ID: i32 = 0; // from https://my.telegram.org
const API_HASH: &str = ""; // from https://my.telegram.org
const PHONE: &str = ""; // e.g. "+15551234567"

/// Group or channel to inspect. Username, invite link, or integer peer ID.
/// Example: "@my_group" or "-1001234567890"
const PEER: &str = "@your_group";

/// How many members to fetch. Pass 0 for the server default (200).
const LIMIT: i32 = 50;

#[tokio::main]
async fn main() {
    if let Err(e) = run().await {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}

async fn run() -> Result<(), Box<dyn std::error::Error>> {
    if API_ID == 0 || API_HASH.is_empty() || PHONE.is_empty() || PEER == "@your_group" {
        eprintln!("Fill in API_ID, API_HASH, PHONE and PEER at the top of get_participants.rs");
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

    println!("Members of {PEER} (up to {LIMIT}):\n{}", "-".repeat(60));

    // get_participants(peer, limit) works for both basic groups and supergroups/channels.
    let participants = client.get_participants(PEER, LIMIT).await?;

    if participants.is_empty() {
        println!("No participants found (you may not be a member).");
        return Ok(());
    }

    let mut creators = 0u32;
    let mut admins = 0u32;
    let mut members = 0u32;

    for (i, p) in participants.iter().enumerate() {
        let u = &p.user;
        let name = format!(
            "{} {}",
            u.first_name.as_deref().unwrap_or(""),
            u.last_name.as_deref().unwrap_or("")
        )
        .trim()
        .to_string();
        let username = u
            .username
            .as_deref()
            .map(|s| format!(" @{s}"))
            .unwrap_or_default();
        let role = match &p.status {
            ParticipantStatus::Creator => {
                creators += 1;
                "👑 creator"
            }
            ParticipantStatus::Admin => {
                admins += 1;
                "🛡 admin"
            }
            ParticipantStatus::Member => {
                members += 1;
                "👤 member"
            }
            ParticipantStatus::Restricted => "🔇 restricted",
            ParticipantStatus::Banned => "🚫 banned",
            ParticipantStatus::Left => "🚪 left",
        };
        println!("{:>4}. [{:>12}]{username}  {name}  ({role})", i + 1, u.id);
    }

    println!(
        "{}\n{} participant(s): {} creator, {} admin(s), {} member(s).",
        "-".repeat(60),
        participants.len(),
        creators,
        admins,
        members,
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
