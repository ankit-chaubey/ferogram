// Copyright (c) Ankit Chaubey <ankitchaubey.dev@gmail.com>
// SPDX-License-Identifier: MIT OR Apache-2.0
//
// ferogram: async Telegram MTProto client in Rust
// https://github.com/ankit-chaubey/ferogram
//
//
// If you use or modify this code, keep this notice at the top of your file
// and include the LICENSE-MIT or LICENSE-APACHE file from this repository:
// https://github.com/ankit-chaubey/ferogram

//! Media upload, download, and typed wrappers.
//!
//! ## Upload
//! - [`Client::upload_file`]  : sequential (small files, < 10 MB)
//! - [`Client::upload_file_concurrent`]: parallel worker pool for large files
//! - [`Client::upload_stream`]: reads AsyncRead → calls upload_file
//!
//! ## Download
//! - [`Client::iter_download`]         : chunk-by-chunk streaming
//! - [`Client::download_media`]        : collect all bytes
//! - [`Client::download_media_concurrent`]: parallel multi-worker download
//!
//! ## Typed wrappers
//! [`Photo`], [`Document`], [`Sticker`]: typed wrappers over raw TL types.
//!
//! ## Downloadable trait
//! [`Downloadable`]: implemented by Photo, Document, Sticker so you can pass
//! any of them to `iter_download` / `download_media`.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};

use ferogram_tl_types as tl;
use ferogram_tl_types::{Cursor, Deserializable};
use tokio::io::AsyncRead;
use tokio::io::AsyncReadExt;
use tokio::sync::Mutex;

use crate::{Client, InvocationError};

/// A single item in a multi-media album send.
///
/// Build via [`AlbumItem::new`], then optionally chain `.caption()`, `.reply_to()`.
pub struct AlbumItem {
    pub media: tl::enums::InputMedia,
    pub caption: String,
    pub entities: Vec<tl::enums::MessageEntity>,
    pub reply_to: Option<i32>,
}

impl AlbumItem {
    pub fn new(media: tl::enums::InputMedia) -> Self {
        Self {
            media,
            caption: String::new(),
            entities: Vec::new(),
            reply_to: None,
        }
    }
    pub fn caption(mut self, text: impl Into<String>) -> Self {
        self.caption = text.into();
        self
    }
    pub fn reply_to(mut self, msg_id: Option<i32>) -> Self {
        self.reply_to = msg_id;
        self
    }
}

impl From<(tl::enums::InputMedia, String)> for AlbumItem {
    fn from((media, caption): (tl::enums::InputMedia, String)) -> Self {
        Self::new(media).caption(caption)
    }
}

/// Download chunk size for sequential / small-file paths.
/// 256 KB: safe at all file sizes.
pub const DOWNLOAD_CHUNK_SIZE: i32 = 256 * 1024;

/// Adaptive download chunk size for the parallel path.
///
/// Telegram's `upload.getFile` rule:
///   `(offset + limit)` must not cross a 1 MB boundary  - so using 512 KB is safe
///   as long as offsets are multiples of 512 KB, which our sequential part
///   numbering guarantees (`part * chunk`).  1 MB chunks are intentionally
///   avoided here; they require extra offset-alignment care and gain little over
///   512 KB in practice.
///
/// | File size  | Chunk  | Rationale                        |
/// |------------|--------|----------------------------------|
/// | < 50 MB    | 256 KB | small files: fewer parts is fine |
/// | ≥ 50 MB    | 512 KB | large files: halve RPC round-trips|
pub fn download_chunk_size(file_size: usize) -> i32 {
    if file_size < 50 * 1024 * 1024 {
        256 * 1024 // 256 KB
    } else if file_size < 500 * 1024 * 1024 {
        512 * 1024 // 512 KB
    } else {
        // 1 MB chunks for files ≥ 500 MB.
        // 1 MB-aligned offsets (0, 1 MB, 2 MB, …) never cross a 1 MB boundary,
        // satisfying Telegram's offset+limit constraint.  This halves RPC round-
        1024 * 1024 // 1 MB
    }
}

/// Hard per-file worker ceiling.
///
/// Never exceed 4 concurrent workers for a single upload or download regardless
/// of file size.  Exceeding this causes the server to shed connections with
/// early-EOF, triggering reconnects.
pub const MAX_WORKERS_PER_FILE: usize = 4;

/// Hard global MTProto sender ceiling across all concurrent transfers.
///
/// Prefer 8 for normal usage; 12 is the absolute burst ceiling.  Enforced via
/// [`ClientInner::worker_semaphore`] which is initialised with this many permits.
pub const MAX_GLOBAL_SENDERS: usize = 12;

/// Files larger than this use `upload.saveBigFilePart`  - Telegram protocol spec.
/// MUST be 10 MB, not 30 MB.
pub const BIG_FILE_THRESHOLD: usize = 10 * 1024 * 1024;

/// Maximum parts per upload.
#[allow(dead_code)]
const UPLOAD_MAX_PARTS: i32 = 4000;

/// Maximum bytes in-flight per upload session  -
/// `kMaxUploadPerSession = 1 MB`.
#[allow(dead_code)]
const UPLOAD_MAX_PER_SESSION: usize = 1024 * 1024;

/// Upload part sizes tried in order  -
/// `kDocumentUploadPartSize{0..4}`.
#[allow(dead_code)]
const UPLOAD_PART_SIZES: &[usize] = &[32 * 1024, 64 * 1024, 128 * 1024, 256 * 1024, 512 * 1024];

/// Choose upload part size for `file_size` bytes.
///
/// Upload part size table:
/// - < 1 MB  → 32 KB  (fits in ≤ 32 parts)
/// - 1–32 MB → 64 KB
/// - 32–512 MB → 128 KB
/// - 512 MB–1 GB → 256 KB
/// - > 1 GB  → 512 KB
///
/// Returns `(part_size_bytes, total_parts)`.
pub fn upload_part_size(file_size: usize) -> (usize, i32) {
    // Enforce Telegram's hard 4000-part limit.
    // For files beyond ~1.95 GB (ceil(1.95 GB / 512 KB) > 4000), grow part size
    // so total_parts stays ≤ 4000; round up to 512-byte boundary (protocol requirement).
    const MAX_PARTS: usize = 4000;
    let mut ps: usize = if file_size < 512 * 1024 {
        32 * 1024
    } else {
        512 * 1024
    };
    if file_size.div_ceil(ps) > MAX_PARTS {
        ps = file_size.div_ceil(MAX_PARTS);
        ps = ps.div_ceil(512); // round up to 512-byte boundary
    }
    (ps, file_size.div_ceil(ps) as i32)
}

/// Internal helper: part-count → worker count, hard-capped at [`MAX_WORKERS_PER_FILE`].
/// Prefer the file-size-aware `download_worker_count` / `upload_worker_count`
/// for new call sites.
#[allow(dead_code)]
pub(crate) fn count_workers(n_parts: usize) -> usize {
    match n_parts {
        0..=5 => 1,
        6..=20 => 2,
        21..=80 => 3,
        _ => MAX_WORKERS_PER_FILE, // hard ceiling: 4
    }
}

