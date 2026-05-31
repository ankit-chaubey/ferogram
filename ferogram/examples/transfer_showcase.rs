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

//! Showcase for all transfer APIs added in 0.6.0.
//!
//! Covers:
//!   1. Upload with progress + pause/resume + cancel
//!   2. Download with progress
//!   3. Auto media type selection (as_auto_media)
//!   4. Typed errors (InvocationErrorExt / ErrorKind)
//!   5. Resumable upload   (feature = "experimental")
//!   6. Resumable download (feature = "experimental")
//!
//! Run:
//!   cargo run --example transfer_showcase
//!   cargo run --example transfer_showcase --features experimental
//!
//! Fill in API_ID, API_HASH, and TARGET_PEER below, then send a file to any
//! chat; the bot will download it, re-upload it, and demonstrate every new API.

use std::time::Duration;

use ferogram::{
    Client, ErrorKind, InvocationErrorExt, TransferError, TransferHandle, TransferProgress,
};

const API_ID: i32 = 0; // from https://my.telegram.org
const API_HASH: &str = ""; // from https://my.telegram.org

// Username, phone, or numeric ID of the chat to use for the showcase.
const TARGET_PEER: &str = "me"; // "me" = Saved Messages

#[tokio::main]
async fn main() {
    if API_ID == 0 || API_HASH.is_empty() {
        eprintln!("Fill in API_ID and API_HASH at the top of transfer_showcase.rs");
        std::process::exit(1);
    }

    tracing_subscriber::fmt()
        .with_target(true)
        .with_env_filter("ferogram::transfer=debug,ferogram=info")
        .init();

    if let Err(e) = run().await {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}

async fn run() -> Result<(), Box<dyn std::error::Error>> {
    #[cfg(not(feature = "experimental"))]
    let (client, _shutdown) =
        Client::quick_connect("transfer_showcase.session", API_ID, API_HASH).await?;

    #[cfg(feature = "experimental")]
    let (client, _shutdown) = Client::builder()
        .api_id(API_ID)
        .api_hash(API_HASH)
        .session("transfer_showcase.session")
        .experimental_features(ExperimentalFeatures {
            resumable_transfers: true,
            ..Default::default()
        })
        .connect()
        .await?;

    let me = client.get_me().await?;
    println!(
        "Logged in as @{} (id={})\n",
        me.username.as_deref().unwrap_or("?"),
        me.id
    );

    section("Typed errors");
    demo_typed_errors();

    section("Upload with progress + pause/resume/cancel");
    demo_upload_controls(&client).await?;

    section("Auto media type selection");
    demo_auto_media(&client).await?;

    section("Download with progress");
    demo_download_progress(&client).await?;

    #[cfg(feature = "experimental")]
    {
        section("Resumable upload (experimental)");
        demo_resumable_upload(&client).await?;

        section("Resumable download (experimental)");
        demo_resumable_download(&client).await?;
    }

    #[cfg(not(feature = "experimental"))]
    println!(
        "Resumable transfer demos skipped.\n\
         Re-run with --features experimental to enable them."
    );

    println!("\nAll transfer showcase sections complete.");
    Ok(())
}

fn demo_typed_errors() {
    // TransferError display strings.
    assert_eq!(
        TransferError::Cancelled.to_string(),
        "transfer cancelled by caller"
    );
    assert_eq!(
        TransferError::FloodWait { seconds: 42 }.to_string(),
        "Telegram rate limit reached. Retry after 42 seconds."
    );
    assert_eq!(
        TransferError::Rpc {
            code: 400,
            name: "FILE_PART_INVALID".into()
        }
        .to_string(),
        "Telegram error (400): FILE_PART_INVALID"
    );
    println!("  TransferError::Cancelled   -> \"transfer cancelled by caller\"");
    println!(
        "  TransferError::FloodWait   -> \"Telegram rate limit reached. Retry after 42 seconds.\""
    );
    println!("  TransferError::Rpc         -> \"Telegram error (400): FILE_PART_INVALID\"");

    // .kind() on an InvocationError converted from a TransferError.
    use ferogram::InvocationError;
    let flood: InvocationError = TransferError::FloodWait { seconds: 30 }.into();
    assert_eq!(flood.kind(), ErrorKind::FloodWait(30));
    println!("  FloodWait -> InvocationError -> .kind() == ErrorKind::FloodWait(30)  ok");

    // .friendly() produces a human string.
    let msg = flood.friendly();
    assert!(msg.contains("30"), "expected seconds in friendly: {msg}");
    println!("  .friendly() = \"{msg}\"  ok");

    // ErrorKind::Cancelled.
    let cancelled: InvocationError = TransferError::Cancelled.into();
    assert_eq!(cancelled.kind(), ErrorKind::Cancelled);
    println!("  Cancelled -> .kind() == ErrorKind::Cancelled  ok");

    // poll_pause_cancel returns Err(Cancelled) after cancel().
    let rt = tokio::runtime::Handle::current();
    let handle = TransferHandle::new();
    handle.cancel();
    let res = rt.block_on(handle.poll_pause_cancel());
    assert!(matches!(res, Err(TransferError::Cancelled)));
    println!("  handle.cancel() -> poll_pause_cancel() -> Err(Cancelled)  ok");
}

async fn demo_upload_controls(client: &Client) -> Result<(), Box<dyn std::error::Error>> {
    // Create a 2 MB dummy payload.
    let data = vec![0u8; 2 * 1024 * 1024];
    let handle = TransferHandle::new();
    let h_ctl = handle.clone();

    // Spawn a task that pauses for 300 ms then resumes, then cancels after another 300 ms.
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(300)).await;
        println!("  [ctl] pausing upload...");
        h_ctl.pause();
        tokio::time::sleep(Duration::from_millis(300)).await;
        println!("  [ctl] resuming upload...");
        h_ctl.resume();
        tokio::time::sleep(Duration::from_millis(300)).await;
        println!("  [ctl] cancelling upload...");
        h_ctl.cancel();
    });

    let result = client
        .upload_with_progress(
            std::io::Cursor::new(data),
            "showcase_dummy.bin",
            &handle,
            |p: TransferProgress| {
                println!(
                    "  upload: {:.0}% | {} | ETA {}s",
                    p.percent(),
                    p.speed_human(),
                    p.eta_secs()
                );
            },
        )
        .await;

    match result {
        Err(e) if e.kind() == ErrorKind::Cancelled => {
            println!("  upload cancelled as expected via ErrorKind::Cancelled  ok");
        }
        Err(e) => println!("  upload ended with: {}", e.friendly()),
        Ok(_) => println!("  upload completed (cancel fired after completion)"),
    }

    Ok(())
}

