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

//! File type detection and metadata extraction.
//!
//! `detect_mime` sniffs the first bytes of a file for its true MIME type,
//! regardless of file extension. `FileMetadata` carries optional width,
//! height, and duration extracted from common formats.

/// Detected MIME type and optional media metadata.
#[derive(Debug, Clone)]
pub struct FileInfo {
    /// True MIME type detected from file bytes (e.g. `"video/mp4"`).
    /// Falls back to extension-based guess, then `"application/octet-stream"`.
    pub mime_type: String,
    /// Pixel width for images and videos (if detectable).
    pub width: Option<u32>,
    /// Pixel height for images and videos (if detectable).
    pub height: Option<u32>,
    /// Duration in seconds for video and audio (if detectable).
    pub duration: Option<f64>,
}

/// Detect MIME type from file bytes using magic-byte signatures.
///
/// Requires at most the first 16 bytes. Extension is used only as a fallback.
pub fn detect_mime(bytes: &[u8], name: &str) -> String {
    if let Some(kind) = infer::get(bytes) {
        return kind.mime_type().to_string();
    }
    // infer fallback: extension-based via mime_guess
    mime_guess::from_path(name)
        .first_or_octet_stream()
        .to_string()
}

/// Extract all available metadata from file bytes.
///
/// Reads only as much of `bytes` as needed for each format.
/// For video duration, pass the full file bytes (or use `file_info_from_path`
/// which reads the file itself).
pub fn file_info(bytes: &[u8], name: &str) -> FileInfo {
    let mime_type = detect_mime(bytes, name);
    let (width, height) = image_dimensions(bytes, &mime_type);
    let duration = video_duration(bytes, &mime_type);
    FileInfo {
        mime_type,
        width,
        height,
        duration,
    }
}

/// Extract metadata from a file on disk without loading it fully into memory.
///
/// Uses async file I/O: reads only the header bytes needed for MIME/dimension
/// detection, then seeks for duration if needed.
pub async fn file_info_from_path(path: impl AsRef<std::path::Path>) -> std::io::Result<FileInfo> {
    use tokio::io::AsyncReadExt;
    let path = path.as_ref();
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
    let mut f = tokio::fs::File::open(path).await?;

    // Read first 64 KB; enough for MIME, image dimensions, and short video headers.
    let mut header = vec![0u8; 65536];
    let n = f.read(&mut header).await?;
    header.truncate(n);

    let mime_type = detect_mime(&header, name);
    let (width, height) = image_dimensions(&header, &mime_type);

    // For video duration we need the full file; use the path directly.
    let duration = video_duration_from_path(path, &mime_type).await;

    Ok(FileInfo {
        mime_type,
        width,
        height,
        duration,
    })
}

// Extract pixel dimensions from common image/video headers.
fn image_dimensions(bytes: &[u8], mime: &str) -> (Option<u32>, Option<u32>) {
    match mime {
        "image/png" => png_dimensions(bytes),
        "image/jpeg" => jpeg_dimensions(bytes),
        "image/gif" => gif_dimensions(bytes),
        "image/webp" => webp_dimensions(bytes),
        _ => (None, None),
    }
}

fn png_dimensions(b: &[u8]) -> (Option<u32>, Option<u32>) {
    // PNG: 8-byte sig + 4-byte len + "IHDR" + 4-byte width + 4-byte height
    if b.len() < 24 {
        return (None, None);
    }
    if &b[0..8] != b"\x89PNG\r\n\x1a\n" {
        return (None, None);
    }
    let Ok(wb): Result<[u8; 4], _> = b[16..20].try_into() else {
        return (None, None);
    };
    let Ok(hb): Result<[u8; 4], _> = b[20..24].try_into() else {
        return (None, None);
    };
    let w = u32::from_be_bytes(wb);
    let h = u32::from_be_bytes(hb);
    (Some(w), Some(h))
}

