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

use crate::DcEntry;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};

use ferogram_tl_types as tl;
use ferogram_tl_types::{Cursor, Deserializable};
use tokio::sync::Mutex;

use crate::{Client, InvocationError};

/// One in-flight pipelined upload request: (part index, part length in
/// bytes, response future). Used as the sliding-window element in
/// [`Client::upload_file_concurrent_streaming_pipelined`].
type PipelinedUploadSlot = (
    i32,
    u64,
    std::pin::Pin<Box<dyn std::future::Future<Output = Result<Vec<u8>, InvocationError>> + Send>>,
);

/// One in-flight pipelined download request: (part index, response future).
/// Used as the sliding-window element in
/// [`Client::download_media_concurrent_on_dc_to_file_pipelined`].
type PipelinedDownloadSlot = (
    usize,
    std::pin::Pin<Box<dyn std::future::Future<Output = Result<Vec<u8>, InvocationError>> + Send>>,
);

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

    /// Set a HTML-formatted caption on this album item.
    ///
    /// Parses `html` into plain text + entities and stores both, so the
    /// formatting is preserved when the album is sent.
    #[cfg(feature = "parsers")]
    pub fn caption_html(mut self, html: impl Into<String>) -> Self {
        let (text, ents) = crate::parsers::parse_html(html.into().as_str());
        self.caption = text;
        self.entities = ents;
        self
    }

    /// Set a Markdown-formatted caption on this album item.
    #[cfg(feature = "parsers")]
    pub fn caption_markdown(mut self, md: impl Into<String>) -> Self {
        let (text, ents) = crate::parsers::parse_markdown(md.into().as_str());
        self.caption = text;
        self.entities = ents;
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

/// Adaptive download chunk size based on file size.
///
/// Telegram's `upload.getFile` rule: `(offset + limit)` must not cross a 1 MB
/// boundary. Using chunk sizes that are powers of two and aligning offsets as
/// `part * chunk` always satisfies this. 1 MB chunks are only used at >= 500 MB
/// where the round-trip savings justify the stricter alignment requirement.
///
/// | File size    | Chunk  | Rationale                          |
/// |--------------|--------|------------------------------------|
/// | < 50 MB      | 256 KB | small files, fewer parts is fine   |
/// | 50-500 MB    | 512 KB | halves round-trips vs 256 KB       |
/// | >= 500 MB    | 1 MB   | halves round-trips vs 512 KB       |
pub fn download_chunk_size(file_size: usize) -> i32 {
    if file_size < 50 * 1024 * 1024 {
        256 * 1024 // 256 KB
    } else if file_size < 500 * 1024 * 1024 {
        512 * 1024 // 512 KB
    } else {
        // 1 MB chunks for files >= 500 MB.
        // 1 MB-aligned offsets (0, 1 MB, 2 MB, ...) never cross a 1 MB boundary,
        // satisfying Telegram's offset+limit constraint. This halves RPC round-trips
        // vs 512 KB for very large files with negligible memory overhead.
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
/// `ClientInner::worker_semaphore` which is initialised with this many permits.
pub const MAX_GLOBAL_SENDERS: usize = 12;

/// Default trucks-per-highway depth (X): how many chunk requests a single
/// pipelined transfer connection keeps in flight at once. Matches
/// Android's upload connection pool depth.
pub const DEFAULT_PIPELINE_DEPTH: usize = 4;

/// Hard ceiling for [`crate::TransferLimits::download_pipeline_depth`] /
/// `upload_pipeline_depth`, regardless of what the user requests.
///
/// Each in-flight request holds a full chunk buffer in memory
/// (`{download,upload}_tcp_connections * pipeline_depth` buffers per
/// transfer), so unlike those fields this ceiling exists to bound memory, not to protect
/// the server from connection shedding.
pub const MAX_PIPELINE_DEPTH: usize = 8;

/// Files larger than this use `upload.saveBigFilePart`  - Telegram protocol spec.
/// MUST be 10 MB, not 30 MB. **Upload-only**: `GetFile` has no small/big
/// distinction, so this never applies to downloads. See
/// [`DOWNLOAD_CONCURRENT_THRESHOLD`] for the download-side equivalent.
pub const BIG_FILE_THRESHOLD: usize = 10 * 1024 * 1024;

/// Routing threshold for downloads: below this, `Client::download` uses a
/// plain single-connection stream; at or above it, downloads go through
/// the concurrent/pipelined engine (Y worker connections, each X requests
/// deep). Independent of [`BIG_FILE_THRESHOLD`] on purpose - that constant
/// is upload's protocol-mandated `saveBigFilePart` boundary, and reusing it
/// here was coincidental, not protocol-driven. Same value today, but free
/// to diverge since nothing in the download path actually depends on it
/// matching the upload number.
pub const DOWNLOAD_CONCURRENT_THRESHOLD: usize = 10 * 1024 * 1024;

/// Maximum parts per upload.
#[allow(dead_code)]
const UPLOAD_MAX_PARTS: i32 = 4000;

/// Maximum upload part size (512 KB). Telegram's protocol hard ceiling.
pub const MAX_PART_SIZE: usize = 512 * 1024;

/// Maximum bytes in-flight per upload session  -
/// `kMaxUploadPerSession = 1 MB`.
#[allow(dead_code)]
const UPLOAD_MAX_PER_SESSION: usize = 1024 * 1024;

/// Upload part sizes tried in order  -
/// `kDocumentUploadPartSize{0..2}`.
#[allow(dead_code)]
const UPLOAD_PART_SIZES: &[usize] = &[128 * 1024, 256 * 1024, 512 * 1024];

/// Choose upload part size for `file_size` bytes.
///
/// Upload part size table:
/// - < 1 MB  → 128 KB
/// - 1-50 MB → 256 KB
/// - > 50 MB → 512 KB
///
/// Returns `(part_size_bytes, total_parts)`.
pub fn upload_part_size(file_size: usize) -> (usize, i32) {
    // Three-tier part-size table. Fewer, larger parts than the old five-tier
    // table  - less per-chunk RPC overhead, still nowhere near the 4000-part
    // hard limit for any file size that matters in practice.
    //
    // | File size   | Part size | Max parts |
    // |-------------|-----------|-----------|
    // | < 1 MB      | 128 KB    | 8         |
    // | 1 MB - 50 MB | 256 KB   | 200       |
    // | > 50 MB     | 512 KB    | grows only for files needing >4000 parts |
    const MAX_PARTS: usize = 4000;
    let mut ps: usize = if file_size < 1024 * 1024 {
        128 * 1024 // < 1 MB  → 128 KB
    } else if file_size < 50 * 1024 * 1024 {
        256 * 1024 // < 50 MB → 256 KB
    } else {
        512 * 1024 // ≥ 50 MB → 512 KB
    };
    // Safety: if the chosen tier still exceeds 4000 parts (large files at
    // the flat 512 KB tier - e.g. premium's up to ~4 GB uploads), grow to
    // the minimum part size that fits.
    if file_size.div_ceil(ps) > MAX_PARTS {
        ps = file_size.div_ceil(MAX_PARTS);
        ps = ps.div_ceil(512) * 512; // round up to 512-byte boundary (protocol requirement)
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

/// Concurrent download workers (Y) for `file_size` bytes.
///
/// `max_workers` is the caller's configured ceiling - in practice
/// `ClientInner::transfer_limits.{download,upload}_tcp_connections`, itself already clamped
/// to [`MAX_WORKERS_PER_FILE`] (see [`crate::TransferLimits`]). The size
/// tiers below are Ferogram's own tuning and stay fixed; `max_workers` only
/// ever pulls the result *down*, never past the absolute ceiling.
///
/// | File size    | Workers (uncapped) |
/// |--------------|---------------------|
/// | < 10 MB      | 1                   |
/// | 10 - 50 MB   | 2                   |
/// | 50 - 300 MB  | 3                   |
/// | > 300 MB     | [`MAX_WORKERS_PER_FILE`] |
///
/// The 300 MB boundary avoids the 199 MB → 3 / 202 MB → 4 cliff that a
/// 200 MB cutoff would create  - files cluster around round sizes.
pub fn download_worker_count(file_size: usize, max_workers: usize) -> usize {
    let tiered = if file_size < 10 * 1024 * 1024 {
        1
    } else if file_size < 50 * 1024 * 1024 {
        2
    } else if file_size < 300 * 1024 * 1024 {
        3
    } else {
        MAX_WORKERS_PER_FILE
    };
    tiered.min(max_workers.max(1))
}

/// Concurrent upload workers (Y) for `file_size` bytes.
///
/// See [`download_worker_count`] for what `max_workers` means; the same
/// contract applies here.
///
/// | File size    | Workers (uncapped) |
/// |--------------|---------------------|
/// | < 10 MB      | 1                   |
/// | 10 - 100 MB  | 2                   |
/// | 100 - 500 MB | 3                   |
/// | > 500 MB     | [`MAX_WORKERS_PER_FILE`] |
pub fn upload_worker_count(file_size: usize, max_workers: usize) -> usize {
    let tiered = if file_size < 10 * 1024 * 1024 {
        1
    } else if file_size < 100 * 1024 * 1024 {
        2
    } else if file_size < 500 * 1024 * 1024 {
        3
    } else {
        MAX_WORKERS_PER_FILE
    };
    tiered.min(max_workers.max(1))
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

/// Public wrapper for `resolve_mime` used by `client/files.rs` experimental code.
pub fn resolve_mime_pub(name: &str) -> String {
    resolve_mime(name, "")
}

/// Detect MIME from bytes first (magic bytes), fall back to extension.
/// Used when uploading to ensure correct media type even without extension.
pub fn detect_mime_from_bytes(bytes: &[u8], name: &str) -> String {
    crate::file_info::detect_mime(bytes, name)
}
/// A successfully uploaded file handle, ready to be sent as media.
#[derive(Debug, Clone)]
pub struct UploadedFile {
    pub(crate) inner: tl::enums::InputFile,
    pub(crate) mime_type: String,
    pub(crate) name: String,
}

impl UploadedFile {
    /// Construct an `UploadedFile` from its parts. Used by experimental resumable upload.
    pub(crate) fn new(inner: tl::enums::InputFile, mime_type: String, name: String) -> Self {
        Self {
            inner,
            mime_type,
            name,
        }
    }

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

    /// Automatically choose the best `InputMedia` type based on MIME.
    ///
    /// | MIME              | Sent as   |
    /// |-------------------|-----------|
    /// | `image/gif`       | Animation |
    /// | `image/*`         | Photo     |
    /// | `video/*`         | Video     |
    /// | `audio/ogg`       | Voice     |
    /// | `audio/*`         | Audio     |
    /// | everything else   | Document  |
    pub fn as_auto_media(&self) -> tl::enums::InputMedia {
        let mime = self.mime_type.as_str();
        let name = self.name.as_str();

        if mime == "image/gif" {
            return tl::enums::InputMedia::UploadedDocument(
                tl::types::InputMediaUploadedDocument {
                    nosound_video: false,
                    force_file: false,
                    spoiler: false,
                    file: self.inner.clone(),
                    thumb: None,
                    mime_type: mime.to_string(),
                    attributes: vec![
                        tl::enums::DocumentAttribute::Animated,
                        tl::enums::DocumentAttribute::Filename(
                            tl::types::DocumentAttributeFilename {
                                file_name: name.to_string(),
                            },
                        ),
                    ],
                    stickers: None,
                    ttl_seconds: None,
                    video_cover: None,
                    video_timestamp: None,
                },
            );
        }

        if mime.starts_with("image/") {
            return self.as_photo_media();
        }

        if mime.starts_with("video/") {
            return tl::enums::InputMedia::UploadedDocument(
                tl::types::InputMediaUploadedDocument {
                    nosound_video: false,
                    force_file: false,
                    spoiler: false,
                    file: self.inner.clone(),
                    thumb: None,
                    mime_type: mime.to_string(),
                    attributes: vec![
                        tl::enums::DocumentAttribute::Video(tl::types::DocumentAttributeVideo {
                            round_message: false,
                            supports_streaming: true,
                            nosound: false,
                            duration: 0.0,
                            w: 0,
                            h: 0,
                            preload_prefix_size: None,
                            video_start_ts: None,
                            video_codec: None,
                        }),
                        tl::enums::DocumentAttribute::Filename(
                            tl::types::DocumentAttributeFilename {
                                file_name: name.to_string(),
                            },
                        ),
                    ],
                    stickers: None,
                    ttl_seconds: None,
                    video_cover: None,
                    video_timestamp: None,
                },
            );
        }

        if mime == "audio/ogg" || mime == "application/ogg" {
            return tl::enums::InputMedia::UploadedDocument(
                tl::types::InputMediaUploadedDocument {
                    nosound_video: false,
                    force_file: false,
                    spoiler: false,
                    file: self.inner.clone(),
                    thumb: None,
                    mime_type: mime.to_string(),
                    attributes: vec![
                        tl::enums::DocumentAttribute::Audio(tl::types::DocumentAttributeAudio {
                            voice: true,
                            duration: 0,
                            title: None,
                            performer: None,
                            waveform: None,
                        }),
                        tl::enums::DocumentAttribute::Filename(
                            tl::types::DocumentAttributeFilename {
                                file_name: name.to_string(),
                            },
                        ),
                    ],
                    stickers: None,
                    ttl_seconds: None,
                    video_cover: None,
                    video_timestamp: None,
                },
            );
        }

        if mime.starts_with("audio/") {
            let stem = std::path::Path::new(name)
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or(name);
            return tl::enums::InputMedia::UploadedDocument(
                tl::types::InputMediaUploadedDocument {
                    nosound_video: false,
                    force_file: false,
                    spoiler: false,
                    file: self.inner.clone(),
                    thumb: None,
                    mime_type: mime.to_string(),
                    attributes: vec![
                        tl::enums::DocumentAttribute::Audio(tl::types::DocumentAttributeAudio {
                            voice: false,
                            duration: 0,
                            title: Some(stem.to_string()),
                            performer: None,
                            waveform: None,
                        }),
                        tl::enums::DocumentAttribute::Filename(
                            tl::types::DocumentAttributeFilename {
                                file_name: name.to_string(),
                            },
                        ),
                    ],
                    stickers: None,
                    ttl_seconds: None,
                    video_cover: None,
                    video_timestamp: None,
                },
            );
        }

        self.as_document_media()
    }
}

/// `UploadedFile` converts to `InputMedia` automatically using MIME detection.
///
/// This lets you pass an `UploadedFile` directly to [`Client::send_file`]:
///
/// ```rust,no_run
/// # use ferogram::{Client, InputMessage};
/// # async fn ex(client: Client) -> anyhow::Result<()> {
/// let uploaded = client.upload_file("video.mp4").await?;
/// client.send_file("me", uploaded, &InputMessage::default()).await?;
/// # Ok(()) }
/// ```
impl From<UploadedFile> for tl::enums::InputMedia {
    fn from(f: UploadedFile) -> Self {
        f.as_auto_media()
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

// Quality selection (alt_documents)

/// Which quality variant of a video to download, when the media carries
/// alternate qualities (`alt_documents`) alongside its primary document.
///
/// Telegram sends alternate qualities as extra [`Document`]s attached to
/// the same [`MessageMedia`](tl::enums::MessageMedia), each carrying its
/// own `documentAttributeVideo` (width/height). Ferogram picks among them
/// by resolution only - it doesn't try to distinguish encoding or bitrate
/// beyond what the resolution implies.
///
/// Media with no alternates (photos, audio, most documents, or videos
/// Telegram didn't transcode) only ever have `Original` to pick from -
/// every variant here falls back to it automatically.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MediaQuality {
    /// The primary document Telegram sends by default - what most clients
    /// show first, before you open a quality picker.
    #[default]
    Original,
    /// The highest-resolution alternate available. Falls back to
    /// `Original` when there are no alternates.
    Highest,
    /// The lowest-resolution alternate available. Falls back to
    /// `Original` when there are no alternates.
    Lowest,
}

/// One available quality variant for a video, as reported by
/// [`available_qualities`].
#[derive(Clone, Copy, Debug)]
pub struct VideoQualityInfo {
    pub width: i32,
    pub height: i32,
    pub size: i64,
}

/// Collect every `Document` on this media that could be a quality
/// variant: the primary document (if any) followed by every entry in
/// `alt_documents` (if any). Order is: primary first, then alternates in
/// the order Telegram sent them - not sorted by resolution.
fn quality_candidates(media: &tl::enums::MessageMedia) -> Vec<tl::types::Document> {
    let tl::enums::MessageMedia::Document(md) = media else {
        return Vec::new();
    };
    let mut docs = Vec::new();
    if let Some(tl::enums::Document::Document(d)) = &md.document {
        docs.push(d.clone());
    }
    if let Some(alts) = &md.alt_documents {
        for alt in alts {
            if let tl::enums::Document::Document(d) = alt {
                docs.push(d.clone());
            }
        }
    }
    docs
}

/// Resolution (width * height) of a document, from its
/// `documentAttributeVideo` if it has one. `0` for documents with no
/// video attribute (audio, plain files) so they sort below anything with
/// a real resolution.
fn video_resolution(doc: &tl::types::Document) -> i64 {
    doc.attributes
        .iter()
        .find_map(|a| match a {
            tl::enums::DocumentAttribute::Video(v) => Some(v.w as i64 * v.h as i64),
            _ => None,
        })
        .unwrap_or(0)
}

/// List every quality variant available for this media, sorted ascending
/// by resolution (primary document included if it has a video
/// attribute). Empty for media with no video attributes at all - photos,
/// audio, and documents Telegram didn't transcode into multiple
/// qualities.
///
/// Use this to build your own quality picker UI, or just reach for
/// [`MediaQuality::Highest`] / [`MediaQuality::Lowest`] directly if you
/// don't need the full list.
pub fn available_qualities(media: &tl::enums::MessageMedia) -> Vec<VideoQualityInfo> {
    let mut out: Vec<VideoQualityInfo> = quality_candidates(media)
        .iter()
        .filter_map(|d| {
            d.attributes.iter().find_map(|a| match a {
                tl::enums::DocumentAttribute::Video(v) => Some(VideoQualityInfo {
                    width: v.w,
                    height: v.h,
                    size: d.size,
                }),
                _ => None,
            })
        })
        .collect();
    out.sort_by_key(|q| q.width as i64 * q.height as i64);
    out
}

/// Resolve `quality` against `media`, returning the chosen [`Document`].
///
/// Falls back to the primary document whenever the requested quality
/// doesn't actually exist for this media (no alternates present, the
/// media isn't a document at all, or `Original` was requested outright).
pub(crate) fn resolve_quality_document(
    media: &tl::enums::MessageMedia,
    quality: MediaQuality,
) -> Option<Document> {
    let primary = match media {
        tl::enums::MessageMedia::Document(md) => match &md.document {
            Some(tl::enums::Document::Document(d)) => Some(Document::from_raw(d.clone())),
            _ => None,
        },
        _ => None,
    };

    let chosen = match quality {
        MediaQuality::Original => None,
        MediaQuality::Highest => quality_candidates(media)
            .into_iter()
            .max_by_key(video_resolution),
        MediaQuality::Lowest => quality_candidates(media)
            .into_iter()
            .min_by_key(video_resolution),
    };

    chosen.map(Document::from_raw).or(primary)
}

// DownloadIter

/// Sequential chunk-by-chunk download iterator.
pub struct DownloadIter {
    client: Client,
    conn: Option<crate::dc_pool::DcConnection>,
    request: Option<tl::functions::upload::GetFile>,
    done: bool,
    dc_id: i32,
}

impl DownloadIter {
    pub(crate) fn new(client: Client, location: tl::enums::InputFileLocation, dc_id: i32) -> Self {
        Self {
            client,
            conn: None,
            done: false,
            dc_id,
            request: Some(tl::functions::upload::GetFile {
                precise: true,
                cdn_supported: false,
                location,
                offset: 0,
                limit: 512 * 1024,
            }),
        }
    }

    /// Set a custom chunk size (must be multiple of 4096, max 524288).
    pub fn chunk_size(mut self, size: i32) -> Self {
        if let Some(r) = &mut self.request {
            r.limit = size;
        }
        self
    }

    /// Start iteration from `offset` bytes into the file.
    ///
    /// The offset is aligned down to the nearest chunk boundary so it satisfies
    /// Telegram's `offset + limit` rule. Use this to implement HTTP range requests:
    /// pass the `Range` header's start byte and skip any leading bytes that fall
    /// before the requested range in the first chunk.
    ///
    /// ```rust,no_run
    /// # use ferogram::Client;
    /// # use ferogram::tl;
    /// # async fn ex(client: Client, media: tl::enums::MessageMedia) {
    /// let mut iter = client.iter_download(&media).unwrap().start_at(1024 * 1024);
    /// while let Some(chunk) = iter.next().await.unwrap() {
    ///     // stream chunk to HTTP response
    /// }
    /// # }
    /// ```
    pub fn start_at(mut self, offset: i64) -> Self {
        if let Some(r) = &mut self.request {
            let chunk = r.limit as i64;
            r.offset = (offset / chunk) * chunk;
        }
        self
    }

    /// Fetch the next chunk. Returns `None` when the download is complete.
    ///
    /// Opens a dedicated worker connection on the first call and reuses it for
    /// all subsequent chunks. The connection is isolated from the main session
    /// and the shared transfer pool, so it never contends with other requests.
    pub async fn next(&mut self) -> Result<Option<Vec<u8>>, InvocationError> {
        if self.done {
            return Ok(None);
        }
        let req = match &self.request {
            Some(r) => r.clone(),
            None => return Ok(None),
        };
        if self.conn.is_none() {
            self.conn = Some(self.client.open_worker_conn(self.dc_id).await?);
        }
        let conn = self.conn.as_mut().expect("conn set above");
        let raw = match conn.rpc_call(&req).await {
            Ok(r) => r,
            Err(InvocationError::Rpc(ref rpc)) if rpc.code == 303 => {
                let new_dc = rpc.value.unwrap_or(1) as i32;
                self.dc_id = new_dc;
                let new_conn = self.client.open_worker_conn(new_dc).await?;
                self.conn = Some(new_conn);
                self.conn.as_mut().unwrap().rpc_call(&req).await?
            }
            Err(e) => return Err(e),
        };
        let mut cur = Cursor::from_slice(&raw);
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
    ///   < 1 MB → 128 KB, 1-50 MB → 256 KB, > 50 MB → 512 KB.
    /// - `upload.saveBigFilePart` used for files > 30 MB (`kUseBigFilesFrom`).
    ///
    /// For files that benefit from parallelism use [`upload_file_concurrent`].
    pub(crate) async fn upload_bytes(
        &self,
        data: &[u8],
        name: &str,
        mime_type: &str,
        handle: Option<&crate::transfer::TransferHandle>,
    ) -> Result<UploadedFile, InvocationError> {
        // Zero-byte upload produces parts=0; add 1 to satisfy FILE_PART_0_MISSING check.
        if data.is_empty() {
            return Err(InvocationError::Deserialize(
                "cannot upload empty file".into(),
            ));
        }
        // Prefer magic-byte detection over extension when mime_type is not already known.
        let resolved_mime = if mime_type.is_empty() || mime_type == "application/octet-stream" {
            detect_mime_from_bytes(data, name)
        } else {
            resolve_mime(name, mime_type)
        };
        let total = data.len();
        let big = total > BIG_FILE_THRESHOLD;
        // Pick smallest part size that keeps total_parts <= 4000.
        let (part_size, total_parts) = upload_part_size(total);
        let file_id = crate::random_i64_pub();

        if let Some(h) = handle {
            h.set_total(total as u64);
            h.reset_start();
        }

        for (part_num, chunk) in data.chunks(part_size).enumerate() {
            if let Some(h) = handle {
                h.poll_pause_cancel().await?;
            }
            let chunk_len = chunk.len();
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
            if let Some(h) = handle {
                h.add_bytes(chunk_len as u64);
            }
        }

        let inner = make_input_file(big, file_id, total_parts, name, data);
        tracing::info!(
            "[ferogram::transfer] upload complete: '{}' ({} bytes, {}B parts x {}, mime={})",
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
    /// Part size: 128 KB for tiny files, up to 512 KB otherwise.
    ///
    /// - Files < 10 MB  -> `upload.saveFilePart`    (small-file API)
    /// - Files >= 10 MB -> `upload.saveBigFilePart`  (big-file API)
    pub async fn upload_file_concurrent(
        &self,
        data: Arc<Vec<u8>>,
        name: &str,
        mime_type: &str,
        handle: Option<&crate::transfer::TransferHandle>,
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
        // Per-file ceiling: transfer_limits.upload_tcp_connections (clamped to MAX_WORKERS_PER_FILE).
        // Global ceiling: transfer_limits.max_tcp_connections permits.
        let n_workers = if self.inner.transfer_limits.bypass_tcp_allotments {
            self.inner.transfer_limits.upload_tcp_connections
        } else {
            upload_worker_count(total, self.inner.transfer_limits.upload_tcp_connections)
        };

        let home_dc = *self.inner.home_dc_id.lock().await;
        let started = std::time::Instant::now();
        let total_mib = total as f64 / (1024.0 * 1024.0);
        tracing::info!(
            "[ferogram::transfer] upload starting: '{}' ({:.1} MiB / {} bytes, {} parts x {}B, DC{}, Y={} connections)",
            name,
            total_mib,
            total,
            total_parts,
            part_size,
            home_dc,
            n_workers
        );

        if let Some(h) = handle {
            h.set_total(total as u64);
            h.reset_start();
        }
        // Arc-wrap handle so workers can report progress without lifetime issues.
        let shared_handle: Option<crate::transfer::TransferHandle> = handle.cloned();
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
                Ok(Err(e)) => {
                    tracing::debug!("[ferogram::transfer] upload worker connection failed: {e}")
                }
                Err(e) => tracing::debug!("[ferogram::transfer] upload worker task panicked: {e}"),
            }
        }
        if conns.is_empty() {
            tracing::debug!(
                "[ferogram::transfer] no worker connections available; uploading sequentially"
            );
            return self
                .upload_bytes(&data, name, mime_type, shared_handle.as_ref())
                .await;
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
            let worker_handle = shared_handle.clone();

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
                    let chunk_len = bytes.len() as u64;

                    // Pause / cancel check before each chunk.
                    if let Some(ref h) = worker_handle {
                        h.poll_pause_cancel().await?;
                    }

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
                            Ok(_) => {
                                if let Some(ref h) = worker_handle {
                                    h.add_bytes(chunk_len);
                                }
                                break;
                            }
                            Err(e) => e,
                        };
                        if let InvocationError::Rpc(ref rpc) = err {
                            // FLOOD_WAIT: sleep, retry same conn.
                            if rpc.code == 420 {
                                let secs = rpc.value.unwrap_or(1) as u64;
                                tracing::debug!("[ferogram::transfer] upload throttled by FLOOD_WAIT_{secs}; sleeping before retry"
                                );
                                tokio::time::sleep(std::time::Duration::from_secs(secs)).await;
                                continue;
                            }
                            // FILE_MIGRATE: server redirected upload to a different DC.
                            if rpc.code == 303 {
                                let new_dc = rpc.value.unwrap_or(1) as i32;
                                tracing::debug!("[ferogram::transfer] upload redirected by FILE_MIGRATE to DC{new_dc} (was DC{worker_dc})"
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
                                    "[ferogram::transfer] upload: AUTH_KEY_UNREGISTERED on DC{worker_dc}; re-establishing worker connection (attempt {}/{MAX_WORKER_RECONNECTS})",
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
                            "[ferogram::transfer] upload worker error ({err}); reconnecting (attempt {total_reconnects}/{MAX_WORKER_RECONNECTS}, backoff {backoff_ms}ms)"
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
            "[ferogram::transfer] upload complete: '{}' ({:.1} MiB / {} bytes, {} parts x {}B, DC{}, Y={} connections, took {:.2}s)",
            name,
            total_mib,
            total,
            total_parts,
            part_size,
            home_dc,
            actual_workers,
            started.elapsed().as_secs_f64()
        );
        Ok(UploadedFile {
            inner,
            mime_type: resolve_mime(name, mime_type),
            name: name.to_string(),
        })
    }

    /// Like [`upload_file_concurrent`](Self::upload_file_concurrent) but uses
    /// [`PipelinedSender`](crate::client::PipelinedSender) connections instead
    /// of blocking [`DcConnection`](crate::dc_pool::DcConnection)s.
    ///
    /// Each worker keeps up to `transfer_limits.upload_pipeline_depth` (X)
    /// `SaveFilePart`/`SaveBigFilePart` requests in flight on its single
    /// connection at once, instead of awaiting each part's ack before sending
    /// the next (X=1, what `upload_file_concurrent` does). Combined with
    /// `n_workers` separate connections (Y), total in-flight chunks for an
    /// upload is `n_workers * upload_pipeline_depth` instead of just
    /// `n_workers`. See [`TransferLimits`](crate::TransferLimits) for the
    /// highway/trucks model this implements.
    ///
    /// `data` is already fully in memory, so unlike the path-based streaming
    /// variant there is no file handle to juggle on reconnect or DC
    /// migration - every worker just slices the same `Arc<Vec<u8>>`.
    ///
    /// Falls back to the existing non-pipelined path if pipelined connections
    /// fail to open, same reliability guarantees either way.
    pub async fn upload_file_concurrent_pipelined(
        &self,
        data: Arc<Vec<u8>>,
        name: &str,
        mime_type: &str,
        handle: Option<&crate::transfer::TransferHandle>,
    ) -> Result<UploadedFile, InvocationError> {
        if data.is_empty() {
            return Err(InvocationError::Deserialize(
                "cannot upload empty file".into(),
            ));
        }
        let total = data.len();
        let (part_size, total_parts) = upload_part_size(total);
        let big = total > BIG_FILE_THRESHOLD;
        let n_workers = if self.inner.transfer_limits.bypass_tcp_allotments {
            self.inner.transfer_limits.upload_tcp_connections
        } else {
            upload_worker_count(total, self.inner.transfer_limits.upload_tcp_connections)
        };

        let home_dc = *self.inner.home_dc_id.lock().await;
        let started = std::time::Instant::now();
        let total_mib = total as f64 / (1024.0 * 1024.0);
        tracing::info!(
            "[ferogram::transfer] pipelined upload starting: '{}' ({:.1} MiB / {} bytes, {} parts x {}B, DC{}, Y={} connections, X={} in-flight)",
            name,
            total_mib,
            total,
            total_parts,
            part_size,
            home_dc,
            n_workers,
            self.inner.transfer_limits.upload_pipeline_depth
        );

        if let Some(h) = handle {
            h.set_total(total as u64);
            h.reset_start();
        }
        let shared_handle: Option<crate::transfer::TransferHandle> = handle.cloned();

        let _global_guard = self
            .inner
            .worker_semaphore
            .acquire_many(n_workers as u32)
            .await
            .expect("worker semaphore unexpectedly closed");

        // Single-worker small files: skip pipelined setup overhead entirely.
        if n_workers == 1 {
            drop(_global_guard);
            return self
                .upload_bytes(&data, name, mime_type, shared_handle.as_ref())
                .await;
        }

        let file_id_atomic =
            std::sync::Arc::new(std::sync::atomic::AtomicI64::new(crate::random_i64_pub()));
        let upload_dc = Arc::new(AtomicI32::new(0i32));

        let mut open_set: tokio::task::JoinSet<
            Result<crate::client::PipelinedSender, InvocationError>,
        > = tokio::task::JoinSet::new();
        for _ in 0..n_workers {
            let client = self.clone();
            open_set.spawn(async move { client.open_worker_sender(0).await });
        }
        let mut senders: Vec<crate::client::PipelinedSender> = Vec::with_capacity(n_workers);
        while let Some(res) = open_set.join_next().await {
            match res {
                Ok(Ok(s)) => senders.push(s),
                Ok(Err(e)) => tracing::debug!(
                    "[ferogram::transfer] pipelined upload worker connection failed: {e}"
                ),
                Err(e) => tracing::debug!(
                    "[ferogram::transfer] pipelined upload worker task panicked: {e}"
                ),
            }
        }
        if senders.is_empty() {
            tracing::debug!(
                "[ferogram::transfer] no pipelined worker connections available; falling back to non-pipelined concurrent upload"
            );
            drop(_global_guard);
            return self
                .upload_file_concurrent(data, name, mime_type, handle)
                .await;
        }
        let actual_workers = senders.len();

        let next_part = Arc::new(Mutex::new(0i32));
        let mut tasks: tokio::task::JoinSet<Result<(), InvocationError>> =
            tokio::task::JoinSet::new();

        for sender in senders {
            let data = Arc::clone(&data);
            let next_part = Arc::clone(&next_part);
            let client = self.clone();
            let upload_dc = Arc::clone(&upload_dc);
            let file_id_atomic = std::sync::Arc::clone(&file_id_atomic);
            let worker_handle = shared_handle.clone();
            let mut sender = sender;

            tasks.spawn(async move {
                const MAX_WORKER_RECONNECTS: u8 = 5;
                let mut total_reconnects = 0u8;
                let mut worker_dc = 0i32;

                // Sliding window of (part_num, chunk_len, response_future).
                // Each future resolves to the raw `Bool` RPC response  - upload
                // doesn't need to inspect the payload, just that it succeeded.
                let mut window: std::collections::VecDeque<PipelinedUploadSlot> =
                    std::collections::VecDeque::with_capacity(
                        client.inner.transfer_limits.upload_pipeline_depth,
                    );

                let slice_part = |part_num: i32| -> Vec<u8> {
                    let start = part_num as usize * part_size;
                    let end = (start + part_size).min(data.len());
                    data[start..end].to_vec()
                };

                loop {
                    // Top up the window to upload_pipeline_depth before draining the front.
                    while window.len() < client.inner.transfer_limits.upload_pipeline_depth {
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
                            sender = match client.open_worker_sender(worker_dc).await {
                                Ok(s) => s,
                                Err(e) => return Err(e),
                            };
                            // A redirect invalidates any parts still in flight from the
                            // old file_id/DC; the window was for the old upload session.
                            window.clear();
                        }

                        let bytes = slice_part(part_num);
                        let chunk_len = bytes.len() as u64;

                        if let Some(ref h) = worker_handle {
                            h.poll_pause_cancel().await?;
                        }

                        let raw_req = if big {
                            tl::Serializable::to_bytes(&tl::functions::upload::SaveBigFilePart {
                                file_id,
                                file_part: part_num,
                                file_total_parts: total_parts,
                                bytes,
                            })
                        } else {
                            tl::Serializable::to_bytes(&tl::functions::upload::SaveFilePart {
                                file_id,
                                file_part: part_num,
                                bytes,
                            })
                        };
                        let body = ferogram_connect::util::maybe_gz_pack(&raw_req);

                        match sender.enqueue(body).await {
                            Ok(fut) => {
                                window.push_back((part_num, chunk_len, Box::pin(fut)))
                            }
                            Err(e) => {
                                tracing::debug!(
                                    "[ferogram::transfer] pipelined upload enqueue failed, reconnecting: {e}"
                                );
                                total_reconnects += 1;
                                if total_reconnects >= MAX_WORKER_RECONNECTS {
                                    return Err(e);
                                }
                                let backoff_ms = 300u64 * (1u64 << (total_reconnects - 1));
                                tokio::time::sleep(std::time::Duration::from_millis(backoff_ms))
                                    .await;
                                sender = match client.open_worker_sender(worker_dc).await {
                                    Ok(s) => s,
                                    Err(e) => return Err(e),
                                };
                                // Re-queue this part: rewind past it and anything else
                                // still in the window before clearing.
                                let min_part = window
                                    .iter()
                                    .map(|(p, _, _)| *p)
                                    .min()
                                    .unwrap_or(part_num)
                                    .min(part_num);
                                window.clear();
                                let mut g = next_part.lock().await;
                                *g = (*g).min(min_part);
                                break;
                            }
                        }
                    }

                    if window.is_empty() {
                        // No more parts to claim and nothing in flight: this worker is done.
                        break;
                    }

                    let (part_num, chunk_len, fut) =
                        window.pop_front().expect("window checked non-empty above");
                    let result = fut.await;
                    let err = match result {
                        Ok(_) => {
                            if let Some(ref h) = worker_handle {
                                h.add_bytes(chunk_len);
                            }
                            continue;
                        }
                        Err(e) => e,
                    };

                    if let InvocationError::Rpc(ref rpc) = err {
                        if rpc.code == 420 {
                            let secs = rpc.value.unwrap_or(1) as u64;
                            tracing::debug!(
                                "[ferogram::transfer] pipelined upload throttled by FLOOD_WAIT_{secs}; sleeping before retry"
                            );
                            tokio::time::sleep(std::time::Duration::from_secs(secs)).await;
                            // Retry the same part directly  - re-enqueue at front of window.
                            let file_id =
                                file_id_atomic.load(std::sync::atomic::Ordering::Relaxed);
                            let bytes = slice_part(part_num);
                            let raw_req = if big {
                                tl::Serializable::to_bytes(
                                    &tl::functions::upload::SaveBigFilePart {
                                        file_id,
                                        file_part: part_num,
                                        file_total_parts: total_parts,
                                        bytes,
                                    },
                                )
                            } else {
                                tl::Serializable::to_bytes(&tl::functions::upload::SaveFilePart {
                                    file_id,
                                    file_part: part_num,
                                    bytes,
                                })
                            };
                            let body = ferogram_connect::util::maybe_gz_pack(&raw_req);
                            match sender.enqueue(body).await {
                                Ok(retry_fut) => {
                                    window.push_front((part_num, chunk_len, Box::pin(retry_fut)));
                                    continue;
                                }
                                Err(e) => return Err(e),
                            }
                        }
                        if rpc.code == 303 {
                            let new_dc = rpc.value.unwrap_or(1) as i32;
                            tracing::debug!(
                                "[ferogram::transfer] pipelined upload redirected by FILE_MIGRATE to DC{new_dc} (was DC{worker_dc})"
                            );
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
                            window.clear();
                            sender = match client.open_worker_sender(new_dc).await {
                                Ok(s) => s,
                                Err(e) => return Err(e),
                            };
                            continue;
                        }
                        if rpc.name == "AUTH_KEY_UNREGISTERED" {
                            tracing::warn!(
                                "[ferogram::transfer] pipelined upload: AUTH_KEY_UNREGISTERED on DC{worker_dc}; re-establishing worker connection (attempt {}/{MAX_WORKER_RECONNECTS})",
                                total_reconnects + 1
                            );
                            total_reconnects += 1;
                            if total_reconnects >= MAX_WORKER_RECONNECTS {
                                return Err(err);
                            }
                            let backoff_ms = 300u64 * (1u64 << (total_reconnects - 1));
                            tokio::time::sleep(std::time::Duration::from_millis(backoff_ms)).await;
                            let min_part = window
                                .iter()
                                .map(|(p, _, _)| *p)
                                .min()
                                .unwrap_or(part_num)
                                .min(part_num);
                            window.clear();
                            let mut g = next_part.lock().await;
                            *g = (*g).min(min_part);
                            drop(g);
                            sender = match client.open_worker_sender(worker_dc).await {
                                Ok(s) => s,
                                Err(e) => return Err(e),
                            };
                            continue;
                        }
                        if rpc.code != -503 {
                            return Err(err);
                        }
                    }

                    total_reconnects += 1;
                    if total_reconnects >= MAX_WORKER_RECONNECTS {
                        return Err(err);
                    }
                    let backoff_ms = 300u64 * (1u64 << (total_reconnects - 1));
                    tracing::warn!(
                        "[ferogram::transfer] pipelined upload worker error ({err}); reconnecting (attempt {total_reconnects}/{MAX_WORKER_RECONNECTS}, backoff {backoff_ms}ms)"
                    );
                    tokio::time::sleep(std::time::Duration::from_millis(backoff_ms)).await;
                    let min_part = window
                        .iter()
                        .map(|(p, _, _)| *p)
                        .min()
                        .unwrap_or(part_num)
                        .min(part_num);
                    window.clear();
                    let mut g = next_part.lock().await;
                    *g = (*g).min(min_part);
                    drop(g);
                    sender = match client.open_worker_sender(worker_dc).await {
                        Ok(s) => s,
                        Err(e) => return Err(e),
                    };
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
            "[ferogram::transfer] pipelined upload complete: '{}' ({:.1} MiB / {} bytes, {} parts x {}B, DC{}, Y={} connections, X={} in-flight, took {:.2}s)",
            name,
            total_mib,
            total,
            total_parts,
            part_size,
            home_dc,
            actual_workers,
            self.inner.transfer_limits.upload_pipeline_depth,
            started.elapsed().as_secs_f64()
        );
        Ok(UploadedFile {
            inner,
            mime_type: resolve_mime(name, mime_type),
            name: name.to_string(),
        })
    }

    /// Upload a file from `path` using parallel workers without loading it into RAM.
    ///
    /// Each worker opens its own independent file handle, seeks to its part offset,
    /// reads exactly `part_size` bytes, and sends. Peak RAM usage is
    /// `n_workers * part_size` (at most 4 x 512 KB = 2 MB) regardless of file size.
    pub(crate) async fn upload_file_concurrent_streaming(
        &self,
        path: &std::path::Path,
        name: &str,
        mime_type: &str,
        handle: Option<&crate::transfer::TransferHandle>,
    ) -> Result<UploadedFile, InvocationError> {
        use tokio::io::{AsyncReadExt, AsyncSeekExt};

        let meta = tokio::fs::metadata(path)
            .await
            .map_err(InvocationError::Io)?;
        let total = meta.len() as usize;
        if total == 0 {
            return Err(InvocationError::Deserialize(
                "cannot upload empty file".into(),
            ));
        }

        let (part_size, total_parts) = upload_part_size(total);
        let big = total > BIG_FILE_THRESHOLD;
        let n_workers = if self.inner.transfer_limits.bypass_tcp_allotments {
            self.inner.transfer_limits.upload_tcp_connections
        } else {
            upload_worker_count(total, self.inner.transfer_limits.upload_tcp_connections)
        };

        let home_dc = *self.inner.home_dc_id.lock().await;
        let started = std::time::Instant::now();
        let total_mib = total as f64 / (1024.0 * 1024.0);
        tracing::info!(
            "[ferogram::transfer] upload starting: '{}' ({:.1} MiB / {} bytes, {} parts x {}B, DC{}, Y={} connections, streaming)",
            name,
            total_mib,
            total,
            total_parts,
            part_size,
            home_dc,
            n_workers
        );

        if let Some(h) = handle {
            h.set_total(total as u64);
            h.reset_start();
        }
        let shared_handle: Option<crate::transfer::TransferHandle> = handle.cloned();

        let _global_guard = self
            .inner
            .worker_semaphore
            .acquire_many(n_workers as u32)
            .await
            .expect("worker semaphore unexpectedly closed");

        let file_id_atomic =
            std::sync::Arc::new(std::sync::atomic::AtomicI64::new(crate::random_i64_pub()));
        let upload_dc = Arc::new(AtomicI32::new(0i32));

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
                Ok(Err(e)) => {
                    tracing::debug!("[ferogram::transfer] upload worker connection failed: {e}")
                }
                Err(e) => tracing::debug!("[ferogram::transfer] upload worker task panicked: {e}"),
            }
        }
        if conns.is_empty() {
            tracing::debug!(
                "[ferogram::transfer] no worker connections available; uploading sequentially"
            );
            let mut data = Vec::with_capacity(total);
            tokio::fs::File::open(path)
                .await
                .map_err(InvocationError::Io)?
                .read_to_end(&mut data)
                .await
                .map_err(InvocationError::Io)?;
            return self
                .upload_bytes(&data, name, mime_type, shared_handle.as_ref())
                .await;
        }
        let actual_workers = conns.len();

        let next_part = Arc::new(Mutex::new(0i32));
        let mut tasks: tokio::task::JoinSet<Result<(), InvocationError>> =
            tokio::task::JoinSet::new();

        let path_arc = std::sync::Arc::new(path.to_path_buf());

        for mut conn in conns {
            let next_part = Arc::clone(&next_part);
            let client = self.clone();
            let upload_dc = Arc::clone(&upload_dc);
            let file_id_atomic = std::sync::Arc::clone(&file_id_atomic);
            let worker_handle = shared_handle.clone();
            let path_arc = std::sync::Arc::clone(&path_arc);

            tasks.spawn(async move {
                const MAX_WORKER_RECONNECTS: u8 = 5;
                let mut total_reconnects = 0u8;
                let mut worker_dc = 0i32;

                let mut file = tokio::fs::File::open(&*path_arc)
                    .await
                    .map_err(InvocationError::Io)?;

                loop {
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
                        file = tokio::fs::File::open(&*path_arc)
                            .await
                            .map_err(InvocationError::Io)?;
                    }

                    let start = part_num as u64 * part_size as u64;
                    let end = (start + part_size as u64).min(total as u64);
                    let chunk_len = (end - start) as usize;

                    file.seek(std::io::SeekFrom::Start(start))
                        .await
                        .map_err(InvocationError::Io)?;
                    let mut bytes = vec![0u8; chunk_len];
                    file.read_exact(&mut bytes)
                        .await
                        .map_err(InvocationError::Io)?;

                    let chunk_u64 = chunk_len as u64;

                    if let Some(ref h) = worker_handle {
                        h.poll_pause_cancel().await?;
                    }

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
                            Ok(_) => {
                                if let Some(ref h) = worker_handle {
                                    h.add_bytes(chunk_u64);
                                }
                                break;
                            }
                            Err(e) => e,
                        };
                        if let InvocationError::Rpc(ref rpc) = err {
                            if rpc.code == 420 {
                                let secs = rpc.value.unwrap_or(1) as u64;
                                tracing::debug!("[ferogram::transfer] upload throttled by FLOOD_WAIT_{secs}; sleeping before retry"
                                );
                                tokio::time::sleep(std::time::Duration::from_secs(secs)).await;
                                continue;
                            }
                            if rpc.code == 303 {
                                let new_dc = rpc.value.unwrap_or(1) as i32;
                                tracing::debug!("[ferogram::transfer] upload redirected by FILE_MIGRATE to DC{new_dc} (was DC{worker_dc})"
                                );
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
                                        file = tokio::fs::File::open(&*path_arc)
                                            .await
                                            .map_err(InvocationError::Io)?;
                                        continue;
                                    }
                                    Err(e) => return Err(e),
                                }
                            }
                            if rpc.name == "AUTH_KEY_UNREGISTERED" {
                                tracing::warn!(
                                    "[ferogram::transfer] upload: AUTH_KEY_UNREGISTERED on DC{worker_dc}; re-establishing worker connection (attempt {}/{MAX_WORKER_RECONNECTS})",
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
                            if rpc.code != -503 {
                                return Err(err);
                            }
                        }
                        total_reconnects += 1;
                        if total_reconnects >= MAX_WORKER_RECONNECTS {
                            return Err(err);
                        }
                        let backoff_ms = 300u64 * (1u64 << (total_reconnects - 1));
                        tracing::warn!(
                            "[ferogram::transfer] upload worker error ({err}); reconnecting (attempt {total_reconnects}/{MAX_WORKER_RECONNECTS}, backoff {backoff_ms}ms)"
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
        let inner = if big {
            tl::enums::InputFile::Big(tl::types::InputFileBig {
                id: file_id,
                parts: total_parts,
                name: name.to_string(),
            })
        } else {
            let mut data = Vec::with_capacity(total);
            tokio::fs::File::open(path)
                .await
                .map_err(InvocationError::Io)?
                .read_to_end(&mut data)
                .await
                .map_err(InvocationError::Io)?;
            let md5_checksum = format!("{:x}", md5::compute(&data));
            tl::enums::InputFile::InputFile(tl::types::InputFile {
                id: file_id,
                parts: total_parts,
                name: name.to_string(),
                md5_checksum,
            })
        };
        tracing::info!(
            "[ferogram::transfer] upload complete: '{}' ({:.1} MiB / {} bytes, {} parts x {}B, DC{}, Y={} connections, streaming, took {:.2}s)",
            name,
            total_mib,
            total,
            total_parts,
            part_size,
            home_dc,
            actual_workers,
            started.elapsed().as_secs_f64()
        );
        Ok(UploadedFile {
            inner,
            mime_type: resolve_mime(name, mime_type),
            name: name.to_string(),
        })
    }

    /// Like [`upload_file_concurrent_streaming`](Self::upload_file_concurrent_streaming)
    /// but uses [`PipelinedSender`](crate::client::PipelinedSender) connections
    /// instead of blocking [`DcConnection`](crate::dc_pool::DcConnection)s.
    ///
    /// Each worker keeps up to `transfer_limits.upload_pipeline_depth` (X)
    /// `SaveFilePart`/`SaveBigFilePart` requests in flight on its single
    /// connection at once, instead of awaiting each part's ack before reading
    /// and sending the next (X=1, what `upload_file_concurrent_streaming`
    /// does). Combined with `n_workers` separate connections (Y), total
    /// in-flight chunks for an upload is `n_workers * upload_pipeline_depth`
    /// instead of just `n_workers`. See [`TransferLimits`](crate::TransferLimits)
    /// for the highway/trucks model this implements.
    ///
    /// Falls back to the existing non-pipelined path if pipelined connections
    /// fail to open, same reliability guarantees either way.
    pub(crate) async fn upload_file_concurrent_streaming_pipelined(
        &self,
        path: &std::path::Path,
        name: &str,
        mime_type: &str,
        handle: Option<&crate::transfer::TransferHandle>,
    ) -> Result<UploadedFile, InvocationError> {
        use tokio::io::{AsyncReadExt, AsyncSeekExt};

        let meta = tokio::fs::metadata(path)
            .await
            .map_err(InvocationError::Io)?;
        let total = meta.len() as usize;
        if total == 0 {
            return Err(InvocationError::Deserialize(
                "cannot upload empty file".into(),
            ));
        }

        let (part_size, total_parts) = upload_part_size(total);
        let big = total > BIG_FILE_THRESHOLD;
        let n_workers = if self.inner.transfer_limits.bypass_tcp_allotments {
            self.inner.transfer_limits.upload_tcp_connections
        } else {
            upload_worker_count(total, self.inner.transfer_limits.upload_tcp_connections)
        };

        let home_dc = *self.inner.home_dc_id.lock().await;
        let started = std::time::Instant::now();
        let total_mib = total as f64 / (1024.0 * 1024.0);
        tracing::info!(
            "[ferogram::transfer] pipelined upload starting: '{}' ({:.1} MiB / {} bytes, {} parts x {}B, DC{}, Y={} connections, X={} in-flight)",
            name,
            total_mib,
            total,
            total_parts,
            part_size,
            home_dc,
            n_workers,
            self.inner.transfer_limits.upload_pipeline_depth
        );

        if let Some(h) = handle {
            h.set_total(total as u64);
            h.reset_start();
        }
        let shared_handle: Option<crate::transfer::TransferHandle> = handle.cloned();

        let _global_guard = self
            .inner
            .worker_semaphore
            .acquire_many(n_workers as u32)
            .await
            .expect("worker semaphore unexpectedly closed");

        // Single-worker small files: skip pipelined setup overhead entirely.
        if n_workers == 1 {
            drop(_global_guard);
            let mut data = Vec::with_capacity(total);
            tokio::fs::File::open(path)
                .await
                .map_err(InvocationError::Io)?
                .read_to_end(&mut data)
                .await
                .map_err(InvocationError::Io)?;
            return self
                .upload_bytes(&data, name, mime_type, shared_handle.as_ref())
                .await;
        }

        let file_id_atomic =
            std::sync::Arc::new(std::sync::atomic::AtomicI64::new(crate::random_i64_pub()));
        let upload_dc = Arc::new(AtomicI32::new(0i32));

        let mut open_set: tokio::task::JoinSet<
            Result<crate::client::PipelinedSender, InvocationError>,
        > = tokio::task::JoinSet::new();
        for _ in 0..n_workers {
            let client = self.clone();
            open_set.spawn(async move { client.open_worker_sender(0).await });
        }
        let mut senders: Vec<crate::client::PipelinedSender> = Vec::with_capacity(n_workers);
        while let Some(res) = open_set.join_next().await {
            match res {
                Ok(Ok(s)) => senders.push(s),
                Ok(Err(e)) => tracing::debug!(
                    "[ferogram::transfer] pipelined upload worker connection failed: {e}"
                ),
                Err(e) => tracing::debug!(
                    "[ferogram::transfer] pipelined upload worker task panicked: {e}"
                ),
            }
        }
        if senders.is_empty() {
            tracing::debug!(
                "[ferogram::transfer] no pipelined worker connections available; falling back to non-pipelined concurrent upload"
            );
            drop(_global_guard);
            return self
                .upload_file_concurrent_streaming(path, name, mime_type, handle)
                .await;
        }
        let actual_workers = senders.len();

        let next_part = Arc::new(Mutex::new(0i32));
        let mut tasks: tokio::task::JoinSet<Result<(), InvocationError>> =
            tokio::task::JoinSet::new();
        let path_arc = std::sync::Arc::new(path.to_path_buf());

        for sender in senders {
            let next_part = Arc::clone(&next_part);
            let client = self.clone();
            let upload_dc = Arc::clone(&upload_dc);
            let file_id_atomic = std::sync::Arc::clone(&file_id_atomic);
            let worker_handle = shared_handle.clone();
            let path_arc = std::sync::Arc::clone(&path_arc);
            let mut sender = sender;

            tasks.spawn(async move {
                const MAX_WORKER_RECONNECTS: u8 = 5;
                let mut total_reconnects = 0u8;
                let mut worker_dc = 0i32;

                let mut file = tokio::fs::File::open(&*path_arc)
                    .await
                    .map_err(InvocationError::Io)?;

                // Sliding window of (part_num, chunk_len, response_future).
                // Each future resolves to the raw `Bool` RPC response  - upload
                // doesn't need to inspect the payload, just that it succeeded.
                let mut window: std::collections::VecDeque<PipelinedUploadSlot> =
                    std::collections::VecDeque::with_capacity(
                        client.inner.transfer_limits.upload_pipeline_depth,
                    );

                loop {
                    // Top up the window to upload_pipeline_depth before draining the front.
                    while window.len() < client.inner.transfer_limits.upload_pipeline_depth {
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
                            sender = match client.open_worker_sender(worker_dc).await {
                                Ok(s) => s,
                                Err(e) => return Err(e),
                            };
                            file = tokio::fs::File::open(&*path_arc)
                                .await
                                .map_err(InvocationError::Io)?;
                            // A redirect invalidates any parts still in flight from the
                            // old file_id/DC; the window was for the old upload session.
                            window.clear();
                        }

                        let start = part_num as u64 * part_size as u64;
                        let end = (start + part_size as u64).min(total as u64);
                        let chunk_len = (end - start) as usize;

                        file.seek(std::io::SeekFrom::Start(start))
                            .await
                            .map_err(InvocationError::Io)?;
                        let mut bytes = vec![0u8; chunk_len];
                        file.read_exact(&mut bytes)
                            .await
                            .map_err(InvocationError::Io)?;

                        if let Some(ref h) = worker_handle {
                            h.poll_pause_cancel().await?;
                        }

                        let raw_req = if big {
                            tl::Serializable::to_bytes(&tl::functions::upload::SaveBigFilePart {
                                file_id,
                                file_part: part_num,
                                file_total_parts: total_parts,
                                bytes,
                            })
                        } else {
                            tl::Serializable::to_bytes(&tl::functions::upload::SaveFilePart {
                                file_id,
                                file_part: part_num,
                                bytes,
                            })
                        };
                        let body = ferogram_connect::util::maybe_gz_pack(&raw_req);

                        match sender.enqueue(body).await {
                            Ok(fut) => {
                                window.push_back((part_num, chunk_len as u64, Box::pin(fut)))
                            }
                            Err(e) => {
                                tracing::debug!(
                                    "[ferogram::transfer] pipelined upload enqueue failed, reconnecting: {e}"
                                );
                                total_reconnects += 1;
                                if total_reconnects >= MAX_WORKER_RECONNECTS {
                                    return Err(e);
                                }
                                let backoff_ms = 300u64 * (1u64 << (total_reconnects - 1));
                                tokio::time::sleep(std::time::Duration::from_millis(backoff_ms))
                                    .await;
                                sender = match client.open_worker_sender(worker_dc).await {
                                    Ok(s) => s,
                                    Err(e) => return Err(e),
                                };
                                // Re-queue this part: rewind past it and anything else
                                // still in the window before clearing.
                                let min_part = window
                                    .iter()
                                    .map(|(p, _, _)| *p)
                                    .min()
                                    .unwrap_or(part_num)
                                    .min(part_num);
                                window.clear();
                                let mut g = next_part.lock().await;
                                *g = (*g).min(min_part);
                                break;
                            }
                        }
                    }

                    if window.is_empty() {
                        // No more parts to claim and nothing in flight: this worker is done.
                        break;
                    }

                    let (part_num, chunk_len, fut) =
                        window.pop_front().expect("window checked non-empty above");
                    let result = fut.await;
                    let err = match result {
                        Ok(_) => {
                            if let Some(ref h) = worker_handle {
                                h.add_bytes(chunk_len);
                            }
                            continue;
                        }
                        Err(e) => e,
                    };

                    if let InvocationError::Rpc(ref rpc) = err {
                        if rpc.code == 420 {
                            let secs = rpc.value.unwrap_or(1) as u64;
                            tracing::debug!(
                                "[ferogram::transfer] pipelined upload throttled by FLOOD_WAIT_{secs}; sleeping before retry"
                            );
                            tokio::time::sleep(std::time::Duration::from_secs(secs)).await;
                            // Retry the same part directly  - re-enqueue at front of window.
                            let file_id =
                                file_id_atomic.load(std::sync::atomic::Ordering::Relaxed);
                            let start = part_num as u64 * part_size as u64;
                            let end = (start + part_size as u64).min(total as u64);
                            let len = (end - start) as usize;
                            let mut bytes = vec![0u8; len];
                            file.seek(std::io::SeekFrom::Start(start))
                                .await
                                .map_err(InvocationError::Io)?;
                            file.read_exact(&mut bytes)
                                .await
                                .map_err(InvocationError::Io)?;
                            let raw_req = if big {
                                tl::Serializable::to_bytes(
                                    &tl::functions::upload::SaveBigFilePart {
                                        file_id,
                                        file_part: part_num,
                                        file_total_parts: total_parts,
                                        bytes,
                                    },
                                )
                            } else {
                                tl::Serializable::to_bytes(&tl::functions::upload::SaveFilePart {
                                    file_id,
                                    file_part: part_num,
                                    bytes,
                                })
                            };
                            let body = ferogram_connect::util::maybe_gz_pack(&raw_req);
                            match sender.enqueue(body).await {
                                Ok(retry_fut) => {
                                    window.push_front((part_num, chunk_len, Box::pin(retry_fut)));
                                    continue;
                                }
                                Err(e) => return Err(e),
                            }
                        }
                        if rpc.code == 303 {
                            let new_dc = rpc.value.unwrap_or(1) as i32;
                            tracing::debug!(
                                "[ferogram::transfer] pipelined upload redirected by FILE_MIGRATE to DC{new_dc} (was DC{worker_dc})"
                            );
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
                            window.clear();
                            sender = match client.open_worker_sender(new_dc).await {
                                Ok(s) => s,
                                Err(e) => return Err(e),
                            };
                            file = tokio::fs::File::open(&*path_arc)
                                .await
                                .map_err(InvocationError::Io)?;
                            continue;
                        }
                        if rpc.name == "AUTH_KEY_UNREGISTERED" {
                            tracing::warn!(
                                "[ferogram::transfer] pipelined upload: AUTH_KEY_UNREGISTERED on DC{worker_dc}; re-establishing worker connection (attempt {}/{MAX_WORKER_RECONNECTS})",
                                total_reconnects + 1
                            );
                            total_reconnects += 1;
                            if total_reconnects >= MAX_WORKER_RECONNECTS {
                                return Err(err);
                            }
                            let backoff_ms = 300u64 * (1u64 << (total_reconnects - 1));
                            tokio::time::sleep(std::time::Duration::from_millis(backoff_ms)).await;
                            let min_part = window
                                .iter()
                                .map(|(p, _, _)| *p)
                                .min()
                                .unwrap_or(part_num)
                                .min(part_num);
                            window.clear();
                            let mut g = next_part.lock().await;
                            *g = (*g).min(min_part);
                            drop(g);
                            sender = match client.open_worker_sender(worker_dc).await {
                                Ok(s) => s,
                                Err(e) => return Err(e),
                            };
                            continue;
                        }
                        if rpc.code != -503 {
                            return Err(err);
                        }
                    }

                    total_reconnects += 1;
                    if total_reconnects >= MAX_WORKER_RECONNECTS {
                        return Err(err);
                    }
                    let backoff_ms = 300u64 * (1u64 << (total_reconnects - 1));
                    tracing::warn!(
                        "[ferogram::transfer] pipelined upload worker error ({err}); reconnecting (attempt {total_reconnects}/{MAX_WORKER_RECONNECTS}, backoff {backoff_ms}ms)"
                    );
                    tokio::time::sleep(std::time::Duration::from_millis(backoff_ms)).await;
                    let min_part = window
                        .iter()
                        .map(|(p, _, _)| *p)
                        .min()
                        .unwrap_or(part_num)
                        .min(part_num);
                    window.clear();
                    let mut g = next_part.lock().await;
                    *g = (*g).min(min_part);
                    drop(g);
                    sender = match client.open_worker_sender(worker_dc).await {
                        Ok(s) => s,
                        Err(e) => return Err(e),
                    };
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
        let inner = if big {
            tl::enums::InputFile::Big(tl::types::InputFileBig {
                id: file_id,
                parts: total_parts,
                name: name.to_string(),
            })
        } else {
            let mut data = Vec::with_capacity(total);
            tokio::fs::File::open(path)
                .await
                .map_err(InvocationError::Io)?
                .read_to_end(&mut data)
                .await
                .map_err(InvocationError::Io)?;
            let md5_checksum = format!("{:x}", md5::compute(&data));
            tl::enums::InputFile::InputFile(tl::types::InputFile {
                id: file_id,
                parts: total_parts,
                name: name.to_string(),
                md5_checksum,
            })
        };
        tracing::info!(
            "[ferogram::transfer] pipelined upload complete: '{}' ({:.1} MiB / {} bytes, {} parts x {}B, DC{}, Y={} connections, X={} in-flight, took {:.2}s)",
            name,
            total_mib,
            total,
            total_parts,
            part_size,
            home_dc,
            actual_workers,
            self.inner.transfer_limits.upload_pipeline_depth,
            started.elapsed().as_secs_f64()
        );
        Ok(UploadedFile {
            inner,
            mime_type: resolve_mime(name, mime_type),
            name: name.to_string(),
        })
    }

    // Send

    /// Send a file as a document or photo to a chat.
    pub async fn send_file(
        &self,
        peer: impl Into<crate::PeerRef>,
        media: impl Into<tl::enums::InputMedia>,
        msg: &crate::InputMessage,
    ) -> Result<crate::update::IncomingMessage, InvocationError> {
        let media = media.into();
        let peer = peer.into().resolve(self).await?;
        let input_peer = self.inner.peer_cache.read().await.peer_to_input(&peer)?;
        // Same conversion send_message does: a bare MentionName entity has no
        // access_hash, so Telegram drops it. Resolve it to InputMessageEntityMentionName
        // first, or markdown/html user-mention links render as plain text.
        let entities = self.resolve_outgoing_entities(msg.entities.clone()).await;
        let req = tl::functions::messages::SendMedia {
            silent: msg.silent,
            background: msg.background,
            clear_draft: msg.clear_draft,
            noforwards: false,
            update_stickersets_order: false,
            invert_media: msg.invert_media,
            allow_paid_floodskip: false,
            peer: input_peer,
            reply_to: msg.reply_header(),
            media,
            message: msg.text.clone(),
            random_id: crate::random_i64_pub(),
            reply_markup: msg.reply_markup.clone(),
            entities,
            schedule_date: msg.schedule_date,
            schedule_repeat_period: None,
            send_as: None,
            quick_reply_shortcut: None,
            effect: None,
            allow_paid_stars: None,
            suggested_post: None,
        };
        let body: Vec<u8> = self.rpc_call_raw(&req).await?;
        Ok(self.parse_send_response(&body, msg, &peer).await)
    }

    /// Send multiple files as an album.
    ///
    /// Each [`AlbumItem`] carries its own media, caption, entities (formatting),
    /// and optional `reply_to` message ID.
    ///
    /// ```rust,no_run
    /// use ferogram::media::AlbumItem;
    /// # use ferogram::Client;
    /// # async fn example(client: Client, peer: ferogram::PeerRef, photo_media: ferogram::tl::enums::InputMedia, video_media: ferogram::tl::enums::InputMedia, photo_media2: ferogram::tl::enums::InputMedia) -> Result<(), ferogram::InvocationError> {
    ///
    /// let msgs = client.send_album(peer.clone(), vec![
    ///     AlbumItem::new(photo_media).caption_html("<b>First photo</b>"),
    ///     AlbumItem::new(video_media).caption("Second item").reply_to(Some(42)),
    /// ]).await?;
    ///
    /// // Shorthand: legacy tuple API still works via From impl
    /// client.send_album(peer, vec![
    ///     (photo_media2, "caption".to_string()).into(),
    /// ]).await?;
    /// # Ok(()) }
    /// ```
    pub async fn send_album(
        &self,
        peer: impl Into<crate::PeerRef>,
        items: Vec<AlbumItem>,
    ) -> Result<Vec<crate::update::IncomingMessage>, InvocationError> {
        let peer = peer.into().resolve(self).await?;
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

        // Same conversion send_message/send_file do: a bare MentionName entity has
        // no access_hash, so Telegram drops it silently. Resolve per item since
        // each item in an album can carry its own caption and entities.
        let mut multi: Vec<tl::enums::InputSingleMedia> = Vec::with_capacity(items.len());
        for item in items {
            let entities = if item.entities.is_empty() {
                None
            } else {
                self.resolve_outgoing_entities(Some(item.entities)).await
            };
            multi.push(tl::enums::InputSingleMedia::InputSingleMedia(
                tl::types::InputSingleMedia {
                    media: item.media,
                    random_id: crate::random_i64_pub(),
                    message: item.caption,
                    entities,
                },
            ));
        }

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
        let body: Vec<u8> = self.rpc_call_raw(&req).await?;

        // Parse the Updates container and collect all sent messages.
        let mut out = Vec::new();
        if body.len() >= 4 {
            let cid = u32::from_le_bytes(body[..4].try_into().unwrap());
            if cid == 0x74ae4240 || cid == 0x725b04c3 {
                let updates_opt = match tl::enums::Updates::from_bytes_exact(&body) {
                    Ok(updates) => Some(updates),
                    Err(e) => {
                        tracing::warn!(
                            "[ferogram::transfer] failed to parse server response as an Updates frame: {e}"
                        );
                        None
                    }
                };
                let (raw_updates, users, chats) = match updates_opt {
                    Some(tl::enums::Updates::Updates(u)) => (u.updates, u.users, u.chats),
                    Some(tl::enums::Updates::Combined(u)) => (u.updates, u.users, u.chats),
                    _ => (vec![], vec![], vec![]),
                };
                self.cache_users_and_chats(&users, &chats).await;
                for upd in raw_updates {
                    match upd {
                        tl::enums::Update::NewMessage(u) => {
                            out.push(
                                crate::update::IncomingMessage::from_raw(u.message)
                                    .with_client(self.clone()),
                            );
                        }
                        tl::enums::Update::NewChannelMessage(u) => {
                            out.push(
                                crate::update::IncomingMessage::from_raw(u.message)
                                    .with_client(self.clone()),
                            );
                        }
                        _ => {}
                    }
                }
            }
        }
        Ok(out)
    }

    // Download

    /// Create a sequential chunk download iterator.
    ///
    /// `dc_id` must be the DC that stores the file (`Document::dc_id()` /
    /// `Photo::dc_id()`). Pass `0` to use the home DC (bots only).
    #[allow(dead_code)]
    pub(crate) fn iter_download_raw(&self, location: tl::enums::InputFileLocation) -> DownloadIter {
        self.iter_download_on_dc(location, 0)
    }

    /// Like [`iter_download_raw`] but routes to a specific DC.
    #[allow(dead_code)]
    pub(crate) fn iter_download_on_dc(
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
            conn: None,
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
    #[allow(dead_code)]
    pub(crate) async fn download_media_bytes(
        &self,
        location: tl::enums::InputFileLocation,
    ) -> Result<Vec<u8>, InvocationError> {
        self.download_media_on_dc(location, 0).await
    }

    /// Like [`download_media_bytes`] but routes `GetFile` to `dc_id`.
    ///
    /// Opens a **dedicated** `DcConnection` for this download so it never
    /// shares the idle transfer-pool connection (which the server silently
    /// closes after ~90 s of inactivity, causing early-eof on the next use).
    ///
    /// Full AUTH_KEY_UNREGISTERED + FILE_MIGRATE recovery,
    /// the resilience of the concurrent worker path.
    #[allow(dead_code)]
    pub(crate) async fn download_media_on_dc(
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
            {
                let _g: tokio::sync::MutexGuard<'_, i32> = self.inner.home_dc_id.lock().await;
                *_g
            }
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
                        "[ferogram::transfer] sequential download redirected by FILE_MIGRATE to DC{new_dc}"
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
                        "[ferogram::transfer] sequential download: AUTH_KEY_UNREGISTERED on DC{worker_dc}; \
                         re-establishing connection (attempt {reopen_attempts}/{MAX_REOPEN})"
                    );
                    // Evict the cached foreign key so open_worker_conn does a
                    // fresh DH + import instead of reusing the dead key again.
                    {
                        let mut opts: tokio::sync::MutexGuard<
                            '_,
                            std::collections::HashMap<i32, DcEntry>,
                        > = self.inner.dc_options.lock().await;
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

    /// Stream-download sequential path: writes chunks directly to `writer` without
    /// buffering the whole file. Returns total bytes written.
    ///
    /// All retry / DC-migration logic mirrors [`download_media_on_dc`].
    pub(crate) async fn download_streaming_on_dc<W: tokio::io::AsyncWrite + Unpin>(
        &self,
        location: tl::enums::InputFileLocation,
        dc_id: i32,
        writer: &mut W,
        handle: Option<&crate::transfer::TransferHandle>,
    ) -> Result<u64, InvocationError> {
        self.download_streaming_on_dc_from(location, dc_id, writer, handle, 0)
            .await
    }

    /// Like [`download_streaming_on_dc`] but starts at `start_offset` bytes.
    ///
    /// Used by resumable downloads to skip already-received bytes.
    /// `start_offset` is aligned down to the nearest 1 MB boundary as Telegram requires.
    pub(crate) async fn download_streaming_on_dc_from<W: tokio::io::AsyncWrite + Unpin>(
        &self,
        location: tl::enums::InputFileLocation,
        dc_id: i32,
        writer: &mut W,
        handle: Option<&crate::transfer::TransferHandle>,
        start_offset: i64,
    ) -> Result<u64, InvocationError> {
        use tokio::io::AsyncWriteExt;
        let chunk = 512 * 1024i32;
        let mut worker_dc = if dc_id == 0 {
            let _g = self.inner.home_dc_id.lock().await;
            *_g
        } else {
            dc_id
        };
        let mut conn = self.open_worker_conn(worker_dc).await?;
        // Align start_offset down to the nearest 1 MB boundary (Telegram requirement).
        let mb = 1024 * 1024i64;
        let mut offset = (start_offset / mb) * mb;
        let mut total_written = 0u64;
        let mut reopen_attempts = 0u8;
        const MAX_REOPEN: u8 = 3;

        loop {
            if let Some(h) = handle {
                h.poll_pause_cancel().await?;
            }
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
                            reopen_attempts = 0;
                            let done = (f.bytes.len() as i32) < chunk;
                            let n = f.bytes.len() as u64;
                            writer
                                .write_all(&f.bytes)
                                .await
                                .map_err(InvocationError::Io)?;
                            total_written += n;
                            if let Some(h) = handle {
                                h.add_bytes(n);
                            }
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
                    let new_dc = rpc.value.unwrap_or(0) as i32;
                    if new_dc == 0 || new_dc == worker_dc {
                        return Err(InvocationError::Rpc(rpc.clone()));
                    }
                    worker_dc = new_dc;
                    conn = self.open_worker_conn(worker_dc).await?;
                }
                Err(InvocationError::Rpc(ref rpc)) if rpc.name == "AUTH_KEY_UNREGISTERED" => {
                    reopen_attempts += 1;
                    if reopen_attempts > MAX_REOPEN {
                        return Err(InvocationError::Rpc(rpc.clone()));
                    }
                    {
                        let mut opts = self.inner.dc_options.lock().await;
                        if let Some(e) = opts.get_mut(&worker_dc) {
                            e.auth_key = None;
                        }
                    }
                    conn = self.open_worker_conn(worker_dc).await?;
                }
                Err(e) => return Err(e),
            }
        }
        writer.flush().await.map_err(InvocationError::Io)?;
        Ok(total_written)
    }

    /// Download a file using parallel sessions.
    ///
    /// `size` must be the exact byte size of the file.
    ///
    /// Returns the full file bytes in order.
    #[allow(dead_code)]
    pub(crate) async fn download_media_concurrent(
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
    #[allow(dead_code)]
    pub(crate) async fn download_media_concurrent_on_dc(
        &self,
        location: tl::enums::InputFileLocation,
        size: usize,
        dc_id: i32,
    ) -> Result<Vec<u8>, InvocationError> {
        let chunk = download_chunk_size(size) as usize; // 256 KB (<50 MB) or 512 KB (≥50 MB)
        let n_parts = size.div_ceil(chunk);
        // Per-file ceiling: transfer_limits.download_tcp_connections (clamped to MAX_WORKERS_PER_FILE).
        // Global ceiling: MAX_GLOBAL_SENDERS = 12 (enforced via semaphore).
        let n_workers = if self.inner.transfer_limits.bypass_tcp_allotments {
            self.inner.transfer_limits.download_tcp_connections
        } else {
            download_worker_count(size, self.inner.transfer_limits.download_tcp_connections)
        };
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
        let home = {
            let _g: tokio::sync::MutexGuard<'_, i32> = self.inner.home_dc_id.lock().await;
            *_g
        };
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
                Ok(Err(e)) => {
                    tracing::debug!("[ferogram::transfer] download worker connection failed: {e}")
                }
                Err(e) => {
                    tracing::debug!("[ferogram::transfer] download worker task panicked: {e}")
                }
            }
        }
        if conns.is_empty() {
            tracing::debug!(
                "[ferogram::transfer] no worker connections available; downloading sequentially"
            );
            return self.download_media_on_dc(location, dc_id).await;
        }

        let next_part = Arc::new(Mutex::new(0usize));
        // Bounded by n_workers * 2: each worker sends one chunk then immediately
        // fetches the next part, so at most n_workers chunks are in-flight at any
        // time. The ×2 headroom prevents a slow consumer from stalling all workers.
        let (tx, mut rx) = tokio::sync::mpsc::channel::<(usize, Vec<u8>)>(conns.len() * 2);
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
                                tracing::debug!("[ferogram::transfer] download throttled by FLOOD_WAIT_{secs}; sleeping before retry"
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
                                tracing::debug!("[ferogram::transfer] download redirected by FILE_MIGRATE to DC{new_dc} (was DC{worker_dc})"
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
                                    "[ferogram::transfer] download: AUTH_KEY_UNREGISTERED on DC{worker_dc}; re-establishing worker connection (attempt {}/{MAX_WORKER_RECONNECTS})",
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
                            "[ferogram::transfer] download worker error ({err}); reconnecting (attempt {total_reconnects}/{MAX_WORKER_RECONNECTS}, backoff {backoff_ms}ms)"
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
                            // Bounded send: blocks until the collector has space.
                            // If the receiver is dropped (abort), treat as fatal.
                            if tx.send((part, f.bytes)).await.is_err() {
                                break;
                            }
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

    /// Parallel download to a file path. Workers write directly to pre-allocated disk space
    /// via seek + write_all; no in-memory assembly. Use this for large file downloads.
    ///
    /// `size` must be the exact byte size of the file (from `size_from_media`).
    pub(crate) async fn download_media_concurrent_on_dc_to_file(
        &self,
        location: tl::enums::InputFileLocation,
        size: usize,
        dc_id: i32,
        path: &std::path::Path,
        handle: Option<&crate::transfer::TransferHandle>,
    ) -> Result<u64, InvocationError> {
        use tokio::io::{AsyncSeekExt, AsyncWriteExt};

        let chunk = download_chunk_size(size) as usize;
        let n_parts = size.div_ceil(chunk);
        let n_workers = if self.inner.transfer_limits.bypass_tcp_allotments {
            self.inner.transfer_limits.download_tcp_connections
        } else {
            download_worker_count(size, self.inner.transfer_limits.download_tcp_connections)
        };

        let home = {
            let _g = self.inner.home_dc_id.lock().await;
            *_g
        };
        let effective_dc = if dc_id == 0 { home } else { dc_id };

        let started = std::time::Instant::now();
        let file_name = path.file_name().and_then(|s| s.to_str()).unwrap_or("?");
        let size_mib = size as f64 / (1024.0 * 1024.0);
        tracing::info!(
            "[ferogram::transfer] download starting: '{}' ({:.1} MiB / {} bytes, {} chunks x {}B, DC{}, Y={} connections)",
            file_name,
            size_mib,
            size,
            n_parts,
            chunk,
            effective_dc,
            n_workers
        );

        if let Some(h) = handle {
            h.set_total(size as u64);
            h.reset_start();
        }

        let _global_guard = self
            .inner
            .worker_semaphore
            .acquire_many(n_workers as u32)
            .await
            .expect("worker semaphore unexpectedly closed");

        // Single-worker small files: skip parallel setup overhead.
        if n_workers == 1 && effective_dc == home {
            drop(_global_guard);
            let mut file = tokio::fs::File::create(path)
                .await
                .map_err(InvocationError::Io)?;
            return self
                .download_streaming_on_dc(location, dc_id, &mut file, handle)
                .await;
        }

        // Pre-allocate the file to avoid fragmentation on disk.
        let file_for_alloc = tokio::fs::File::create(path)
            .await
            .map_err(InvocationError::Io)?;
        file_for_alloc
            .set_len(size as u64)
            .await
            .map_err(InvocationError::Io)?;
        drop(file_for_alloc);

        let mut open_set: tokio::task::JoinSet<
            Result<crate::dc_pool::DcConnection, InvocationError>,
        > = tokio::task::JoinSet::new();
        for _ in 0..n_workers {
            let client = self.clone();
            open_set.spawn(async move { client.open_worker_conn(effective_dc).await });
        }
        let mut conns: Vec<crate::dc_pool::DcConnection> = Vec::with_capacity(n_workers);
        while let Some(res) = open_set.join_next().await {
            match res {
                Ok(Ok(c)) => conns.push(c),
                Ok(Err(e)) => {
                    tracing::debug!("[ferogram::transfer] download worker connection failed: {e}")
                }
                Err(e) => {
                    tracing::debug!("[ferogram::transfer] download worker task panicked: {e}")
                }
            }
        }
        if conns.is_empty() {
            tracing::debug!(
                "[ferogram::transfer] no worker connections available; downloading sequentially"
            );
            let mut file = tokio::fs::OpenOptions::new()
                .write(true)
                .open(path)
                .await
                .map_err(InvocationError::Io)?;
            return self
                .download_streaming_on_dc(location, dc_id, &mut file, handle)
                .await;
        }

        let next_part = Arc::new(Mutex::new(0usize));
        let actual_workers = conns.len();
        let (tx, mut rx) = tokio::sync::mpsc::channel::<(usize, Vec<u8>)>(conns.len() * 2);
        let mut tasks: tokio::task::JoinSet<Result<(), InvocationError>> =
            tokio::task::JoinSet::new();
        let abort = Arc::new(AtomicBool::new(false));

        for mut conn in conns {
            let location = location.clone();
            let next_part = Arc::clone(&next_part);
            let tx = tx.clone();
            let client = self.clone();
            let abort = Arc::clone(&abort);
            let mut worker_dc = effective_dc;

            tasks.spawn(async move {
                const MAX_WORKER_RECONNECTS: u8 = 5;
                let mut total_reconnects = 0u8;

                loop {
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
                        offset: (part * chunk) as i64,
                        limit: chunk as i32,
                    };
                    let raw = loop {
                        let err = match conn.rpc_call(&req).await {
                            Ok(r) => break r,
                            Err(e) => e,
                        };
                        if let InvocationError::Rpc(ref rpc) = err {
                            if rpc.code == 420 {
                                let secs = rpc.value.unwrap_or(1) as u64;
                                tokio::time::sleep(std::time::Duration::from_secs(secs)).await;
                                continue;
                            }
                            if rpc.code == 303 {
                                let new_dc = rpc.value.unwrap_or(1) as i32;
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
                            if rpc.name == "AUTH_KEY_UNREGISTERED" {
                                total_reconnects += 1;
                                if total_reconnects >= MAX_WORKER_RECONNECTS {
                                    abort.store(true, Ordering::Relaxed);
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
                                    Err(e) => {
                                        abort.store(true, Ordering::Relaxed);
                                        return Err(e);
                                    }
                                }
                            }
                            if rpc.code != -503 {
                                abort.store(true, Ordering::Relaxed);
                                return Err(err);
                            }
                        }
                        total_reconnects += 1;
                        if total_reconnects >= MAX_WORKER_RECONNECTS {
                            abort.store(true, Ordering::Relaxed);
                            return Err(err);
                        }
                        let backoff_ms = 300u64 * (1u64 << (total_reconnects - 1));
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
                            if tx.send((part, f.bytes)).await.is_err() {
                                break;
                            }
                        }
                        tl::enums::upload::File::CdnRedirect(_) => {
                            abort.store(true, Ordering::Relaxed);
                            return Err(InvocationError::Deserialize(
                                "CDN redirect in concurrent download; retry via sequential".into(),
                            ));
                        }
                    }
                }
                Ok(())
            });
        }
        drop(tx);

        // Writer task: single tokio::fs::File, seeks to each chunk offset and writes.
        // This is the only writer - no locking needed.
        let path_owned = path.to_path_buf();
        let shared_handle = handle.cloned();
        let writer_task: tokio::task::JoinHandle<Result<u64, InvocationError>> =
            tokio::spawn(async move {
                let mut file = tokio::fs::OpenOptions::new()
                    .write(true)
                    .open(&path_owned)
                    .await
                    .map_err(InvocationError::Io)?;
                let mut total_written = 0u64;
                while let Some((part, data)) = rx.recv().await {
                    let offset = (part * chunk) as u64;
                    file.seek(std::io::SeekFrom::Start(offset))
                        .await
                        .map_err(InvocationError::Io)?;
                    file.write_all(&data).await.map_err(InvocationError::Io)?;
                    let n = data.len() as u64;
                    total_written += n;
                    if let Some(ref h) = shared_handle {
                        h.add_bytes(n);
                    }
                }
                file.flush().await.map_err(InvocationError::Io)?;
                Ok(total_written)
            });

        while let Some(res) = tasks.join_next().await {
            if let Err(e) =
                res.map_err(|e| InvocationError::Io(std::io::Error::other(e.to_string())))?
            {
                tasks.abort_all();
                writer_task.abort();
                return Err(e);
            }
        }

        let total_written = writer_task
            .await
            .map_err(|e| InvocationError::Io(std::io::Error::other(e.to_string())))??;

        tracing::info!(
            "[ferogram::transfer] download complete: '{}' ({:.1} MiB / {} bytes, {} chunks x {}B, DC{}, Y={} connections, took {:.2}s)",
            file_name,
            total_written as f64 / (1024.0 * 1024.0),
            total_written,
            n_parts,
            chunk,
            effective_dc,
            actual_workers,
            started.elapsed().as_secs_f64()
        );
        Ok(total_written)
    }

    /// Like [`download_media_concurrent_on_dc_to_file`](Self::download_media_concurrent_on_dc_to_file)
    /// but uses [`PipelinedSender`](crate::client::PipelinedSender) connections
    /// instead of blocking [`DcConnection`](crate::dc_pool::DcConnection)s.
    ///
    /// Each worker keeps up to `transfer_limits.download_pipeline_depth` (X)
    /// `GetFile` requests in flight on its single connection at once,
    /// instead of sending one and waiting for the response before sending
    /// the next (X=1, what `download_media_concurrent_on_dc_to_file` does).
    /// Combined with `n_workers` separate connections (Y), total in-flight
    /// chunks for a transfer is `n_workers * download_pipeline_depth` instead
    /// of just `n_workers`  - this is the "X pieces in flight, Y queues"
    /// model Telegram's docs recommend for upload/download performance. See
    /// [`TransferLimits`](crate::TransferLimits) for the highway/trucks model.
    ///
    /// Falls back to the existing non-pipelined path internally if pipelined
    /// connections fail to open (e.g. `into_parts`/`spawn_sender_task`
    /// unavailable for some reason), so callers get the same reliability
    /// guarantees either way.
    pub(crate) async fn download_media_concurrent_on_dc_to_file_pipelined(
        &self,
        location: tl::enums::InputFileLocation,
        size: usize,
        dc_id: i32,
        path: &std::path::Path,
        handle: Option<&crate::transfer::TransferHandle>,
    ) -> Result<u64, InvocationError> {
        use tokio::io::{AsyncSeekExt, AsyncWriteExt};

        let chunk = download_chunk_size(size) as usize;
        let n_parts = size.div_ceil(chunk);
        let n_workers = if self.inner.transfer_limits.bypass_tcp_allotments {
            self.inner.transfer_limits.download_tcp_connections
        } else {
            download_worker_count(size, self.inner.transfer_limits.download_tcp_connections)
        };

        let home = {
            let _g = self.inner.home_dc_id.lock().await;
            *_g
        };
        let effective_dc = if dc_id == 0 { home } else { dc_id };

        let started = std::time::Instant::now();
        let file_name = path.file_name().and_then(|s| s.to_str()).unwrap_or("?");
        let size_mib = size as f64 / (1024.0 * 1024.0);
        tracing::info!(
            "[ferogram::transfer] pipelined download starting: '{}' ({:.1} MiB / {} bytes, {} chunks x {}B, DC{}, Y={} connections, X={} in-flight)",
            file_name,
            size_mib,
            size,
            n_parts,
            chunk,
            effective_dc,
            n_workers,
            self.inner.transfer_limits.download_pipeline_depth
        );

        if let Some(h) = handle {
            h.set_total(size as u64);
            h.reset_start();
        }

        let _global_guard = self
            .inner
            .worker_semaphore
            .acquire_many(n_workers as u32)
            .await
            .expect("worker semaphore unexpectedly closed");

        // Single-worker small files: skip parallel/pipelined setup overhead
        // entirely  - pipelining only pays off once there's more than one
        // chunk worth queuing.
        if n_workers == 1 && effective_dc == home {
            drop(_global_guard);
            let mut file = tokio::fs::File::create(path)
                .await
                .map_err(InvocationError::Io)?;
            return self
                .download_streaming_on_dc(location, dc_id, &mut file, handle)
                .await;
        }

        // Pre-allocate the file to avoid fragmentation on disk.
        let file_for_alloc = tokio::fs::File::create(path)
            .await
            .map_err(InvocationError::Io)?;
        file_for_alloc
            .set_len(size as u64)
            .await
            .map_err(InvocationError::Io)?;
        drop(file_for_alloc);

        let mut open_set: tokio::task::JoinSet<
            Result<crate::client::PipelinedSender, InvocationError>,
        > = tokio::task::JoinSet::new();
        for _ in 0..n_workers {
            let client = self.clone();
            open_set.spawn(async move { client.open_worker_sender(effective_dc).await });
        }
        let mut senders: Vec<crate::client::PipelinedSender> = Vec::with_capacity(n_workers);
        while let Some(res) = open_set.join_next().await {
            match res {
                Ok(Ok(s)) => senders.push(s),
                Ok(Err(e)) => tracing::debug!(
                    "[ferogram::transfer] pipelined download worker connection failed: {e}"
                ),
                Err(e) => tracing::debug!(
                    "[ferogram::transfer] pipelined download worker task panicked: {e}"
                ),
            }
        }
        if senders.is_empty() {
            tracing::debug!(
                "[ferogram::transfer] no pipelined worker connections available; falling back to non-pipelined concurrent download"
            );
            drop(_global_guard);
            return self
                .download_media_concurrent_on_dc_to_file(location, size, dc_id, path, handle)
                .await;
        }

        let actual_workers = senders.len();
        let next_part = Arc::new(Mutex::new(0usize));
        let (tx, mut rx) = tokio::sync::mpsc::channel::<(usize, Vec<u8>)>(senders.len() * 2);
        let mut tasks: tokio::task::JoinSet<Result<(), InvocationError>> =
            tokio::task::JoinSet::new();
        let abort = Arc::new(AtomicBool::new(false));

        for sender in senders {
            let location = location.clone();
            let next_part = Arc::clone(&next_part);
            let tx = tx.clone();
            let client = self.clone();
            let abort = Arc::clone(&abort);
            let mut worker_dc = effective_dc;
            let mut sender = sender;

            tasks.spawn(async move {
                const MAX_WORKER_RECONNECTS: u8 = 5;
                let mut total_reconnects = 0u8;

                // Sliding window of (part, response_future) pairs, in the
                // order parts were claimed. We resolve them front-to-back so
                // ordering for the writer channel stays deterministic per
                // worker even though responses can arrive out of order on
                // the wire  - `PipelinedSender::enqueue` already matches each
                // future to its own msg_id internally.
                let mut window: std::collections::VecDeque<PipelinedDownloadSlot> =
                    std::collections::VecDeque::with_capacity(
                        client.inner.transfer_limits.download_pipeline_depth,
                    );

                loop {
                    if abort.load(Ordering::Relaxed) {
                        break;
                    }

                    // Top up the window to download_pipeline_depth before draining the front.
                    while window.len() < client.inner.transfer_limits.download_pipeline_depth && !abort.load(Ordering::Relaxed) {
                        let part = {
                            let mut g = next_part.lock().await;
                            if *g >= n_parts {
                                None
                            } else {
                                let p = *g;
                                *g += 1;
                                Some(p)
                            }
                        };
                        let Some(part) = part else {
                            break;
                        };
                        let req = tl::functions::upload::GetFile {
                            precise: true,
                            cdn_supported: false,
                            location: location.clone(),
                            offset: (part * chunk) as i64,
                            limit: chunk as i32,
                        };
                        let body =
                            ferogram_connect::util::maybe_gz_pack(&tl::Serializable::to_bytes(&req));
                        match sender.enqueue(body).await {
                            Ok(fut) => window.push_back((part, Box::pin(fut))),
                            Err(e) => {
                                // Sender task is dead; reconnect and retry filling the window.
                                tracing::debug!(
                                    "[ferogram::transfer] pipelined enqueue failed, reconnecting: {e}"
                                );
                                total_reconnects += 1;
                                if total_reconnects >= MAX_WORKER_RECONNECTS {
                                    abort.store(true, Ordering::Relaxed);
                                    return Err(e);
                                }
                                let backoff_ms = 300u64 * (1u64 << (total_reconnects - 1));
                                tokio::time::sleep(std::time::Duration::from_millis(backoff_ms))
                                    .await;
                                match client.open_worker_sender(worker_dc).await {
                                    Ok(s) => {
                                        sender = s;
                                        // Re-queue this part for the next loop iteration.
                                        let mut g = next_part.lock().await;
                                        *g = (*g).min(part);
                                        drop(g);
                                        break;
                                    }
                                    Err(e) => {
                                        abort.store(true, Ordering::Relaxed);
                                        return Err(e);
                                    }
                                }
                            }
                        }
                    }

                    if window.is_empty() {
                        // No more parts to claim and nothing in flight: this worker is done.
                        break;
                    }

                    let (part, fut) = window.pop_front().expect("window checked non-empty above");
                    let raw = match fut.await {
                        Ok(r) => r,
                        Err(err) => {
                            if let InvocationError::Rpc(ref rpc) = err {
                                if rpc.code == 420 {
                                    let secs = rpc.value.unwrap_or(1) as u64;
                                    tokio::time::sleep(std::time::Duration::from_secs(secs)).await;
                                    // Re-claim this part by decrementing the shared cursor isn't
                                    // safe under concurrent workers; instead, retry the same
                                    // part directly on a fresh enqueue.
                                    let req = tl::functions::upload::GetFile {
                                        precise: true,
                                        cdn_supported: false,
                                        location: location.clone(),
                                        offset: (part * chunk) as i64,
                                        limit: chunk as i32,
                                    };
                                    let body = ferogram_connect::util::maybe_gz_pack(
                                        &tl::Serializable::to_bytes(&req),
                                    );
                                    match sender.enqueue(body).await {
                                        Ok(retry_fut) => {
                                            window.push_front((part, Box::pin(retry_fut)));
                                            continue;
                                        }
                                        Err(e) => {
                                            abort.store(true, Ordering::Relaxed);
                                            return Err(e);
                                        }
                                    }
                                }
                                if rpc.code == 303 {
                                    let new_dc = rpc.value.unwrap_or(1) as i32;
                                    worker_dc = new_dc;
                                    match client.open_worker_sender(new_dc).await {
                                        Ok(s) => {
                                            sender = s;
                                            // Parts still queued in `window` haven't been
                                            // written yet; rewind past the lowest of them
                                            // (not just `part`) before clearing, or they're
                                            // silently dropped and the file ends up with gaps.
                                            let min_part = window
                                                .iter()
                                                .map(|(p, _)| *p)
                                                .min()
                                                .unwrap_or(part)
                                                .min(part);
                                            window.clear();
                                            let mut g = next_part.lock().await;
                                            *g = (*g).min(min_part);
                                            continue;
                                        }
                                        Err(e) => {
                                            abort.store(true, Ordering::Relaxed);
                                            return Err(e);
                                        }
                                    }
                                }
                            }
                            // Connection-level failure (sender task died, etc): reconnect
                            // and let the outer loop re-fill the window from scratch.
                            total_reconnects += 1;
                            if total_reconnects >= MAX_WORKER_RECONNECTS {
                                abort.store(true, Ordering::Relaxed);
                                return Err(err);
                            }
                            let backoff_ms = 300u64 * (1u64 << (total_reconnects - 1));
                            tokio::time::sleep(std::time::Duration::from_millis(backoff_ms)).await;
                            match client.open_worker_sender(worker_dc).await {
                                Ok(s) => {
                                    sender = s;
                                    // Same gap-prevention rewind as the 303 branch above:
                                    // account for everything still queued in `window`.
                                    let min_part = window
                                        .iter()
                                        .map(|(p, _)| *p)
                                        .min()
                                        .unwrap_or(part)
                                        .min(part);
                                    window.clear();
                                    let mut g = next_part.lock().await;
                                    *g = (*g).min(min_part);
                                    continue;
                                }
                                Err(e) => {
                                    abort.store(true, Ordering::Relaxed);
                                    return Err(e);
                                }
                            }
                        }
                    };

                    let mut cur = Cursor::from_slice(&raw);
                    match tl::enums::upload::File::deserialize(&mut cur)? {
                        tl::enums::upload::File::File(f) => {
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
                            if tx.send((part, f.bytes)).await.is_err() {
                                break;
                            }
                        }
                        tl::enums::upload::File::CdnRedirect(_) => {
                            abort.store(true, Ordering::Relaxed);
                            return Err(InvocationError::Deserialize(
                                "CDN redirect in pipelined download; retry via sequential".into(),
                            ));
                        }
                    }
                }
                Ok(())
            });
        }
        drop(tx);

        // Writer task: single tokio::fs::File, seeks to each chunk offset and writes.
        let path_owned = path.to_path_buf();
        let shared_handle = handle.cloned();
        let writer_task: tokio::task::JoinHandle<Result<u64, InvocationError>> =
            tokio::spawn(async move {
                let mut file = tokio::fs::OpenOptions::new()
                    .write(true)
                    .open(&path_owned)
                    .await
                    .map_err(InvocationError::Io)?;
                let mut total_written = 0u64;
                while let Some((part, data)) = rx.recv().await {
                    let offset = (part * chunk) as u64;
                    file.seek(std::io::SeekFrom::Start(offset))
                        .await
                        .map_err(InvocationError::Io)?;
                    file.write_all(&data).await.map_err(InvocationError::Io)?;
                    let n = data.len() as u64;
                    total_written += n;
                    if let Some(ref h) = shared_handle {
                        h.add_bytes(n);
                    }
                }
                file.flush().await.map_err(InvocationError::Io)?;
                Ok(total_written)
            });

        while let Some(res) = tasks.join_next().await {
            if let Err(e) =
                res.map_err(|e| InvocationError::Io(std::io::Error::other(e.to_string())))?
            {
                tasks.abort_all();
                writer_task.abort();
                return Err(e);
            }
        }

        let total_written = writer_task
            .await
            .map_err(|e| InvocationError::Io(std::io::Error::other(e.to_string())))??;

        tracing::info!(
            "[ferogram::transfer] pipelined download complete: '{}' ({:.1} MiB / {} bytes, {} chunks x {}B, DC{}, Y={} connections, X={} in-flight, took {:.2}s)",
            file_name,
            total_written as f64 / (1024.0 * 1024.0),
            total_written,
            n_parts,
            chunk,
            effective_dc,
            actual_workers,
            self.inner.transfer_limits.download_pipeline_depth,
            started.elapsed().as_secs_f64()
        );
        Ok(total_written)
    }

    /// Download any [`Downloadable`] item (internal).
    ///
    /// Public API: use [`Client::download`] with `&MessageMedia`.
    #[allow(dead_code)]
    pub(crate) async fn download_item<D: Downloadable>(
        &self,
        item: &D,
    ) -> Result<Vec<u8>, InvocationError> {
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

    /// Like [`Self::download_location`] but also returns the file's DC id.
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

    /// Download this message's media to any [`AsyncWrite`] sink. Returns bytes written.
    ///
    /// Requires the message to have an attached client (i.e. it came from a handler).
    ///
    /// [`AsyncWrite`]: tokio::io::AsyncWrite
    ///
    /// # Example
    /// ```rust,no_run
    /// # use ferogram::update::IncomingMessage;
    /// # async fn ex(msg: IncomingMessage) {
    /// let mut buf = Vec::new();
    /// msg.download(&mut buf).await.unwrap();
    /// # }
    /// ```
    pub async fn download(
        &self,
        dest: impl tokio::io::AsyncWrite + Unpin,
    ) -> Result<u64, crate::InvocationError> {
        let client = self.require_client("download")?.clone();
        let media = match &self.raw {
            tl::enums::Message::Message(m) => m.media.as_ref().ok_or_else(|| {
                crate::InvocationError::Deserialize("message has no media".into())
            })?,
            _ => {
                return Err(crate::InvocationError::Deserialize(
                    "not a regular message".into(),
                ));
            }
        };
        client.download(media, dest, None).await
    }

    /// Download this message's media into memory and return the raw bytes.
    ///
    /// Convenience wrapper over [`download`] for small files. For large files
    /// prefer [`download`] with a [`tokio::fs::File`] to avoid memory pressure.
    ///
    /// [`download`]: crate::update::IncomingMessage::download
    pub async fn bytes(&self) -> Result<Vec<u8>, crate::InvocationError> {
        let mut buf = Vec::new();
        self.download(&mut buf).await?;
        Ok(buf)
    }
}

/// Extract a download `InputFileLocation` and DC id from a raw `MessageMedia`.
///
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

    /// Like `upload_file_concurrent_streaming` but with caller-supplied worker count
    /// and chunk size. Used by `upload_exp`.
    #[cfg(feature = "experimental")]
    pub(crate) async fn upload_file_concurrent_streaming_exp(
        &self,
        path: &std::path::Path,
        n_workers: usize,
        chunk_size: usize,
        handle: Option<&crate::transfer::TransferHandle>,
    ) -> Result<UploadedFile, InvocationError> {
        use tokio::io::{AsyncReadExt, AsyncSeekExt};

        let meta = tokio::fs::metadata(path)
            .await
            .map_err(InvocationError::Io)?;
        let total = meta.len() as usize;
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("file");
        let big = total > BIG_FILE_THRESHOLD;
        let total_parts = total.div_ceil(chunk_size) as i32;
        let file_id = crate::random_i64_pub();

        // Sniff MIME from header bytes.
        let mut header_f = tokio::fs::File::open(path)
            .await
            .map_err(InvocationError::Io)?;
        let mut header = vec![0u8; chunk_size.min(65536)];
        let n = header_f
            .read(&mut header)
            .await
            .map_err(InvocationError::Io)?;
        header.truncate(n);
        let mime_type = detect_mime_from_bytes(&header, name);
        drop(header_f);

        if let Some(h) = handle {
            h.set_total(total as u64);
            h.reset_start();
        }

        let _global_guard = self
            .inner
            .worker_semaphore
            .acquire_many(n_workers as u32)
            .await
            .expect("worker semaphore unexpectedly closed");

        let next_part = Arc::new(Mutex::new(0i32));
        let shared_handle: Option<crate::transfer::TransferHandle> = handle.cloned();
        let mut tasks: tokio::task::JoinSet<Result<(), InvocationError>> =
            tokio::task::JoinSet::new();

        for _ in 0..n_workers {
            let client = self.clone();
            let next_part = Arc::clone(&next_part);
            let worker_handle = shared_handle.clone();
            let path = path.to_path_buf();

            tasks.spawn(async move {
                let mut conn = client.open_worker_conn(0).await?;
                let mut f = tokio::fs::File::open(&path)
                    .await
                    .map_err(InvocationError::Io)?;

                loop {
                    let part_num = {
                        let mut g = next_part.lock().await;
                        if *g >= total_parts {
                            break;
                        }
                        let n = *g;
                        *g += 1;
                        n
                    };

                    if let Some(ref h) = worker_handle {
                        h.poll_pause_cancel().await?;
                    }

                    let offset = part_num as u64 * chunk_size as u64;
                    f.seek(std::io::SeekFrom::Start(offset))
                        .await
                        .map_err(InvocationError::Io)?;
                    let mut buf = vec![0u8; chunk_size];
                    let mut bytes_read = 0;
                    while bytes_read < chunk_size {
                        match f
                            .read(&mut buf[bytes_read..])
                            .await
                            .map_err(InvocationError::Io)?
                        {
                            0 => break,
                            n => bytes_read += n,
                        }
                    }
                    if bytes_read == 0 {
                        break;
                    }
                    let bytes = buf[..bytes_read].to_vec();
                    let chunk_len = bytes.len() as u64;

                    if big {
                        conn.rpc_call(&tl::functions::upload::SaveBigFilePart {
                            file_id,
                            file_part: part_num,
                            file_total_parts: total_parts,
                            bytes,
                        })
                        .await?;
                    } else {
                        conn.rpc_call(&tl::functions::upload::SaveFilePart {
                            file_id,
                            file_part: part_num,
                            bytes,
                        })
                        .await?;
                    }

                    if let Some(ref h) = worker_handle {
                        h.add_bytes(chunk_len);
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

        let inner = make_input_file(big, file_id, total_parts, name, &[]);
        tracing::info!(
            "[ferogram::transfer] upload_exp complete: '{}' ({} bytes, {}B chunks x {}, {} workers)",
            name,
            total,
            chunk_size,
            total_parts,
            n_workers
        );
        Ok(UploadedFile {
            inner,
            mime_type,
            name: name.to_string(),
        })
    }

    /// Parallel download with caller-supplied worker count and chunk size.
    /// Used by `download_exp`. Writes assembled bytes into `dest`.
    #[cfg(feature = "experimental")]
    pub(crate) async fn download_concurrent_exp(
        &self,
        location: tl::enums::InputFileLocation,
        dc_id: i32,
        size: usize,
        dest: &mut Vec<u8>,
        n_workers: usize,
        chunk_size: i32,
        handle: Option<&crate::transfer::TransferHandle>,
    ) -> Result<u64, InvocationError> {
        use tokio::io::AsyncWriteExt;

        let n_parts = size.div_ceil(chunk_size as usize);

        // Pre-allocate destination buffer.
        dest.resize(size, 0u8);
        let dest_arc = Arc::new(tokio::sync::Mutex::new(dest));

        let _global_guard = self
            .inner
            .worker_semaphore
            .acquire_many(n_workers as u32)
            .await
            .expect("worker semaphore unexpectedly closed");

        let next_part = Arc::new(Mutex::new(0usize));
        let shared_handle: Option<crate::transfer::TransferHandle> = handle.cloned();
        let mut tasks: tokio::task::JoinSet<Result<(), InvocationError>> =
            tokio::task::JoinSet::new();

        for _ in 0..n_workers {
            let client = self.clone();
            let location = location.clone();
            let next_part = Arc::clone(&next_part);
            let dest_arc = Arc::clone(&dest_arc);
            let worker_handle = shared_handle.clone();

            tasks.spawn(async move {
                let mut conn = client.open_worker_conn(dc_id).await?;

                loop {
                    let part_num = {
                        let mut g = next_part.lock().await;
                        if *g >= n_parts {
                            break;
                        }
                        let n = *g;
                        *g += 1;
                        n
                    };

                    if let Some(ref h) = worker_handle {
                        h.poll_pause_cancel().await?;
                    }

                    let offset = part_num as i64 * chunk_size as i64;
                    let req = tl::functions::upload::GetFile {
                        precise: true,
                        cdn_supported: false,
                        location: location.clone(),
                        offset,
                        limit: chunk_size,
                    };
                    let raw = conn.rpc_call(&req).await?;
                    let mut cur = Cursor::from_slice(&raw);
                    let bytes = match tl::enums::upload::File::deserialize(&mut cur)? {
                        tl::enums::upload::File::File(f) => f.bytes,
                        tl::enums::upload::File::CdnRedirect(_) => {
                            return Err(InvocationError::Deserialize(
                                "CDN redirect not supported in download_exp".into(),
                            ));
                        }
                    };

                    let start = part_num * chunk_size as usize;
                    let end = (start + bytes.len()).min(size);
                    {
                        let mut d = dest_arc.lock().await;
                        d[start..end].copy_from_slice(&bytes[..end - start]);
                    }

                    if let Some(ref h) = worker_handle {
                        h.add_bytes(bytes.len() as u64);
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

        let n = size as u64;
        tracing::info!(
            "[ferogram::transfer] download_exp complete: {} bytes, {} workers, {}B chunks",
            n,
            n_workers,
            chunk_size
        );
        Ok(n)
    }
}

/// Resolve a [`tl::enums::MessageMedia`] to its download location + DC.
///
/// Returns `None` if the media variant has no downloadable file.
pub fn location_from_media(
    media: &tl::enums::MessageMedia,
) -> Option<(tl::enums::InputFileLocation, i32)> {
    if let Some(doc) = Document::from_media(media) {
        return Some((doc.to_input_location()?, doc.dc_id()));
    }
    if let Some(photo) = Photo::from_media(media) {
        return Some((photo.to_input_location()?, photo.dc_id()));
    }
    None
}

/// Return the known byte size of `media`, if available.
pub fn size_from_media(media: &tl::enums::MessageMedia) -> Option<usize> {
    if let Some(doc) = Document::from_media(media) {
        return Some(doc.raw.size as usize);
    }
    if let Some(photo) = Photo::from_media(media) {
        let sz = photo
            .raw
            .sizes
            .iter()
            .filter_map(|s| match s {
                tl::enums::PhotoSize::PhotoSize(ps) => Some(ps.size as usize),
                tl::enums::PhotoSize::Progressive(ps) => ps.sizes.last().map(|&s| s as usize),
                _ => None,
            })
            .max();
        return sz;
    }
    None
}

// Helpers

/// Public wrapper around `make_input_file` for use from `client/files.rs`.
pub fn make_input_file_pub(
    big: bool,
    file_id: i64,
    total_parts: i32,
    name: &str,
    data: &[u8],
) -> tl::enums::InputFile {
    make_input_file(big, file_id, total_parts, name, data)
}

/// Generate a random upload session file_id. Used by experimental resumable upload.
pub fn random_file_id_pub() -> i64 {
    crate::random_i64_pub()
}
