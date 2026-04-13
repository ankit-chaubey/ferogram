// Copyright (c) Ankit Chaubey <ankitchaubey.dev@gmail.com>
// SPDX-License-Identifier: MIT OR Apache-2.0
//
// ferogram: async Telegram MTProto client in Rust
// https://github.com/ankit-chaubey/ferogram
//
// Based on layer: https://github.com/ankit-chaubey/layer
// Follows official Telegram client behaviour (tdesktop, TDLib).
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

/// Chunk size used for uploads and downloads (512 KB).
pub const UPLOAD_CHUNK_SIZE: i32 = 512 * 1024;
pub const DOWNLOAD_CHUNK_SIZE: i32 = 512 * 1024;
/// Files larger than this use `SaveBigFilePart` and the parallel upload path.
const BIG_FILE_THRESHOLD: usize = 10 * 1024 * 1024;

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
/// Number of parallel workers for concurrent transfer.
const WORKER_COUNT: usize = 4;

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
        let body = self.client.rpc_call_raw_pub(&req).await?;
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
            tl::enums::upload::File::CdnRedirect(_) => {
                self.done = true;
                Err(InvocationError::Deserialize(
                    "CDN redirect not supported".into(),
                ))
            }
        }
    }
}

impl Client {
    // Upload

    /// Upload bytes sequentially. For big files (≥ 10 MB) prefer
    /// [`upload_file_concurrent`] which uses parallel workers.
    pub async fn upload_file(
        &self,
        data: &[u8],
        name: &str,
        mime_type: &str,
    ) -> Result<UploadedFile, InvocationError> {
        // auto-detect MIME from filename when caller passes "" or the generic fallback.
        let resolved_mime = resolve_mime(name, mime_type);

        let file_id = crate::random_i64_pub();
        let total = data.len();
        let big = total >= BIG_FILE_THRESHOLD;
        let part_size = UPLOAD_CHUNK_SIZE as usize;
        let total_parts = total.div_ceil(part_size) as i32;

        for (part_num, chunk) in data.chunks(part_size).enumerate() {
            if big {
                self.rpc_call_raw_pub(&tl::functions::upload::SaveBigFilePart {
                    file_id,
                    file_part: part_num as i32,
                    file_total_parts: total_parts,
                    bytes: chunk.to_vec(),
                })
                .await?;
            } else {
                self.rpc_call_raw_pub(&tl::functions::upload::SaveFilePart {
                    file_id,
                    file_part: part_num as i32,
                    bytes: chunk.to_vec(),
                })
                .await?;
            }
        }

        let inner = make_input_file(big, file_id, total_parts, name, data);
        tracing::info!(
            "[ferogram] uploaded '{}' ({} bytes, {} parts, mime={})",
            name,
            total,
            total_parts,
            resolved_mime
        );
        Ok(UploadedFile {
            inner,
            mime_type: resolved_mime,
            name: name.to_string(),
        })
    }