async fn demo_auto_media(client: &Client) -> Result<(), Box<dyn std::error::Error>> {
    // Upload a tiny valid JPEG (1x1 white pixel).
    let jpeg_bytes: Vec<u8> = vec![
        0xff, 0xd8, 0xff, 0xe0, 0x00, 0x10, 0x4a, 0x46, 0x49, 0x46, 0x00, 0x01, 0x01, 0x00, 0x00,
        0x01, 0x00, 0x01, 0x00, 0x00, 0xff, 0xdb, 0x00, 0x43, 0x00, 0x08, 0x06, 0x06, 0x07, 0x06,
        0x05, 0x08, 0x07, 0x07, 0x07, 0x09, 0x09, 0x08, 0x0a, 0x0c, 0x14, 0x0d, 0x0c, 0x0b, 0x0b,
        0x0c, 0x19, 0x12, 0x13, 0x0f, 0x14, 0x1d, 0x1a, 0x1f, 0x1e, 0x1d, 0x1a, 0x1c, 0x1c, 0x20,
        0x24, 0x2e, 0x27, 0x20, 0x22, 0x2c, 0x23, 0x1c, 0x1c, 0x28, 0x37, 0x29, 0x2c, 0x30, 0x31,
        0x34, 0x34, 0x34, 0x1f, 0x27, 0x39, 0x3d, 0x38, 0x32, 0x3c, 0x2e, 0x33, 0x34, 0x32, 0xff,
        0xc0, 0x00, 0x0b, 0x08, 0x00, 0x01, 0x00, 0x01, 0x01, 0x01, 0x11, 0x00, 0xff, 0xc4, 0x00,
        0x1f, 0x00, 0x00, 0x01, 0x05, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b,
        0xff, 0xc4, 0x00, 0xb5, 0x10, 0x00, 0x02, 0x01, 0x03, 0x03, 0x02, 0x04, 0x03, 0x05, 0x05,
        0x04, 0x04, 0x00, 0x00, 0x01, 0x7d, 0x01, 0x02, 0x03, 0x00, 0x04, 0x11, 0x05, 0x12, 0x21,
        0x31, 0x41, 0x06, 0x13, 0x51, 0x61, 0x07, 0x22, 0x71, 0x14, 0x32, 0x81, 0x91, 0xa1, 0x08,
        0x23, 0x42, 0xb1, 0xc1, 0x15, 0x52, 0xd1, 0xf0, 0x24, 0x33, 0x62, 0x72, 0x82, 0x09, 0x0a,
        0x16, 0x17, 0x18, 0x19, 0x1a, 0x25, 0x26, 0x27, 0x28, 0x29, 0x2a, 0x34, 0x35, 0x36, 0x37,
        0x38, 0x39, 0x3a, 0x43, 0x44, 0x45, 0x46, 0x47, 0x48, 0x49, 0x4a, 0x53, 0x54, 0x55, 0x56,
        0x57, 0x58, 0x59, 0x5a, 0x63, 0x64, 0x65, 0x66, 0x67, 0x68, 0x69, 0x6a, 0x73, 0x74, 0x75,
        0x76, 0x77, 0x78, 0x79, 0x7a, 0x83, 0x84, 0x85, 0x86, 0x87, 0x88, 0x89, 0x8a, 0x92, 0x93,
        0x94, 0x95, 0x96, 0x97, 0x98, 0x99, 0x9a, 0xa2, 0xa3, 0xa4, 0xa5, 0xa6, 0xa7, 0xa8, 0xa9,
        0xaa, 0xb2, 0xb3, 0xb4, 0xb5, 0xb6, 0xb7, 0xb8, 0xb9, 0xba, 0xc2, 0xc3, 0xc4, 0xc5, 0xc6,
        0xc7, 0xc8, 0xc9, 0xca, 0xd2, 0xd3, 0xd4, 0xd5, 0xd6, 0xd7, 0xd8, 0xd9, 0xda, 0xe1, 0xe2,
        0xe3, 0xe4, 0xe5, 0xe6, 0xe7, 0xe8, 0xe9, 0xea, 0xf1, 0xf2, 0xf3, 0xf4, 0xf5, 0xf6, 0xf7,
        0xf8, 0xf9, 0xfa, 0xff, 0xda, 0x00, 0x08, 0x01, 0x01, 0x00, 0x00, 0x3f, 0x00, 0xfb, 0xd7,
        0xff, 0xd9,
    ];

    let uploaded = client
        .upload(std::io::Cursor::new(jpeg_bytes), "showcase.jpg", None)
        .await?;

    // as_auto_media() should pick photo for a JPEG.
    let media = uploaded.as_auto_media();
    let kind = match &media {
        ferogram::tl::enums::InputMedia::UploadedPhoto(_) => "InputMedia::UploadedPhoto",
        ferogram::tl::enums::InputMedia::UploadedDocument(_) => "InputMedia::UploadedDocument",
        _ => "other",
    };
    println!("  showcase.jpg -> as_auto_media() -> {kind}  ok");

    // Send it to Saved Messages using the Into<InputMedia> impl on UploadedFile.
    // (uploaded consumed by as_auto_media, so re-upload for the send demo)
    let uploaded2 = client
        .upload(
            std::io::Cursor::new(vec![0u8; 512]),
            "showcase_doc.bin",
            None,
        )
        .await?;

    // From<UploadedFile> for InputMedia; pass directly to send_file.
    client
        .send_file(TARGET_PEER, uploaded2, &Default::default())
        .await
        .map(|_| println!("  send_file(uploaded2) via From<UploadedFile>  ok"))
        .unwrap_or_else(|e| println!("  send_file: {}", e.friendly()));

    Ok(())
}

