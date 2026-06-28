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

//! Bot that mirrors any file a user sends, showing live progress and a Cancel
//! button during both download and upload phases.
//!
//! Run:
//!   cargo run --example progress_transfer
//!
//! When prompted, enter your bot token (from @BotFather).
//! Send the bot any file/photo/video and watch it download then re-upload
//! with a live progress bar. Tap "Cancel" at any point to abort.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use ferogram::tl;
use ferogram::{Client, InputMessage, PeerRef, TransferHandle, TransferProgress, update::Update};

const API_ID: i32 = 0; // fill in from https://my.telegram.org
const API_HASH: &str = ""; // fill in from https://my.telegram.org

// Edit the Telegram message at most once per this many seconds (flood safety).
const CHAT_EDIT_INTERVAL_SECS: u64 = 5;

type TransferMap = Arc<Mutex<HashMap<i32, TransferHandle>>>;

#[tokio::main]
async fn main() {
    if let Err(e) = run().await {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}

async fn run() -> Result<(), Box<dyn std::error::Error>> {
    if API_ID == 0 || API_HASH.is_empty() {
        eprintln!("Fill in API_ID and API_HASH at the top of progress_transfer.rs");
        std::process::exit(1);
    }

    let (client, _shutdown) =
        Client::quick_connect("progress_transfer.session", API_ID, API_HASH).await?;

    let me = client.get_me().await?;
    println!(
        "Logged in as @{} (id={})",
        me.username.as_deref().unwrap_or("?"),
        me.id
    );
    println!("Send the bot any file to mirror it with progress.");

    let transfers: TransferMap = Arc::new(Mutex::new(HashMap::new()));

    let mut stream = client.stream_updates();
    while let Some(upd) = stream.next().await {
        match upd {
            Update::NewMessage(msg) if !msg.outgoing() => {
                if msg.media().is_none() {
                    continue;
                }
                let client = client.clone();
                let transfers = transfers.clone();
                tokio::spawn(async move {
                    if let Err(e) = handle_media(client, msg, transfers).await {
                        eprintln!("transfer error: {e}");
                    }
                });
            }

            Update::CallbackQuery(cb) => {
                let data = cb.data().unwrap_or_default();
                if let Some(id_str) = data.strip_prefix("cancel:") {
                    let msg_id: i32 = id_str.parse().unwrap_or(0);
                    let handle = transfers.lock().unwrap().remove(&msg_id);

                    let peer: PeerRef = cb
                        .chat_peer
                        .clone()
                        .map(PeerRef::from)
                        .unwrap_or_else(|| PeerRef::from(cb.user_id));

                    if let Some(h) = handle {
                        h.cancel();
                        let _ = client
                            .answer_callback_query(cb.query_id, Some("Cancelled."), false)
                            .await;
                        let _ = client
                            .edit_message(peer, msg_id, InputMessage::text("Transfer cancelled."))
                            .await;
                    } else {
                        let _ = client
                            .answer_callback_query(
                                cb.query_id,
                                Some("Already finished or cancelled."),
                                false,
                            )
                            .await;
                    }
                }
            }

            _ => {}
        }
    }
    Ok(())
}

async fn handle_media(
    client: Client,
    msg: ferogram::update::IncomingMessage,
    transfers: TransferMap,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let peer: PeerRef = msg
        .peer_id()
        .cloned()
        .map(PeerRef::from)
        .unwrap_or_else(|| PeerRef::from(msg.id() as i64));

    let media = msg.media().unwrap();
    let fname = file_name_from_media(&media);

    // Send initial status with placeholder Cancel button.
    let status = client
        .send_message(
            peer.clone(),
            InputMessage::html(format!(
                "Downloading <b>{fname}</b>...\n{} starting...",
                progress_bar(0.0)
            ))
            .reply_markup(cancel_kb(0)),
        )
        .await?;
    let sid = status.id();

    // Re-edit immediately with the real sid in the button data.
    let _ = client
        .edit_message(
            peer.clone(),
            sid,
            InputMessage::html(format!(
                "Downloading <b>{fname}</b>...\n{} starting...",
                progress_bar(0.0)
            ))
            .reply_markup(cancel_kb(sid)),
        )
        .await;

    // Download phase.
    let dl_handle = TransferHandle::new();
    transfers.lock().unwrap().insert(sid, dl_handle.clone());

    // Channel carries progress snapshots from the sync callback into async chat-edit task.
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<TransferProgress>();

    // Async task: reads from channel, edits chat every CHAT_EDIT_INTERVAL_SECS ticks.
    let chat_task = {
        let client = client.clone();
        let peer = peer.clone();
        let fname = fname.clone();
        let label = "Downloading";
        tokio::spawn(async move {
            let mut ticks = 0u64;
            while let Some(p) = rx.recv().await {
                ticks += 1;
                if ticks % CHAT_EDIT_INTERVAL_SECS == 0 {
                    let bar = progress_bar(p.percent());
                    let html = if p.total > 0 && p.speed_bps() > 512 {
                        format!(
                            "{label} <b>{fname}</b>...\n{bar} {:.0}%\n{} | ETA {}s",
                            p.percent(),
                            p.speed_human(),
                            p.eta_secs()
                        )
                    } else {
                        format!("{label} <b>{fname}</b>...\n{bar} {:.0}%", p.percent())
                    };
                    let _ = client
                        .edit_message(
                            peer.clone(),
                            sid,
                            InputMessage::html(html).reply_markup(cancel_kb(sid)),
                        )
                        .await;
                }
            }
        })
    };

    let mut buf = Vec::new();
    let fname2 = fname.clone();
    let ctl = dl_handle.clone();
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
            if ctl.is_cancelled() {
                break;
            }
            let p = ctl.progress();
            let bar = progress_bar(p.percent());
            if p.total > 0 && p.speed_bps() > 512 {
                println!(
                    "Downloading {fname2}: {bar} {:.0}% | {} | ETA {}s",
                    p.percent(),
                    p.speed_human(),
                    p.eta_secs()
                );
            } else {
                println!("Downloading {fname2}: {bar} {:.0}%", p.percent());
            }
            let _ = tx.send(p);
        }
    });
    let dl = client.download(&media, &mut buf, Some(&dl_handle)).await;

    chat_task.abort();
    println!("Downloading {fname}: {} 100% | done", progress_bar(100.0));

    if dl.is_err() || dl_handle.is_cancelled() {
        transfers.lock().unwrap().remove(&sid);
        let _ = client
            .edit_message(peer, sid, InputMessage::text("Transfer cancelled."))
            .await;
        return Ok(());
    }

    // Upload phase: new handle so cancel still routes to sid.
    let up_handle = TransferHandle::new();
    transfers.lock().unwrap().insert(sid, up_handle.clone());

    let _ = client
        .edit_message(
            peer.clone(),
            sid,
            InputMessage::html(format!(
                "Uploading <b>{fname}</b>...\n{} starting...",
                progress_bar(0.0)
            ))
            .reply_markup(cancel_kb(sid)),
        )
        .await;

    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<TransferProgress>();

    let chat_task = {
        let client = client.clone();
        let peer = peer.clone();
        let fname = fname.clone();
        let label = "Uploading";
        tokio::spawn(async move {
            let mut ticks = 0u64;
            while let Some(p) = rx.recv().await {
                ticks += 1;
                if ticks % CHAT_EDIT_INTERVAL_SECS == 0 {
                    let bar = progress_bar(p.percent());
                    let html = if p.total > 0 && p.speed_bps() > 512 {
                        format!(
                            "{label} <b>{fname}</b>...\n{bar} {:.0}%\n{} | ETA {}s",
                            p.percent(),
                            p.speed_human(),
                            p.eta_secs()
                        )
                    } else {
                        format!("{label} <b>{fname}</b>...\n{bar} {:.0}%", p.percent())
                    };
                    let _ = client
                        .edit_message(
                            peer.clone(),
                            sid,
                            InputMessage::html(html).reply_markup(cancel_kb(sid)),
                        )
                        .await;
                }
            }
        })
    };

    let fname2 = fname.clone();
    let ctl = up_handle.clone();
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
            if ctl.is_cancelled() {
                break;
            }
            let p = ctl.progress();
            let bar = progress_bar(p.percent());
            if p.total > 0 && p.speed_bps() > 512 {
                println!(
                    "Uploading {fname2}: {bar} {:.0}% | {} | ETA {}s",
                    p.percent(),
                    p.speed_human(),
                    p.eta_secs()
                );
            } else {
                println!("Uploading {fname2}: {bar} {:.0}%", p.percent());
            }
            let _ = tx.send(p);
        }
    });
    let up = client
        .upload(std::io::Cursor::new(buf), &fname, Some(&up_handle))
        .await;

    chat_task.abort();
    println!("Uploading {fname}: {} 100% | done", progress_bar(100.0));

    transfers.lock().unwrap().remove(&sid);

    match up {
        Err(_) => {
            let _ = client
                .edit_message(peer, sid, InputMessage::text("Transfer cancelled."))
                .await;
        }
        Ok(uploaded) => {
            let _ = client
                .edit_message(
                    peer.clone(),
                    sid,
                    InputMessage::html(format!("Sending <b>{fname}</b>...")),
                )
                .await;
            let _ = client
                .send_message(
                    peer.clone(),
                    InputMessage::text("").copy_media(uploaded.as_auto_media()),
                )
                .await;
            let _ = client
                .edit_message(
                    peer,
                    sid,
                    InputMessage::html(format!("<b>{fname}</b> mirrored successfully.")),
                )
                .await;
        }
    }

    Ok(())
}