fn jpeg_dimensions(b: &[u8]) -> (Option<u32>, Option<u32>) {
    if b.len() < 4 || b[0] != 0xFF || b[1] != 0xD8 {
        return (None, None);
    }
    let mut i = 2;
    while i + 9 < b.len() {
        if b[i] != 0xFF {
            break;
        }
        let marker = b[i + 1];
        // SOF markers: C0, C1, C2
        if matches!(marker, 0xC0..=0xC2) && i + 9 < b.len() {
            let h = u16::from_be_bytes([b[i + 5], b[i + 6]]) as u32;
            let w = u16::from_be_bytes([b[i + 7], b[i + 8]]) as u32;
            return (Some(w), Some(h));
        }
        if i + 4 > b.len() {
            break;
        }
        let len = u16::from_be_bytes([b[i + 2], b[i + 3]]) as usize;
        i += 2 + len;
    }
    (None, None)
}

fn gif_dimensions(b: &[u8]) -> (Option<u32>, Option<u32>) {
    // GIF: 6-byte header + 2-byte width (LE) + 2-byte height (LE)
    if b.len() < 10 {
        return (None, None);
    }
    if &b[0..6] != b"GIF87a" && &b[0..6] != b"GIF89a" {
        return (None, None);
    }
    let w = u16::from_le_bytes([b[6], b[7]]) as u32;
    let h = u16::from_le_bytes([b[8], b[9]]) as u32;
    (Some(w), Some(h))
}

fn webp_dimensions(b: &[u8]) -> (Option<u32>, Option<u32>) {
    // RIFF....WEBPVP8 or WEBPVP8L or WEBPVP8X
    if b.len() < 30 || &b[0..4] != b"RIFF" || &b[8..12] != b"WEBP" {
        return (None, None);
    }
    match &b[12..16] {
        b"VP8 " if b.len() >= 30 => {
            // VP8 bitstream: skip 10 bytes of chunk header, then 3 bytes tag, then w/h
            let w = (u16::from_le_bytes([b[26], b[27]]) & 0x3FFF) as u32;
            let h = (u16::from_le_bytes([b[28], b[29]]) & 0x3FFF) as u32;
            (Some(w), Some(h))
        }
        b"VP8L" if b.len() >= 21 => {
            let Ok(arr): Result<[u8; 4], _> = b[17..21].try_into() else {
                return (None, None);
            };
            let bits = u32::from_le_bytes(arr);
            let w = (bits & 0x3FFF) + 1;
            let h = ((bits >> 14) & 0x3FFF) + 1;
            (Some(w), Some(h))
        }
        b"VP8X" if b.len() >= 30 => {
            // VP8X: canvas width-1 at bytes 24-26 (24-bit LE), height-1 at 27-29
            let w = u32::from_le_bytes([b[24], b[25], b[26], 0]) + 1;
            let h = u32::from_le_bytes([b[27], b[28], b[29], 0]) + 1;
            (Some(w), Some(h))
        }
        _ => (None, None),
    }
}

fn video_duration(_bytes: &[u8], mime: &str) -> Option<f64> {
    // For in-memory bytes, only attempt if we have enough data and it's mp4/mov.
    if !matches!(
        mime,
        "video/mp4" | "video/quicktime" | "audio/mp4" | "audio/x-m4a"
    ) {
        return None;
    }
    // mp4 crate needs a Reader; wrap bytes in Cursor.
    // Only available under `experimental` feature.
    #[cfg(feature = "experimental")]
    {
        use std::io::Cursor;
        let cursor = Cursor::new(_bytes);
        if let Ok(mp4) = mp4::Mp4Reader::read_header(cursor, _bytes.len() as u64) {
            return Some(mp4.duration().as_secs_f64());
        }
    }
    None
}

async fn video_duration_from_path(_path: &std::path::Path, mime: &str) -> Option<f64> {
    if !matches!(
        mime,
        "video/mp4" | "video/quicktime" | "audio/mp4" | "audio/x-m4a"
    ) {
        return None;
    }
    #[cfg(feature = "experimental")]
    {
        // mp4 crate is sync; run in blocking thread.
        let path = _path.to_path_buf();
        return tokio::task::spawn_blocking(move || {
            let f = std::fs::File::open(&path).ok()?;
            let size = f.metadata().ok()?.len();
            let reader = std::io::BufReader::new(f);
            mp4::Mp4Reader::read_header(reader, size)
                .ok()
                .map(|m| m.duration().as_secs_f64())
        })
        .await
        .ok()
        .flatten();
    }
    #[allow(unreachable_code)]
    None
}