/// Concurrent download workers for `file_size` bytes.
///
/// Hard ceiling: [`MAX_WORKERS_PER_FILE`] = 4.
///
/// | File size    | Workers |
/// |--------------|---------|
/// | < 10 MB      | 1       |
/// | 10 – 50 MB   | 2       |
/// | 50 – 300 MB  | 3       |
/// | > 300 MB     | 4       |
///
/// The 300 MB boundary avoids the 199 MB → 3 / 202 MB → 4 cliff that a
/// 200 MB cutoff would create  - files cluster around round sizes.
pub fn download_worker_count(file_size: usize) -> usize {
    if file_size < 10 * 1024 * 1024 {
        1
    } else if file_size < 50 * 1024 * 1024 {
        2
    } else if file_size < 300 * 1024 * 1024 {
        3
    } else {
        MAX_WORKERS_PER_FILE // 4
    }
}

/// Concurrent upload workers for `file_size` bytes.
///
/// Hard ceiling: [`MAX_WORKERS_PER_FILE`] = 4.
///
/// | File size    | Workers |
/// |--------------|---------|
/// | < 10 MB      | 1       |
/// | 10 – 100 MB  | 2       |
/// | 100 – 500 MB | 3       |
/// | > 500 MB     | 4       |
pub fn upload_worker_count(file_size: usize) -> usize {
    if file_size < 10 * 1024 * 1024 {
        1
    } else if file_size < 100 * 1024 * 1024 {
        2
    } else if file_size < 500 * 1024 * 1024 {
        3
    } else {
        MAX_WORKERS_PER_FILE // 4
    }
}

// Kept for backwards compat; upload chunk size is now dynamic  - use `upload_part_size(file_size)`.
#[deprecated(note = "use upload_part_size(file_size).0")]
pub const UPLOAD_CHUNK_SIZE: i32 = 128 * 1024;

/// Return `mime_type` as-is if it is non-empty and not the generic fallback,
/// otherwise infer from `name`'s extension via `mime_guess`.
fn resolve_mime(name: &str, mime_type: &str) -> String {
    if !mime_type.is_empty() && mime_type != "application/octet-stream" {
        return mime_type.to_string();
    }
    mime_guess::from_path(name)
        .first_or_octet_stream()
        .to_string()
}
/// A successfully uploaded file handle, ready to be sent as media.
#[derive(Debug, Clone)]
pub struct UploadedFile {
    pub(crate) inner: tl::enums::InputFile,
    pub(crate) mime_type: String,
    pub(crate) name: String,
}

impl UploadedFile {
    pub fn mime_type(&self) -> &str {
        &self.mime_type
    }
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Wrap as `InputMedia` for sending as a document.
    pub fn as_document_media(&self) -> tl::enums::InputMedia {
        tl::enums::InputMedia::UploadedDocument(tl::types::InputMediaUploadedDocument {
            nosound_video: false,
            force_file: false,
            spoiler: false,
            file: self.inner.clone(),
            thumb: None,
            mime_type: self.mime_type.clone(),
            attributes: vec![tl::enums::DocumentAttribute::Filename(
                tl::types::DocumentAttributeFilename {
                    file_name: self.name.clone(),
                },
            )],
            stickers: None,
            ttl_seconds: None,
            video_cover: None,
            video_timestamp: None,
        })
    }

    /// Wrap as `InputMedia` for sending as a photo.
    pub fn as_photo_media(&self) -> tl::enums::InputMedia {
        tl::enums::InputMedia::UploadedPhoto(tl::types::InputMediaUploadedPhoto {
            spoiler: false,
            live_photo: false,
            file: self.inner.clone(),
            stickers: None,
            ttl_seconds: None,
            video: None,
        })
    }
}

// Downloadable trait

/// Something that can be downloaded via [`Client::iter_download`].
///
/// Implemented by [`Photo`], [`Document`], and [`Sticker`].
pub trait Downloadable {
    /// Return the `InputFileLocation` needed for `upload.getFile`.
    fn to_input_location(&self) -> Option<tl::enums::InputFileLocation>;

    /// DC that stores this file. `upload.getFile` MUST be routed here.
    /// Sending to the wrong DC causes AuthKeyMismatch on the main connection.
    fn dc_id(&self) -> i32;

    /// File size in bytes, if known (used to choose the concurrent path).
    fn size(&self) -> Option<usize> {
        None
    }
}

// Typed media wrappers

/// Typed wrapper over a Telegram photo.
#[derive(Debug, Clone)]
pub struct Photo {
    pub raw: tl::types::Photo,
}

impl Photo {
    pub fn from_raw(raw: tl::types::Photo) -> Self {
        Self { raw }
    }

    /// Try to extract from a `MessageMedia` variant.
    pub fn from_media(media: &tl::enums::MessageMedia) -> Option<Self> {
        if let tl::enums::MessageMedia::Photo(mp) = media
            && let Some(tl::enums::Photo::Photo(p)) = &mp.photo
        {
            return Some(Self { raw: p.clone() });
        }
        None
    }

    pub fn id(&self) -> i64 {
        self.raw.id
    }
    pub fn access_hash(&self) -> i64 {
        self.raw.access_hash
    }
    pub fn date(&self) -> i32 {
        self.raw.date
    }
    pub fn has_stickers(&self) -> bool {
        self.raw.has_stickers
    }

    /// The largest available thumb type letter (e.g. `"s"`, `"m"`, `"x"`).
    pub fn largest_thumb_type(&self) -> &str {
        self.raw
            .sizes
            .iter()
            .filter_map(|s| match s {
                tl::enums::PhotoSize::PhotoSize(ps) => Some(ps.r#type.as_str()),
                _ => None,
            })
            .next_back()
            .unwrap_or("s")
    }
}

impl Downloadable for Photo {
    fn to_input_location(&self) -> Option<tl::enums::InputFileLocation> {
        Some(tl::enums::InputFileLocation::InputPhotoFileLocation(
            tl::types::InputPhotoFileLocation {
                id: self.raw.id,
                access_hash: self.raw.access_hash,
                file_reference: self.raw.file_reference.clone(),
                thumb_size: self.largest_thumb_type().to_string(),
            },
        ))
    }
    fn dc_id(&self) -> i32 {
        self.raw.dc_id
    }
}

/// Typed wrapper over a Telegram document (file, video, audio).
#[derive(Debug, Clone)]
pub struct Document {
    pub raw: tl::types::Document,
}

impl Document {
    pub fn from_raw(raw: tl::types::Document) -> Self {
        Self { raw }
    }

    /// Try to extract from a `MessageMedia` variant.
    pub fn from_media(media: &tl::enums::MessageMedia) -> Option<Self> {
        if let tl::enums::MessageMedia::Document(md) = media
            && let Some(tl::enums::Document::Document(d)) = &md.document
        {
            return Some(Self { raw: d.clone() });
        }
        None
    }

    pub fn id(&self) -> i64 {
        self.raw.id
    }
    pub fn access_hash(&self) -> i64 {
        self.raw.access_hash
    }
    pub fn date(&self) -> i32 {
        self.raw.date
    }
    pub fn mime_type(&self) -> &str {
        &self.raw.mime_type
    }
    pub fn size(&self) -> i64 {
        self.raw.size
    }