async fn demo_download_progress(client: &Client) -> Result<(), Box<dyn std::error::Error>> {
    // Upload something first so we have a media to download.
    let data = vec![42u8; 256 * 1024]; // 256 KB
    let uploaded = client
        .upload(std::io::Cursor::new(data), "showcase_dl.bin", None)
        .await?;
    let msg = client
        .send_file(TARGET_PEER, uploaded, &Default::default())
        .await?;

    let media = msg.media().ok_or("sent message has no media")?;

    let handle = TransferHandle::new();
    let mut buf = Vec::new();

    let bytes = client
        .download_with_progress(&media, &mut buf, &handle, |p: TransferProgress| {
            println!(
                "  download: {:.0}% | {} | ETA {}s",
                p.percent(),
                p.speed_human(),
                p.eta_secs()
            );
        })
        .await?;

    println!("  downloaded {bytes} bytes  ok");
    Ok(())
}

#[cfg(feature = "experimental")]
async fn demo_resumable_upload(client: &Client) -> Result<(), Box<dyn std::error::Error>> {
    // 512 KB file; small enough to finish quickly, large enough to have parts.
    let data = vec![1u8; 512 * 1024];
    let handle = TransferHandle::new();

    // First call: cancel partway through to simulate an interruption.
    let h_ctl = handle.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(100)).await;
        h_ctl.cancel();
    });

    let r1 = client
        .upload_resumable(data.clone(), "resumable_up.bin", &handle, |p| {
            println!("  upload_resumable (run 1): {:.0}%", p.percent());
        })
        .await;
    println!(
        "  run 1 result: {} (checkpoint saved if parts were uploaded)",
        if r1.is_err() {
            "interrupted"
        } else {
            "completed"
        }
    );

    // Second call: resumes from the checkpoint.
    let handle2 = TransferHandle::new();
    let uploaded = client
        .upload_resumable(data, "resumable_up.bin", &handle2, |p| {
            println!("  upload_resumable (run 2): {:.0}%", p.percent());
        })
        .await?;

    println!("  run 2 completed, mime={}  ok", uploaded.mime_type());
    Ok(())
}