    /// Upload bytes using `WORKER_COUNT` (4) parallel workers.
    ///
    /// Only beneficial for big files (≥ 10 MB).  Falls through to sequential
    /// for small files automatically.
    pub async fn upload_file_concurrent(
        &self,
        data: Arc<Vec<u8>>,
        name: &str,
        mime_type: &str,
    ) -> Result<UploadedFile, InvocationError> {
        let total = data.len();
        let part_size = UPLOAD_CHUNK_SIZE as usize;
        let total_parts = total.div_ceil(part_size) as i32;

        if total < BIG_FILE_THRESHOLD {
            // Not big enough to benefit: fall back to sequential.
            return self.upload_file(&data, name, mime_type).await;
        }

        let file_id = crate::random_i64_pub();
        let next_part = Arc::new(Mutex::new(0i32));
        let mut tasks = tokio::task::JoinSet::new();

        for _ in 0..WORKER_COUNT {
            let client = self.clone();
            let data = Arc::clone(&data);
            let next_part = Arc::clone(&next_part);

            tasks.spawn(async move {
                loop {
                    let part_num = {
                        let mut guard = next_part.lock().await;
                        if *guard >= total_parts {
                            break;
                        }
                        let n = *guard;
                        *guard += 1;
                        n
                    };
                    let start = part_num as usize * part_size;
                    let end = (start + part_size).min(data.len());
                    let bytes = data[start..end].to_vec();

                    client
                        .rpc_call_raw_pub(&tl::functions::upload::SaveBigFilePart {
                            file_id,
                            file_part: part_num,
                            file_total_parts: total_parts,
                            bytes,
                        })
                        .await?;
                }
                Ok::<(), InvocationError>(())
            });
        }

        while let Some(res) = tasks.join_next().await {
            res.map_err(|e| InvocationError::Io(std::io::Error::other(e.to_string())))??;
        }

        let inner = tl::enums::InputFile::Big(tl::types::InputFileBig {
            id: file_id,
            parts: total_parts,
            name: name.to_string(),
        });
        tracing::info!(
            "[ferogram] concurrent-uploaded '{}' ({} bytes, {} parts, {} workers)",
            name,
            total,
            total_parts,
            WORKER_COUNT
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
        if data.len() >= BIG_FILE_THRESHOLD {
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
        let input_peer = self.inner.peer_cache.peer_to_input(&peer);
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
        let input_peer = self.inner.peer_cache.peer_to_input(&peer);

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
    pub fn iter_download(&self, location: tl::enums::InputFileLocation) -> DownloadIter {
        DownloadIter {
            client: self.clone(),
            done: false,
            request: Some(tl::functions::upload::GetFile {
                precise: false,
                cdn_supported: false,
                location,
                offset: 0,
                limit: DOWNLOAD_CHUNK_SIZE,
            }),
        }
    }

    /// Download all bytes of a media attachment at once (sequential).
    pub async fn download_media(
        &self,
        location: tl::enums::InputFileLocation,
    ) -> Result<Vec<u8>, InvocationError> {
        let mut bytes = Vec::new();
        let mut iter = self.iter_download(location);
        while let Some(chunk) = iter.next().await? {
            bytes.extend_from_slice(&chunk);
        }
        Ok(bytes)
    }

    /// Download a file using `WORKER_COUNT` (4) parallel workers.
    ///
    /// `size` must be the exact byte size of the file (obtained from the
    /// [`Downloadable::size`] accessor, or from the document's `size` field).
    ///
    /// Returns the full file bytes in order.
    pub async fn download_media_concurrent(
        &self,
        location: tl::enums::InputFileLocation,
        size: usize,
    ) -> Result<Vec<u8>, InvocationError> {
        let chunk = DOWNLOAD_CHUNK_SIZE as usize;
        let n_parts = size.div_ceil(chunk);
        let next_part = Arc::new(Mutex::new(0usize));

        // Channel: each worker sends (part_index, bytes)
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<(usize, Vec<u8>)>();
        let mut tasks = tokio::task::JoinSet::new();

        for _ in 0..WORKER_COUNT {
            let client = self.clone();
            let location = location.clone();
            let next_part = Arc::clone(&next_part);
            let tx = tx.clone();

            tasks.spawn(async move {
                loop {
                    let part = {
                        let mut g = next_part.lock().await;
                        if *g >= n_parts {
                            break;
                        }
                        let p = *g;
                        *g += 1;
                        p
                    };
                    let offset = (part * chunk) as i64;
                    let req = tl::functions::upload::GetFile {
                        precise: true,
                        cdn_supported: false,
                        location: location.clone(),
                        offset,
                        limit: DOWNLOAD_CHUNK_SIZE,
                    };
                    let raw = client.rpc_call_raw_pub(&req).await?;
                    let mut cur = Cursor::from_slice(&raw);
                    if let tl::enums::upload::File::File(f) =
                        tl::enums::upload::File::deserialize(&mut cur)?
                    {
                        let _ = tx.send((part, f.bytes));
                    }
                }
                Ok::<(), InvocationError>(())
            });
        }
        drop(tx);

        // Collect all parts
        let mut parts: Vec<Option<Vec<u8>>> = (0..n_parts).map(|_| None).collect();
        while let Some((idx, data)) = rx.recv().await {
            if idx < parts.len() {
                parts[idx] = Some(data);
            }
        }

        // Join workers
        while let Some(res) = tasks.join_next().await {
            res.map_err(|e| InvocationError::Io(std::io::Error::other(e.to_string())))??;
        }

        // Assemble in order
        let mut out = Vec::with_capacity(size);
        for part in parts.into_iter().flatten() {
            out.extend_from_slice(&part);
        }
        out.truncate(size);
        Ok(out)
    }

    /// Download any [`Downloadable`] item, automatically choosing concurrent
    /// mode for files ≥ 10 MB (/ integration).
    pub async fn download<D: Downloadable>(&self, item: &D) -> Result<Vec<u8>, InvocationError> {
        let loc = item
            .to_input_location()
            .ok_or_else(|| InvocationError::Deserialize("item has no download location".into()))?;
        match item.size() {
            Some(sz) if sz >= BIG_FILE_THRESHOLD => self.download_media_concurrent(loc, sz).await,
            _ => self.download_media(loc).await,
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
}

/// Extract a download [`InputFileLocation`] from a raw `MessageMedia` reference.
///
/// Used by [`IncomingMessage::download_media`] which only holds a reference to
/// the optional media. Returns `None` when the media has no downloadable file.
pub fn download_location_from_media(
    media: Option<&tl::enums::MessageMedia>,
) -> Option<tl::enums::InputFileLocation> {
    let m = media?;
    if let Some(doc) = Document::from_media(m) {
        return doc.to_input_location();
    }
    if let Some(photo) = Photo::from_media(m) {
        return photo.to_input_location();
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
        let _ = data; // MD5 omitted: Telegram accepts empty checksum
        tl::enums::InputFile::InputFile(tl::types::InputFile {
            id: file_id,
            parts: total_parts,
            name: name.to_string(),
            md5_checksum: String::new(),
        })
    }
}
