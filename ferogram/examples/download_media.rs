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

//! Auto-downloads every incoming photo and document to a local downloads/
//! folder. Useful as a starting point for media archiving userbots.
//!
//! Run:
//!   cargo run --example download_media
//!
//! First run will prompt for your phone number, login code, and 2FA if set.
//! Files are saved to ./downloads/ relative to where you run the command.

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
        eprintln!("Fill in API_ID and API_HASH at the top of download_media.rs");
        std::process::exit(1);
    }

    let downloads = std::path::Path::new("downloads");
    tokio::fs::create_dir_all(downloads).await?;

    let (client, _shutdown) = Client::quick_connect("download.session", API_ID, API_HASH).await?;

    let me = client.get_me().await?;
    println!(
        "Logged in as {} ({})",
        me.first_name.as_deref().unwrap_or("?"),
        me.id
    );
    println!("Saving media to ./downloads/ ...");

    let mut stream = client.stream_updates();
    while let Some(upd) = stream.next().await {
        if let Update::NewMessage(msg) = upd {
            if msg.outgoing() {
                continue;
            }

            let has_photo = msg.photo().is_some();
            let has_doc = msg.document().is_some();

            if !has_photo && !has_doc {
                continue;
            }

            let msg_id = msg.id();

            let file_name = if has_photo {
                format!("photo_{msg_id}.jpg")
            } else {
                let doc = msg.document().unwrap();
                doc.file_name()
                    .map(|n| n.to_string())
                    .unwrap_or_else(|| format!("file_{msg_id}"))
            };

            let dest = downloads.join(&file_name);
            let dest_str = dest.to_string_lossy().into_owned();

            match msg.download_media(&dest_str).await {
                Ok(true) => {
                    let size = tokio::fs::metadata(&dest_str)
                        .await
                        .map(|m| m.len())
                        .unwrap_or(0);
                    println!("Saved: {dest_str}  ({} KB)", size / 1024);
                }
                Ok(false) => {
                    println!("Skipped msg {msg_id}: no download location.");
                }
                Err(e) => {
                    println!("Error downloading msg {msg_id}: {e}");
                }
            }
        }
    }

    Ok(())
}