#[cfg(feature = "experimental")]
async fn demo_resumable_download(client: &Client) -> Result<(), Box<dyn std::error::Error>> {
    // Upload a 1.5 MB file to use as download target.
    let payload = vec![7u8; 3 * 512 * 1024];
    let uploaded = client
        .upload(
            std::io::Cursor::new(payload.clone()),
            "resumable_dl.bin",
            None,
        )
        .await?;
    let msg = client
        .send_file(TARGET_PEER, uploaded, &Default::default())
        .await?;
    let media = msg.media().ok_or("no media on sent message")?;

    // First call: cancel after 200 ms to simulate network interruption.
    let handle1 = TransferHandle::new();
    let h_ctl = handle1.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(200)).await;
        h_ctl.cancel();
    });

    let mut buf1 = Vec::new();
    let r1 = client
        .download_resumable(&media, &mut buf1, &handle1, |p| {
            println!(
                "  download_resumable (run 1): {:.0}% | {}",
                p.percent(),
                p.speed_human()
            );
        })
        .await;
    println!(
        "  run 1 result: {} ({} bytes in buf, partial file saved)",
        if r1.is_err() {
            "interrupted"
        } else {
            "completed"
        },
        buf1.len()
    );

    // Second call: restores partial bytes from disk and resumes.
    let handle2 = TransferHandle::new();
    let mut buf2 = Vec::new();
    let bytes = client
        .download_resumable(&media, &mut buf2, &handle2, |p| {
            println!(
                "  download_resumable (run 2): {:.0}% | {}",
                p.percent(),
                p.speed_human()
            );
        })
        .await?;

    assert_eq!(bytes as usize, payload.len(), "size mismatch after resume");
    assert_eq!(buf2, payload, "content mismatch after resume");
    println!("  run 2 completed, {bytes} bytes, content verified  ok");
    Ok(())
}

fn section(name: &str) {
    let line = "─".repeat(60);
    println!("\n{line}\n  {name}\n{line}");
}