    /// File name from document attributes, if present.
    pub fn file_name(&self) -> Option<&str> {
        self.raw.attributes.iter().find_map(|a| match a {
            tl::enums::DocumentAttribute::Filename(f) => Some(f.file_name.as_str()),
            _ => None,
        })
    }

    /// `true` if the document has animated sticker attributes.
    pub fn is_animated(&self) -> bool {
        self.raw
            .attributes
            .iter()
            .any(|a| matches!(a, tl::enums::DocumentAttribute::Animated))
    }
}

impl Downloadable for Document {
    fn to_input_location(&self) -> Option<tl::enums::InputFileLocation> {
        Some(tl::enums::InputFileLocation::InputDocumentFileLocation(
            tl::types::InputDocumentFileLocation {
                id: self.raw.id,
                access_hash: self.raw.access_hash,
                file_reference: self.raw.file_reference.clone(),
                thumb_size: String::new(),
            },
        ))
    }
    fn dc_id(&self) -> i32 {
        self.raw.dc_id
    }
    fn size(&self) -> Option<usize> {
        Some(self.raw.size as usize)
    }
}

/// Typed wrapper over a Telegram sticker.
#[derive(Debug, Clone)]
pub struct Sticker {
    pub inner: Document,
}

impl Sticker {
    /// Wrap a document that carries `DocumentAttributeSticker`.
    pub fn from_document(doc: Document) -> Option<Self> {
        let has_sticker_attr = doc
            .raw
            .attributes
            .iter()
            .any(|a| matches!(a, tl::enums::DocumentAttribute::Sticker(_)));
        if has_sticker_attr {
            Some(Self { inner: doc })
        } else {
            None
        }
    }

    /// Try to extract directly from `MessageMedia`.
    pub fn from_media(media: &tl::enums::MessageMedia) -> Option<Self> {
        Document::from_media(media).and_then(Self::from_document)
    }

    /// The emoji associated with the sticker.
    pub fn emoji(&self) -> Option<&str> {
        self.inner.raw.attributes.iter().find_map(|a| match a {
            tl::enums::DocumentAttribute::Sticker(s) => Some(s.alt.as_str()),
            _ => None,
        })
    }

    /// `true` if this is a video sticker.
    pub fn is_video(&self) -> bool {
        self.inner
            .raw
            .attributes
            .iter()
            .any(|a| matches!(a, tl::enums::DocumentAttribute::Video(_)))
    }

    pub fn id(&self) -> i64 {
        self.inner.id()
    }
    pub fn mime_type(&self) -> &str {
        self.inner.mime_type()
    }
}

impl Downloadable for Sticker {
    fn to_input_location(&self) -> Option<tl::enums::InputFileLocation> {
        self.inner.to_input_location()
    }
    fn dc_id(&self) -> i32 {
        self.inner.dc_id()
    }
    fn size(&self) -> Option<usize> {
        Some(self.inner.raw.size as usize)
    }
}

// DownloadIter

/// Sequential chunk-by-chunk download iterator.
pub struct DownloadIter {
    client: Client,
    request: Option<tl::functions::upload::GetFile>,
    done: bool,
    /// DC that hosts the file  - GetFile is routed here via invoke_on_dc.
    dc_id: i32,
}

impl DownloadIter {
    /// Set a custom chunk size (must be multiple of 4096, max 524288).
    pub fn chunk_size(mut self, size: i32) -> Self {
        if let Some(r) = &mut self.request {
            r.limit = size;
        }
        self
    }

    /// Fetch the next chunk. Returns `None` when the download is complete.
    pub async fn next(&mut self) -> Result<Option<Vec<u8>>, InvocationError> {
        if self.done {
            return Ok(None);
        }
        let req = match &self.request {
            Some(r) => r.clone(),
            None => return Ok(None),
        };
        // Route to the file's dedicated transfer connection, isolated from the main session.
        // Using rpc_on_dc_raw_pub (main session) caused Crypto(InvalidBuffer) on reconnects
        // because file traffic contaminated the main session's seq_no/msg_id state.
        let body = self.client.rpc_transfer_on_dc_pub(self.dc_id, &req).await?;
        let mut cur = Cursor::from_slice(&body);
        match tl::enums::upload::File::deserialize(&mut cur)? {
            tl::enums::upload::File::File(f) => {
                if (f.bytes.len() as i32) < req.limit {
                    self.done = true;
                    if f.bytes.is_empty() {
                        return Ok(None);
                    }
                }
                if let Some(r) = &mut self.request {
                    r.offset += req.limit as i64;
                }
                Ok(Some(f.bytes))
            }
            // CDN redirect: the server wants us to download from a CDN DC.
            // cdn_supported=false means Telegram should not send this, but some
            // DCs still do. Treat it as a retriable failure so the caller can
            // fall back (e.g. switch cdn_supported=true and use CdnDownloader).
            tl::enums::upload::File::CdnRedirect(_) => {
                self.done = true;
                Err(InvocationError::Deserialize(
                    "upload.fileCdnRedirect received (cdn_supported=false was ignored by server)"
                        .into(),
                ))
            }
        }
    }
}

impl Client {
    // Upload

    /// Upload bytes sequentially (single session).
    ///
    /// Part size and big-file threshold:
    /// - Part size chosen by [`upload_part_size`]:
    ///   < 1 MB → 32 KB, 1–32 MB → 64 KB, 32–512 MB → 128 KB, etc.
    /// - `upload.saveBigFilePart` used for files > 30 MB (`kUseBigFilesFrom`).
    ///
    /// For files that benefit from parallelism use [`upload_file_concurrent`].
    pub async fn upload_file(
        &self,
        data: &[u8],
        name: &str,
        mime_type: &str,
    ) -> Result<UploadedFile, InvocationError> {
        // Zero-byte upload produces parts=0; add 1 to satisfy FILE_PART_0_MISSING check.
        if data.is_empty() {
            return Err(InvocationError::Deserialize(
                "cannot upload empty file".into(),
            ));
        }
        let resolved_mime = resolve_mime(name, mime_type);
        let total = data.len();
        let big = total > BIG_FILE_THRESHOLD;
        // Pick smallest part size that keeps total_parts <= 4000.
        let (part_size, total_parts) = upload_part_size(total);
        let file_id = crate::random_i64_pub();

        for (part_num, chunk) in data.chunks(part_size).enumerate() {
            if big {
                // Always through transfer pool, never main session.
                self.rpc_transfer_on_dc_pub(
                    0,
                    &tl::functions::upload::SaveBigFilePart {
                        file_id,
                        file_part: part_num as i32,
                        file_total_parts: total_parts,
                        bytes: chunk.to_vec(),
                    },
                )
                .await?;
            } else {
                self.rpc_transfer_on_dc_pub(
                    0,
                    &tl::functions::upload::SaveFilePart {
                        file_id,
                        file_part: part_num as i32,
                        bytes: chunk.to_vec(),
                    },
                )
                .await?;
            }
        }

        let inner = make_input_file(big, file_id, total_parts, name, data);
        tracing::info!(
            "[ferogram] uploaded '{}' ({} bytes, part={}B × {} parts, mime={})",
            name,
            total,
            part_size,
            total_parts,
            resolved_mime
        );
        Ok(UploadedFile {
            inner,
            mime_type: resolved_mime,
            name: name.to_string(),
        })
    }