fn file_name_from_media(media: &tl::enums::MessageMedia) -> String {
    use ferogram::media::Document;
    if let Some(doc) = Document::from_media(media) {
        for attr in &doc.raw.attributes {
            if let tl::enums::DocumentAttribute::Filename(f) = attr {
                return f.file_name.clone();
            }
        }
        return format!(
            "file.{}",
            doc.mime_type().split('/').last().unwrap_or("bin")
        );
    }
    "photo.jpg".to_string()
}

fn progress_bar(percent: f64) -> String {
    let filled = (percent / 10.0).round() as usize;
    let empty = 10usize.saturating_sub(filled);
    format!("{}{}", "▰".repeat(filled), "▱".repeat(empty))
}

fn cancel_kb(msg_id: i32) -> tl::enums::ReplyMarkup {
    tl::enums::ReplyMarkup::ReplyInlineMarkup(tl::types::ReplyInlineMarkup {
        rows: vec![tl::enums::KeyboardButtonRow::KeyboardButtonRow(
            tl::types::KeyboardButtonRow {
                buttons: vec![tl::enums::KeyboardButton::Callback(
                    tl::types::KeyboardButtonCallback {
                        requires_password: false,
                        style: None,
                        text: "Cancel".to_string(),
                        data: format!("cancel:{msg_id}").into_bytes(),
                    },
                )],
            },
        )],
    })
}
