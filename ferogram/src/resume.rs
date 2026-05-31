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

//! Experimental: persistent transfer checkpoints.
//!
//! Enabled with `features = ["experimental"]`. Off by default.
//!
//! Checkpoints are stored as JSON files under `.ferogram-transfers/` next to
//! the session file. They are deleted automatically on successful completion.
//!
//! # Download checkpoint
//!
//! Stores the byte offset reached so far. On resume, download starts from
//! that offset instead of 0.
//!
//! # Upload checkpoint
//!
//! Stores the last confirmed part number. On resume, already-uploaded parts
//! are skipped. Note: Telegram invalidates upload sessions after ~1 hour, so
//! resuming uploads that stalled longer than that will restart from 0.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Checkpoint data for a download in progress.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DownloadCheckpoint {
    /// Unique key derived from the file location (dc + access hash + file id).
    pub key: String,
    /// Byte offset of the next chunk to fetch.
    pub offset: i64,
    /// Total file size in bytes (0 if unknown).
    pub total: u64,
    /// SHA-256 hex of bytes received so far (rolling, for integrity check on completion).
    pub sha256_partial: String,
}

/// Checkpoint data for an upload in progress.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UploadCheckpoint {
    /// Unique key: sha256 of the first 64 KB of the file + file name.
    pub key: String,
    /// Telegram file_id assigned at the start of this upload session.
    pub file_id: i64,
    /// Last successfully uploaded part number (0-indexed).
    pub last_part: i32,
    /// Total number of parts.
    pub total_parts: i32,
    /// Part size in bytes.
    pub part_size: usize,
    /// Total file size in bytes.
    pub total: u64,
    /// Whether the big-file API (saveBigFilePart) is used.
    pub big: bool,
    /// File name.
    pub name: String,
    /// MIME type.
    pub mime_type: String,
    /// Unix ms timestamp when this upload session started (Telegram sessions expire ~1h).
    pub started_ms: u64,
}

/// Directory manager for checkpoint files.
pub struct CheckpointStore {
    dir: PathBuf,
}

impl CheckpointStore {
    /// Create or open the checkpoint store at `<session_dir>/.ferogram-transfers/`.
    pub async fn open(session_path: impl AsRef<Path>) -> std::io::Result<Self> {
        let dir = session_path
            .as_ref()
            .parent()
            .unwrap_or(Path::new("."))
            .join(".ferogram-transfers");
        tokio::fs::create_dir_all(&dir).await?;
        Ok(Self { dir })
    }

    fn path_for(&self, key: &str, prefix: &str) -> PathBuf {
        // Sanitize key: keep only alphanumeric and hyphens.
        let safe: String = key
            .chars()
            .map(|c| {
                if c.is_alphanumeric() || c == '-' {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        self.dir.join(format!("{prefix}_{safe}.json"))
    }

    /// Path for the partial bytes file for a download in progress.
    pub fn partial_path(&self, key: &str) -> std::path::PathBuf {
        let safe: String = key
            .chars()
            .map(|c| {
                if c.is_alphanumeric() || c == '-' {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        self.dir.join(format!("dl_{safe}.partial"))
    }

    pub async fn load_download(&self, key: &str) -> Option<DownloadCheckpoint> {
        let path = self.path_for(key, "dl");
        let bytes = tokio::fs::read(&path).await.ok()?;
        serde_json::from_slice(&bytes).ok()
    }

    pub async fn save_download(&self, cp: &DownloadCheckpoint) {
        let path = self.path_for(&cp.key, "dl");
        if let Ok(json) = serde_json::to_vec_pretty(cp) {
            let _ = tokio::fs::write(path, json).await;
        }
    }

    pub async fn delete_download(&self, key: &str) {
        let path = self.path_for(key, "dl");
        let _ = tokio::fs::remove_file(path).await;
    }

    pub async fn load_upload(&self, key: &str) -> Option<UploadCheckpoint> {
        let path = self.path_for(key, "ul");
        let bytes = tokio::fs::read(&path).await.ok()?;
        serde_json::from_slice(&bytes).ok()
    }

    pub async fn save_upload(&self, cp: &UploadCheckpoint) {
        let path = self.path_for(&cp.key, "ul");
        if let Ok(json) = serde_json::to_vec_pretty(cp) {
            let _ = tokio::fs::write(path, json).await;
        }
    }

    pub async fn delete_upload(&self, key: &str) {
        let path = self.path_for(key, "ul");
        let _ = tokio::fs::remove_file(path).await;
    }
}

/// Build a stable download key from dc_id and the serialized location.
pub fn download_key(dc_id: i32, location: &tl::enums::InputFileLocation) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(dc_id.to_le_bytes());
    // Use debug repr as a stable-enough key. A real impl would TL-serialize.
    h.update(format!("{location:?}").as_bytes());
    format!("{:x}", h.finalize())
}

/// Build a stable upload key from the first 64 KB of data + file name.
pub fn upload_key(data: &[u8], name: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(&data[..data.len().min(65536)]);
    h.update(name.as_bytes());
    format!("{:x}", h.finalize())
}

/// Compute SHA-256 of a byte slice. Returns hex string.
pub fn sha256_hex(data: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    format!("{:x}", Sha256::digest(data))
}

/// Current unix milliseconds.
pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

/// Telegram upload sessions are valid for roughly 1 hour.
pub const UPLOAD_SESSION_TTL_MS: u64 = 55 * 60 * 1000;