    /// Upload bytes with parallel worker sessions.
    ///
    /// Parallel upload using per-worker connections. Worker count scales with file size.
    /// Part size: 32 KB for tiny files, 512 KB otherwise.
    ///
    /// - Files < 10 MB  -> `upload.saveFilePart`    (small-file API)
    /// - Files >= 10 MB -> `upload.saveBigFilePart`  (big-file API)
    pub async fn upload_file_concurrent(
        &self,
        data: Arc<Vec<u8>>,
        name: &str,
        mime_type: &str,
    ) -> Result<UploadedFile, InvocationError> {
        // Zero-byte upload produces parts=0; add 1 to satisfy FILE_PART_0_MISSING check.
        if data.is_empty() {
            return Err(InvocationError::Deserialize(
                "cannot upload empty file".into(),
            ));
        }
        let total = data.len();
        let (part_size, total_parts) = upload_part_size(total);
        let big = total > BIG_FILE_THRESHOLD;
        // Per-file hard ceiling: max 4 workers. Global ceiling: MAX_GLOBAL_SENDERS permits.
        let n_workers = upload_worker_count(total).min(MAX_WORKERS_PER_FILE);
        let _global_guard = self
            .inner
            .worker_semaphore
            .acquire_many(n_workers as u32)
            .await
            .expect("worker semaphore unexpectedly closed");

        // file_id is shared across workers.
        // On FILE_MIGRATE the first worker that detects the migration stores a
        // NEW file_id here so all workers restart from part 0 with a fresh id
        // on the new DC. Reusing the old id on a different DC causes the server
        // to return FILE_ID_INVALID / MEDIA_EMPTY at sendMedia time.
        let file_id_atomic =
            std::sync::Arc::new(std::sync::atomic::AtomicI64::new(crate::random_i64_pub()));

        // FILE_MIGRATE is a per-file-id directive  - ALL workers
        // must redirect to the same new DC, not just the one that received the error.
        // Share the current upload DC as an atomic so any migrating worker updates
        // it and all others follow on their next iteration.  0 = home DC (sentinel).
        let upload_dc = Arc::new(AtomicI32::new(0i32));

        // Open all worker connections concurrently.
        let mut open_set: tokio::task::JoinSet<
            Result<crate::dc_pool::DcConnection, InvocationError>,
        > = tokio::task::JoinSet::new();
        for _ in 0..n_workers {
            let client = self.clone();
            open_set.spawn(async move { client.open_worker_conn(0).await });
        }
        let mut conns: Vec<crate::dc_pool::DcConnection> = Vec::with_capacity(n_workers);
        while let Some(res) = open_set.join_next().await {
            match res {
                Ok(Ok(c)) => conns.push(c),
                Ok(Err(e)) => tracing::warn!("[ferogram] upload: worker conn failed: {e}"),
                Err(e) => tracing::warn!("[ferogram] upload: worker conn join error: {e}"),
            }
        }
        if conns.is_empty() {
            tracing::warn!("[ferogram] upload: no worker conns, falling back to sequential");
            return self.upload_file(&data, name, mime_type).await;
        }
        let actual_workers = conns.len();

        let next_part = Arc::new(Mutex::new(0i32));
        let mut tasks: tokio::task::JoinSet<Result<(), InvocationError>> =
            tokio::task::JoinSet::new();

        for mut conn in conns {
            let data = Arc::clone(&data);
            let next_part = Arc::clone(&next_part);
            let client = self.clone();
            let upload_dc = Arc::clone(&upload_dc);
            let file_id_atomic = std::sync::Arc::clone(&file_id_atomic);

            tasks.spawn(async move {
                // Reconnect budget is per-worker lifetime.
                const MAX_WORKER_RECONNECTS: u8 = 5;
                let mut total_reconnects = 0u8;
                // Mutable: FILE_MIGRATE (303) can redirect uploads to another DC.
                // worker_dc stays in sync with the shared upload_dc atomic.
                let mut worker_dc = 0i32; // 0 = home DC

                loop {
                    // Read file_id, dc, and part_num under one next_part lock so
                    // FILE_MIGRATE's triple-update is atomic with respect to all workers.
                    let (part_num, file_id, current_dc) = {
                        let mut g = next_part.lock().await;
                        let fid = file_id_atomic.load(std::sync::atomic::Ordering::Relaxed);
                        let dc = upload_dc.load(Ordering::Relaxed);
                        if *g >= total_parts {
                            break;
                        }
                        let n = *g;
                        *g += 1;
                        (n, fid, dc)
                    };
                    if current_dc != worker_dc {
                        worker_dc = current_dc;
                        conn = match client.open_worker_conn(worker_dc).await {
                            Ok(c) => c,
                            Err(e) => return Err(e),
                        };
                    }
                    let start = part_num as usize * part_size;
                    let end = (start + part_size).min(data.len());
                    let bytes = data[start..end].to_vec();

                    // Error handling:
                    //   FLOOD_WAIT (420)          → sleep, retry same conn
                    //   FILE_MIGRATE (303)        → switch worker to redirected DC
                    //   AUTH_KEY_UNREGISTERED     → reopen worker (fresh DH + importAuth)
                    //   Server Timeout (-503) / IO → reconnect with exponential backoff
                    //   Any other RPC error       → propagate immediately
                    loop {
                        let result = if big {
                            conn.rpc_call(&tl::functions::upload::SaveBigFilePart {
                                file_id,
                                file_part: part_num,
                                file_total_parts: total_parts,
                                bytes: bytes.clone(),
                            })
                            .await
                        } else {
                            conn.rpc_call(&tl::functions::upload::SaveFilePart {
                                file_id,
                                file_part: part_num,
                                bytes: bytes.clone(),
                            })
                            .await
                        };
                        let err = match result {
                            Ok(_) => break,
                            Err(e) => e,
                        };
                        if let InvocationError::Rpc(ref rpc) = err {
                            // FLOOD_WAIT: sleep, retry same conn.
                            if rpc.code == 420 {
                                let secs = rpc.value.unwrap_or(1) as u64;
                                tracing::info!(
                                    "[ferogram] upload: FLOOD_WAIT_{secs}; sleeping before retry"
                                );
                                tokio::time::sleep(std::time::Duration::from_secs(secs)).await;
                                continue;
                            }
                            // FILE_MIGRATE: server redirected upload to a different DC.
                            if rpc.code == 303 {
                                let new_dc = rpc.value.unwrap_or(1) as i32;
                                tracing::info!(
                                    "[ferogram] upload: FILE_MIGRATE_{new_dc}; \
                                     switching worker DC{worker_dc}→DC{new_dc}"
                                );
                                // On FILE_MIGRATE, use a NEW file_id on
                                // the new DC. The original DC's partial upload is abandoned.
                                // Reusing the old file_id on a different DC causes the server
                                // to return FILE_ID_INVALID / MEDIA_EMPTY at sendMedia time
                                // because the new DC has no record of that file_id.
                                // Signal the outer task to restart with a fresh file_id by
                                // returning a sentinel error; the outer loop handles it.
                                // Store new file_id BEFORE publishing DC change so
                                // other workers see the new id atomically with the DC switch.
                                // Hold next_part lock across all three stores.
                                {
                                    let mut g = next_part.lock().await;
                                    file_id_atomic.store(
                                        crate::random_i64_pub(),
                                        std::sync::atomic::Ordering::SeqCst,
                                    );
                                    upload_dc.store(new_dc, Ordering::SeqCst);
                                    *g = 0;
                                }
                                worker_dc = new_dc;
                                match client.open_worker_conn(new_dc).await {
                                    Ok(c) => {
                                        conn = c;
                                        continue;
                                    }
                                    Err(e) => return Err(e),
                                }
                            }
                            // AUTH_KEY_UNREGISTERED: reopen with fresh DH + importAuth.
                            if rpc.name == "AUTH_KEY_UNREGISTERED" {
                                tracing::warn!(
                                    "[ferogram] upload: AUTH_KEY_UNREGISTERED DC{worker_dc}; \
                                     reopening worker [{}/{MAX_WORKER_RECONNECTS}]",
                                    total_reconnects + 1
                                );
                                total_reconnects += 1;
                                if total_reconnects >= MAX_WORKER_RECONNECTS {
                                    return Err(err);
                                }
                                let backoff_ms = 300u64 * (1u64 << (total_reconnects - 1));
                                tokio::time::sleep(std::time::Duration::from_millis(backoff_ms))
                                    .await;
                                match client.open_worker_conn(worker_dc).await {
                                    Ok(c) => {
                                        conn = c;
                                        continue;
                                    }
                                    Err(e) => return Err(e),
                                }
                            }
                            // Non-retriable RPC error: propagate immediately.
                            if rpc.code != -503 {
                                return Err(err);
                            }
                        }
                        // I/O error or server-side Timeout (-503): reconnect with backoff.
                        total_reconnects += 1;
                        if total_reconnects >= MAX_WORKER_RECONNECTS {
                            return Err(err);
                        }
                        let backoff_ms = 300u64 * (1u64 << (total_reconnects - 1));
                        tracing::warn!(
                            "[ferogram] upload: worker error ({err}), reconnecting \
                             [{total_reconnects}/{MAX_WORKER_RECONNECTS}] (backoff {backoff_ms}ms)"
                        );
                        tokio::time::sleep(std::time::Duration::from_millis(backoff_ms)).await;
                        conn = match client.open_worker_conn(worker_dc).await {
                            Ok(c) => c,
                            Err(e) => return Err(e),
                        };
                    }
                }
                Ok(())
            });
        }

        while let Some(res) = tasks.join_next().await {
            if let Err(e) =
                res.map_err(|e| InvocationError::Io(std::io::Error::other(e.to_string())))?
            {
                tasks.abort_all();
                return Err(e);
            }
        }

        let file_id = file_id_atomic.load(std::sync::atomic::Ordering::Relaxed);
        let inner = make_input_file(big, file_id, total_parts, name, &data);
        tracing::info!(
            "[ferogram] uploaded '{}' ({} bytes, part={}B x {} parts, {} workers)",
            name,
            total,
            part_size,
            total_parts,
            actual_workers
        );
        Ok(UploadedFile {
            inner,
            mime_type: resolve_mime(name, mime_type),
            name: name.to_string(),
        })
    }

    /// Upload from an `AsyncRead`. Reads fully into memory then uploads.
    pub async fn upload_stream<R: AsyncRead + Unpin>(
        &self,
        reader: &mut R,
        name: &str,
        mime_type: &str,
    ) -> Result<UploadedFile, InvocationError> {
        let mut data = Vec::new();
        reader.read_to_end(&mut data).await?;
        if data.len() > BIG_FILE_THRESHOLD {
            self.upload_file_concurrent(Arc::new(data), name, mime_type)
                .await
        } else {
            self.upload_file(&data, name, mime_type).await
        }
    }

    // Send

    /// Send a file as a document or photo to a chat.
    pub async fn send_file(
        &self,
        peer: tl::enums::Peer,
        media: tl::enums::InputMedia,
        caption: &str,
    ) -> Result<(), InvocationError> {
        let input_peer = self.inner.peer_cache.read().await.peer_to_input(&peer)?;
        let req = tl::functions::messages::SendMedia {
            silent: false,
            background: false,
            clear_draft: false,
            noforwards: false,
            update_stickersets_order: false,
            invert_media: false,
            allow_paid_floodskip: false,
            peer: input_peer,
            reply_to: None,
            media,
            message: caption.to_string(),
            random_id: crate::random_i64_pub(),
            reply_markup: None,
            entities: None,
            schedule_date: None,
            schedule_repeat_period: None,
            send_as: None,
            quick_reply_shortcut: None,
            effect: None,
            allow_paid_stars: None,
            suggested_post: None,
        };
        self.rpc_call_raw_pub(&req).await?;
        Ok(())
    }

    /// Send multiple files as an album.
    ///
    /// Each [`AlbumItem`] carries its own media, caption, entities (formatting),
    /// and optional `reply_to` message ID.
    ///
    /// ```rust,no_run
    /// use ferogram::media::AlbumItem;
    ///
    /// client.send_album(peer, vec![
    /// AlbumItem::new(photo_media).caption("First photo"),
    /// AlbumItem::new(video_media).caption("Second photo").reply_to(Some(42)),
    /// ]).await?;
    ///
    /// // Shorthand: legacy tuple API still works via From impl
    /// client.send_album(peer, vec![
    /// (photo_media, "caption".to_string()).into(),
    /// ]).await?;
    /// ```
    pub async fn send_album(
        &self,
        peer: tl::enums::Peer,
        items: Vec<AlbumItem>,
    ) -> Result<(), InvocationError> {
        let input_peer = self.inner.peer_cache.read().await.peer_to_input(&peer)?;

        // Use reply_to from the first item that has one.
        let reply_to = items.iter().find_map(|i| i.reply_to).map(|id| {
            tl::enums::InputReplyTo::Message(tl::types::InputReplyToMessage {
                reply_to_msg_id: id,
                top_msg_id: None,
                reply_to_peer_id: None,
                quote_text: None,
                quote_entities: None,
                quote_offset: None,
                monoforum_peer_id: None,
                poll_option: None,
                todo_item_id: None,
            })
        });

        let multi: Vec<tl::enums::InputSingleMedia> = items
            .into_iter()
            .map(|item| {
                tl::enums::InputSingleMedia::InputSingleMedia(tl::types::InputSingleMedia {
                    media: item.media,
                    random_id: crate::random_i64_pub(),
                    message: item.caption,
                    entities: if item.entities.is_empty() {
                        None
                    } else {
                        Some(item.entities)
                    },
                })
            })
            .collect();

        let req = tl::functions::messages::SendMultiMedia {
            silent: false,
            background: false,
            clear_draft: false,
            noforwards: false,
            update_stickersets_order: false,
            invert_media: false,
            allow_paid_floodskip: false,
            peer: input_peer,
            reply_to,
            multi_media: multi,
            schedule_date: None,
            send_as: None,
            quick_reply_shortcut: None,
            effect: None,
            allow_paid_stars: None,
        };
        self.rpc_call_raw_pub(&req).await?;
        Ok(())
    }

    // Download

    /// Create a sequential chunk download iterator.
    ///
    /// `dc_id` must be the DC that stores the file (`Document::dc_id()` /
    /// `Photo::dc_id()`). Pass `0` to use the home DC (bots only).
    pub fn iter_download(&self, location: tl::enums::InputFileLocation) -> DownloadIter {
        self.iter_download_on_dc(location, 0)
    }

    /// Like [`iter_download`] but routes to a specific DC.
    pub fn iter_download_on_dc(
        &self,
        location: tl::enums::InputFileLocation,
        dc_id: i32,
    ) -> DownloadIter {
        // 512 KB chunks: offset advances by `limit` each step, so offsets are
        // 0, 512 KB, 1 MB, 1.5 MB …  - always 512 KB-aligned and within the same
        // 1 MB block (Telegram's offset+limit rule).  Caller can override with
        // `.chunk_size()` if needed.
        DownloadIter {
            client: self.clone(),
            done: false,
            dc_id,
            request: Some(tl::functions::upload::GetFile {
                precise: false,
                cdn_supported: false,
                location,
                offset: 0,
                limit: 512 * 1024,
            }),
        }
    }

    /// Download all bytes of a media attachment at once (sequential).
    pub async fn download_media(
        &self,
        location: tl::enums::InputFileLocation,
    ) -> Result<Vec<u8>, InvocationError> {
        self.download_media_on_dc(location, 0).await
    }

    /// Like [`download_media`] but routes `GetFile` to `dc_id`.
    ///
    /// Opens a **dedicated** `DcConnection` for this download so it never
    /// shares the idle transfer-pool connection (which the server silently
    /// closes after ~90 s of inactivity, causing early-eof on the next use).
    ///
    /// Full AUTH_KEY_UNREGISTERED + FILE_MIGRATE recovery,
    /// the resilience of the concurrent worker path.
    pub async fn download_media_on_dc(
        &self,
        location: tl::enums::InputFileLocation,
        dc_id: i32,
    ) -> Result<Vec<u8>, InvocationError> {
        // Use the dynamic chunk size with a conservative 0-byte probe (yields
        // 256 KB); the loop will switch to 512 KB once we have accumulated
        // enough to know the file is large, but for the sequential path the
        // uniform 512 KB is safe and simpler.
        let chunk = 512 * 1024i32;
        let mut worker_dc = if dc_id == 0 {
            *self.inner.home_dc_id.lock().await
        } else {
            dc_id
        };
        let mut conn = self.open_worker_conn(worker_dc).await?;
        let mut offset = 0i64;
        let mut bytes = Vec::new();
        // Per-chunk retry budget for transient errors.
        let mut reopen_attempts = 0u8;
        const MAX_REOPEN: u8 = 3;

        loop {
            let req = tl::functions::upload::GetFile {
                precise: true,
                cdn_supported: false,
                location: location.clone(),
                offset,
                limit: chunk,
            };
            match conn.rpc_call(&req).await {
                Ok(raw) => {
                    let mut cur = Cursor::from_slice(&raw);
                    match tl::enums::upload::File::deserialize(&mut cur)? {
                        tl::enums::upload::File::File(f) => {
                            reopen_attempts = 0; // successful chunk  - reset counter
                            let done = (f.bytes.len() as i32) < chunk;
                            bytes.extend_from_slice(&f.bytes);
                            if done {
                                break;
                            }
                            offset += chunk as i64;
                        }
                        tl::enums::upload::File::CdnRedirect(_) => break,
                    }
                }
                Err(InvocationError::Rpc(ref rpc))
                    if rpc.name == "FILE_MIGRATE" || rpc.name == "FILE_MIGRATE_X" =>
                {
                    // FILE_MIGRATE: file lives on a different DC.
                    let new_dc = rpc.value.unwrap_or(0) as i32;
                    if new_dc == 0 || new_dc == worker_dc {
                        return Err(InvocationError::Rpc(rpc.clone()));
                    }
                    tracing::debug!(
                        "[ferogram] seq download: FILE_MIGRATE_{new_dc}; reopening worker on DC{new_dc}"
                    );
                    worker_dc = new_dc;
                    conn = self.open_worker_conn(worker_dc).await?;
                    // Retry same offset on new DC  - do not advance offset.
                }
                Err(InvocationError::Rpc(ref rpc)) if rpc.name == "AUTH_KEY_UNREGISTERED" => {
                    // AUTH_KEY_UNREGISTERED: reopen connection with fresh DH.
                    reopen_attempts += 1;
                    if reopen_attempts > MAX_REOPEN {
                        return Err(InvocationError::Rpc(rpc.clone()));
                    }
                    tracing::debug!(
                        "[ferogram] seq download: AUTH_KEY_UNREGISTERED DC{worker_dc}; \
                         reopening worker [{reopen_attempts}/{MAX_REOPEN}]"
                    );
                    // Evict the cached foreign key so open_worker_conn does a
                    // fresh DH + import instead of reusing the dead key again.
                    {
                        let mut opts = self.inner.dc_options.lock().await;
                        if let Some(e) = opts.get_mut(&worker_dc) {
                            e.auth_key = None;
                        }
                    }
                    conn = self.open_worker_conn(worker_dc).await?;
                    // Retry same offset with the fresh connection.
                }
                Err(e) => return Err(e),
            }
        }
        Ok(bytes)
    }

    /// Download a file using parallel sessions.
    ///
    /// `size` must be the exact byte size of the file.
    ///
    /// Returns the full file bytes in order.
    pub async fn download_media_concurrent(
        &self,
        location: tl::enums::InputFileLocation,
        size: usize,
    ) -> Result<Vec<u8>, InvocationError> {
        self.download_media_concurrent_on_dc(location, size, 0)
            .await
    }

    /// Like [`download_media_concurrent`] but routes `GetFile` to `dc_id`.
    ///
    /// Parallel download using per-worker connections. Worker count scales with file size.
    pub async fn download_media_concurrent_on_dc(
        &self,
        location: tl::enums::InputFileLocation,
        size: usize,
        dc_id: i32,
    ) -> Result<Vec<u8>, InvocationError> {
        let chunk = download_chunk_size(size) as usize; // 256 KB (<50 MB) or 512 KB (≥50 MB)
        let n_parts = size.div_ceil(chunk);
        // Per-file hard ceiling: MAX_WORKERS_PER_FILE = 4.
        // Global ceiling: MAX_GLOBAL_SENDERS = 12 (enforced via semaphore).
        let n_workers = download_worker_count(size).min(MAX_WORKERS_PER_FILE);
        let _global_guard = self
            .inner
            .worker_semaphore
            .acquire_many(n_workers as u32)
            .await
            .expect("worker semaphore unexpectedly closed");

        // For small files that only need 1 worker, opening a fresh parallel
        // connection (with its own initConnection + bad_server_salt round-trip)
        // adds ~400 ms of unnecessary overhead. Fall through to the sequential
        // path which is equivalent for a single part.
        let home = *self.inner.home_dc_id.lock().await;
        let effective_dc = if dc_id == 0 { home } else { dc_id };
        if n_workers == 1 && effective_dc == home {
            return self.download_media_on_dc(location, dc_id).await;
        }

        // Open all worker connections CONCURRENTLY so they are all ready at the
        // same time. Sequential opening takes N × ~0.7 s (one bad_server_salt
        // round-trip per connection), leaving early connections idle while later
        // ones set up  - which causes the server to close idle connections before
        // the download even starts.
        let mut open_set: tokio::task::JoinSet<
            Result<crate::dc_pool::DcConnection, InvocationError>,
        > = tokio::task::JoinSet::new();
        for _ in 0..n_workers {
            let client = self.clone();
            open_set.spawn(async move { client.open_worker_conn(dc_id).await });
        }
        let mut conns: Vec<crate::dc_pool::DcConnection> = Vec::with_capacity(n_workers);
        while let Some(res) = open_set.join_next().await {
            match res {
                Ok(Ok(c)) => conns.push(c),
                Ok(Err(e)) => tracing::warn!("[ferogram] download: worker conn failed: {e}"),
                Err(e) => tracing::warn!("[ferogram] download: worker conn join error: {e}"),
            }
        }
        if conns.is_empty() {
            tracing::warn!("[ferogram] download: no worker conns, falling back to sequential");
            return self.download_media_on_dc(location, dc_id).await;
        }

        let next_part = Arc::new(Mutex::new(0usize));
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<(usize, Vec<u8>)>();
        let mut tasks: tokio::task::JoinSet<Result<(), InvocationError>> =
            tokio::task::JoinSet::new();
        // Shared abort flag so a failing worker signals remaining workers
        // to exit instead of exhausting their full reconnect budget independently.
        // Without this, tasks.abort_all() fires after rx.recv() drains (i.e. after
        // ALL workers have already exited), making the abort structurally dead.
        let abort = Arc::new(AtomicBool::new(false));

        for mut conn in conns {
            let location = location.clone();
            let next_part = Arc::clone(&next_part);
            let tx = tx.clone();
            let client = self.clone();
            let abort = Arc::clone(&abort);
            // Capture the resolved DC (effective_dc), not the raw dc_id which
            // may be 0 (sentinel for "home DC").  open_worker_conn(0) would correctly
            // resolve to home DC, but after a FILE_MIGRATE the worker updates worker_dc
            // to the new DC; if a reconnect then occurs before FILE_MIGRATE the sentinel
            // is replaced by the concrete DC ID, avoiding ambiguity on every reconnect.
            let init_dc = effective_dc;

            tasks.spawn(async move {
                // Reconnect budget is per-worker lifetime, NOT per-chunk.
                const MAX_WORKER_RECONNECTS: u8 = 5;
                let mut total_reconnects = 0u8;
                // Mutable: FILE_MIGRATE (303) may redirect to a different DC mid-transfer.
                let mut worker_dc = init_dc;

                loop {
                    // Check abort flag before starting each part.
                    if abort.load(Ordering::Relaxed) {
                        break;
                    }
                    let part = {
                        let mut g = next_part.lock().await;
                        if *g >= n_parts {
                            break;
                        }
                        let p = *g;
                        *g += 1;
                        p
                    };
                    let req = tl::functions::upload::GetFile {
                        precise: true,
                        cdn_supported: false,
                        location: location.clone(),
                        offset: (part * chunk) as i64, // chunk-aligned  - safe with 512 KB
                        limit: chunk as i32,           // matches offset stride exactly
                    };
                    // Error handling:
                    //   FLOOD_WAIT (420)          → sleep, retry same conn
                    //   FILE_MIGRATE (303)        → switch worker to redirected DC, retry same part
                    //   AUTH_KEY_UNREGISTERED     → reopen worker (fresh DH + importAuth), retry
                    //   Server Timeout (-503) / IO → reconnect with exponential backoff
                    //   Any other RPC error       → propagate immediately
                    let raw = loop {
                        let err = match conn.rpc_call(&req).await {
                            Ok(r) => break r,
                            Err(e) => e,
                        };
                        if let InvocationError::Rpc(ref rpc) = err {
                            if rpc.code == 420 {
                                let secs = rpc.value.unwrap_or(1) as u64;
                                tracing::info!(
                                    "[ferogram] download: FLOOD_WAIT_{secs}; sleeping before retry"
                                );
                                if abort.load(Ordering::Relaxed) {
                                    abort.store(true, Ordering::Relaxed);
                                    return Err(err);
                                }
                                tokio::time::sleep(std::time::Duration::from_secs(secs)).await;
                                continue; // retry on same connection  - no reconnect
                            }
                            // FILE_MIGRATE: file is hosted on a different DC.
                            // Open a fresh worker on the new DC and retry the same part.
                            // Does not count against the reconnect budget.
                            if rpc.code == 303 {
                                let new_dc = rpc.value.unwrap_or(1) as i32;
                                tracing::info!(
                                    "[ferogram] download: FILE_MIGRATE_{new_dc}; \
                                     switching worker DC{worker_dc}→DC{new_dc}"
                                );
                                worker_dc = new_dc;
                                match client.open_worker_conn(new_dc).await {
                                    Ok(c) => {
                                        conn = c;
                                        continue;
                                    }
                                    Err(e) => {
                                        abort.store(true, Ordering::Relaxed);
                                        return Err(e);
                                    }
                                }
                            }
                            // AUTH_KEY_UNREGISTERED: the server invalidated this worker's key.
                            // open_worker_conn for foreign DCs does fresh DH + importAuth,
                            // which creates a new registered key. Counts against reconnect budget.
                            if rpc.name == "AUTH_KEY_UNREGISTERED" {
                                tracing::warn!(
                                    "[ferogram] download: AUTH_KEY_UNREGISTERED DC{worker_dc}; \
                                     reopening worker [{}/{MAX_WORKER_RECONNECTS}]",
                                    total_reconnects + 1
                                );
                                total_reconnects += 1;
                                if total_reconnects >= MAX_WORKER_RECONNECTS {
                                    abort.store(true, Ordering::Relaxed);
                                    return Err(err);
                                }
                                let backoff_ms = 300u64 * (1u64 << (total_reconnects - 1));
                                if abort.load(Ordering::Relaxed) {
                                    abort.store(true, Ordering::Relaxed);
                                    return Err(err);
                                }
                                tokio::time::sleep(std::time::Duration::from_millis(backoff_ms))
                                    .await;
                                match client.open_worker_conn(worker_dc).await {
                                    Ok(c) => {
                                        conn = c;
                                        continue;
                                    }
                                    Err(e) => {
                                        abort.store(true, Ordering::Relaxed);
                                        return Err(e);
                                    }
                                }
                            }
                            if rpc.code != -503 {
                                abort.store(true, Ordering::Relaxed);
                                return Err(err); // non-retriable RPC error
                            }
                        }
                        // I/O error or server-side Timeout (-503): reconnect with backoff.
                        total_reconnects += 1;
                        if total_reconnects >= MAX_WORKER_RECONNECTS {
                            abort.store(true, Ordering::Relaxed);
                            return Err(err);
                        }
                        // Exponential backoff: 300 ms, 600 ms, 1.2 s, 2.4 s …
                        let backoff_ms = 300u64 * (1u64 << (total_reconnects - 1));
                        tracing::warn!(
                            "[ferogram] download: worker error ({err}), reconnecting \
                             [{total_reconnects}/{MAX_WORKER_RECONNECTS}] (backoff {backoff_ms}ms)"
                        );
                        // Check abort before sleeping.
                        // full backoff duration when another worker has already failed.
                        if abort.load(Ordering::Relaxed) {
                            abort.store(true, Ordering::Relaxed);
                            return Err(err);
                        }
                        tokio::time::sleep(std::time::Duration::from_millis(backoff_ms)).await;
                        match client.open_worker_conn(worker_dc).await {
                            Ok(c) => {
                                conn = c;
                            }
                            Err(e) => {
                                abort.store(true, Ordering::Relaxed);
                                return Err(e);
                            }
                        }
                    };
                    let mut cur = Cursor::from_slice(&raw);
                    match tl::enums::upload::File::deserialize(&mut cur)? {
                        tl::enums::upload::File::File(f) => {
                            // Validate chunk size; short interior chunk shifts
                            // all subsequent parts to wrong offsets, silently corrupting output.
                            let expected = if part == n_parts - 1 {
                                size - part * chunk
                            } else {
                                chunk
                            };
                            if f.bytes.len() != expected {
                                abort.store(true, Ordering::Relaxed);
                                return Err(InvocationError::Deserialize(format!(
                                    "download part {part}: expected {expected} B, got {} B",
                                    f.bytes.len()
                                )));
                            }
                            let _ = tx.send((part, f.bytes));
                        }
                        tl::enums::upload::File::CdnRedirect(_redir) => {
                            // Signal error to collector task.
                            // leaving the slot as None and producing a corrupt file with a
                            // missing chunk.  Return an error so the caller can fall back to
                            // sequential download which has CDN handling.
                            abort.store(true, Ordering::Relaxed);
                            return Err(InvocationError::Deserialize(
                                "upload.fileCdnRedirect: CDN redirect received in concurrent \
                                 download; retry via sequential path"
                                    .into(),
                            ));
                        }
                    }
                }
                Ok(())
            });
        }
        drop(tx);

        // Writer-ordering safety: workers send (part_index, bytes) over a channel.
        // We slot each chunk into a pre-allocated Vec<Option<Vec<u8>>> at its exact
        // index  - no seek, no append, no race.  Final assembly iterates indices 0..n
        // in order, so out-of-order arrival from parallel workers never corrupts output.
        let mut parts: Vec<Option<Vec<u8>>> = (0..n_parts).map(|_| None).collect();
        // Break as soon as abort is set so tasks.abort_all() fires
        // while workers are still alive (in backoff), not after they have all exited.
        while let Some((idx, data)) = rx.recv().await {
            if idx < parts.len() {
                parts[idx] = Some(data);
            }
            if abort.load(Ordering::Relaxed) {
                break;
            }
        }
        // Drain any chunks buffered before we broke out.
        while let Ok((idx, data)) = rx.try_recv() {
            if idx < parts.len() {
                parts[idx] = Some(data);
            }
        }

        while let Some(res) = tasks.join_next().await {
            if let Err(e) =
                res.map_err(|e| InvocationError::Io(std::io::Error::other(e.to_string())))?
            {
                tasks.abort_all(); // now fires while remaining workers are still live
                return Err(e);
            }
        }

        let mut out = Vec::with_capacity(size);
        for part in parts.into_iter().flatten() {
            out.extend_from_slice(&part);
        }
        out.truncate(size);
        Ok(out)
    }

    /// Download any [`Downloadable`] item.
    ///
    /// Uses concurrent mode for files > 30 MB (`kUseBigFilesFrom`),
    /// sequential for smaller files.
    pub async fn download<D: Downloadable>(&self, item: &D) -> Result<Vec<u8>, InvocationError> {
        let loc = item
            .to_input_location()
            .ok_or_else(|| InvocationError::Deserialize("item has no download location".into()))?;
        let dc = item.dc_id();
        // Always use concurrent path when size is known  - even for small files  -
        // so every download gets a fresh dedicated connection (no idle-conn eof).
        // When size is unknown fall back to sequential (also uses fresh conn now).
        match item.size() {
            Some(sz) => self.download_media_concurrent_on_dc(loc, sz, dc).await,
            None => self.download_media_on_dc(loc, dc).await,
        }
    }
}

// InputFileLocation from IncomingMessage

impl crate::update::IncomingMessage {
    /// Get the download location for the media in this message, if any.
    pub fn download_location(&self) -> Option<tl::enums::InputFileLocation> {
        let media = match &self.raw {
            tl::enums::Message::Message(m) => m.media.as_ref()?,
            _ => return None,
        };
        if let Some(doc) = Document::from_media(media) {
            return doc.to_input_location();
        }
        if let Some(photo) = Photo::from_media(media) {
            return photo.to_input_location();
        }
        None
    }

    /// Like [`download_location`] but also returns the file's DC id.
    pub fn download_location_with_dc(&self) -> Option<(tl::enums::InputFileLocation, i32)> {
        let media = match &self.raw {
            tl::enums::Message::Message(m) => m.media.as_ref()?,
            _ => return None,
        };
        if let Some(doc) = Document::from_media(media) {
            return Some((doc.to_input_location()?, doc.dc_id()));
        }
        if let Some(photo) = Photo::from_media(media) {
            return Some((photo.to_input_location()?, photo.dc_id()));
        }
        None
    }
}

/// Extract a download [`InputFileLocation`] and DC id from a raw `MessageMedia`.
///
/// Used by [`IncomingMessage::download_media_with`].
/// Returns `(location, dc_id)` or `None` when the media has no downloadable file.
pub fn download_location_from_media(
    media: Option<&tl::enums::MessageMedia>,
) -> Option<(tl::enums::InputFileLocation, i32)> {
    let m = media?;
    if let Some(doc) = Document::from_media(m) {
        return Some((doc.to_input_location()?, doc.dc_id()));
    }
    if let Some(photo) = Photo::from_media(m) {
        return Some((photo.to_input_location()?, photo.dc_id()));
    }
    None
}

// Helpers

fn make_input_file(
    big: bool,
    file_id: i64,
    total_parts: i32,
    name: &str,
    data: &[u8],
) -> tl::enums::InputFile {
    if big {
        tl::enums::InputFile::Big(tl::types::InputFileBig {
            id: file_id,
            parts: total_parts,
            name: name.to_string(),
        })
    } else {
        // Compute MD5 over the full file data for server-side integrity.
        // verification.  An empty checksum bypasses the check on DC1/DC4/DC5 and
        // can cause random MEDIA_EMPTY / FILE_PART_0_MISSING on sendMedia.
        // md5 = "0.7" API: md5::compute(data) returns md5::Digest, formatted with {:x}.
        let md5_checksum = format!("{:x}", md5::compute(data));
        tl::enums::InputFile::InputFile(tl::types::InputFile {
            id: file_id,
            parts: total_parts,
            name: name.to_string(),
            md5_checksum,
        })
    }
}
