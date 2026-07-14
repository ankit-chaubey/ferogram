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

#[allow(unused_imports)]
use ferogram_tl_types::{Cursor, Deserializable};
#[cfg(feature = "experimental")]
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use crate::*;
#[allow(unused_imports)]
use crate::{
    InputMessage, InvocationError, PeerRef,
    dialog::{Dialog, DialogIter, MessageIter},
    inline_iter, media, participants, search, update,
};

/// Builder returned by [`Client::download_file`].
///
/// Awaiting it directly downloads with no progress tracking. Chain
/// [`.handle()`](DownloadFile::handle) before `.await` to track progress,
/// pause, or cancel the transfer. Chain
/// [`.with_safety()`](DownloadFile::with_safety) to use a different
/// [`TransferSafety`](crate::TransferSafety) policy than the client's
/// default for just this one download.
pub struct DownloadFile<'a, D: media::Downloadable> {
    client: &'a Client,
    item: &'a D,
    path: std::path::PathBuf,
    handle: Option<&'a crate::transfer::TransferHandle>,
    safety: Option<crate::transfer_safety::TransferSafety>,
}

impl<'a, D: media::Downloadable> DownloadFile<'a, D> {
    /// Track progress, pause, or cancel this transfer with `handle`.
    pub fn handle(mut self, handle: &'a crate::transfer::TransferHandle) -> Self {
        self.handle = Some(handle);
        self
    }

    /// Use `safety` instead of the client's default
    /// [`TransferSafety`](crate::TransferSafety) for this download only -
    /// e.g. a tighter budget for a large file, or a looser one for a
    /// trusted DC. Everything else about the transfer (worker count,
    /// pipeline depth magnitude) still comes from
    /// [`TransferLimits`](crate::TransferLimits) as usual; this only
    /// changes the hard ceiling layered on top.
    pub fn with_safety(mut self, safety: crate::transfer_safety::TransferSafety) -> Self {
        self.safety = Some(safety);
        self
    }
}

impl<'a, D: media::Downloadable + Sync> std::future::IntoFuture for DownloadFile<'a, D> {
    type Output = Result<u64, InvocationError>;
    type IntoFuture =
        std::pin::Pin<Box<dyn std::future::Future<Output = Self::Output> + Send + 'a>>;

    fn into_future(self) -> Self::IntoFuture {
        let governor = match self.safety {
            Some(safety) => {
                std::sync::Arc::new(crate::transfer_safety::TransferSafetyGovernor::new(safety))
            }
            None => self.client.inner.transfer_safety.clone(),
        };
        Box::pin(async move {
            self.client
                .download_file_inner(self.item, &self.path, self.handle, governor)
                .await
        })
    }
}

/// Builder returned by [`Client::upload`].
///
/// Awaiting it directly uploads with no progress tracking. Chain
/// [`.handle()`](Upload::handle) before `.await` to track progress, pause,
/// or cancel the transfer. Chain [`.with_safety()`](Upload::with_safety) to
/// use a different [`TransferSafety`](crate::TransferSafety) policy than
/// the client's default for just this one upload.
pub struct Upload<'a, R> {
    client: &'a Client,
    source: R,
    name: String,
    handle: Option<&'a crate::transfer::TransferHandle>,
    safety: Option<crate::transfer_safety::TransferSafety>,
}

impl<'a, R> Upload<'a, R> {
    /// Track progress, pause, or cancel this transfer with `handle`.
    pub fn handle(mut self, handle: &'a crate::transfer::TransferHandle) -> Self {
        self.handle = Some(handle);
        self
    }

    /// Use `safety` instead of the client's default
    /// [`TransferSafety`](crate::TransferSafety) for this upload only.
    pub fn with_safety(mut self, safety: crate::transfer_safety::TransferSafety) -> Self {
        self.safety = Some(safety);
        self
    }
}

impl<'a, R> std::future::IntoFuture for Upload<'a, R>
where
    R: tokio::io::AsyncRead + Unpin + Send + 'a,
{
    type Output = Result<crate::media::UploadedFile, InvocationError>;
    type IntoFuture =
        std::pin::Pin<Box<dyn std::future::Future<Output = Self::Output> + Send + 'a>>;

    fn into_future(self) -> Self::IntoFuture {
        let governor = match self.safety {
            Some(safety) => {
                std::sync::Arc::new(crate::transfer_safety::TransferSafetyGovernor::new(safety))
            }
            None => self.client.inner.transfer_safety.clone(),
        };
        Box::pin(async move {
            self.client
                .upload_inner(self.source, &self.name, self.handle, governor)
                .await
        })
    }
}

/// Builder returned by [`Client::upload_file`].
///
/// Awaiting it directly uploads with no progress tracking. Chain
/// [`.handle()`](UploadFile::handle) before `.await` to track progress,
/// pause, or cancel the transfer. Chain
/// [`.with_safety()`](UploadFile::with_safety) to use a different
/// [`TransferSafety`](crate::TransferSafety) policy than the client's
/// default for just this one upload.
pub struct UploadFile<'a> {
    client: &'a Client,
    path: std::path::PathBuf,
    handle: Option<&'a crate::transfer::TransferHandle>,
    safety: Option<crate::transfer_safety::TransferSafety>,
}

impl<'a> UploadFile<'a> {
    /// Track progress, pause, or cancel this transfer with `handle`.
    pub fn handle(mut self, handle: &'a crate::transfer::TransferHandle) -> Self {
        self.handle = Some(handle);
        self
    }

    /// Use `safety` instead of the client's default
    /// [`TransferSafety`](crate::TransferSafety) for this upload only.
    pub fn with_safety(mut self, safety: crate::transfer_safety::TransferSafety) -> Self {
        self.safety = Some(safety);
        self
    }
}

impl<'a> std::future::IntoFuture for UploadFile<'a> {
    type Output = Result<crate::media::UploadedFile, InvocationError>;
    type IntoFuture =
        std::pin::Pin<Box<dyn std::future::Future<Output = Self::Output> + Send + 'a>>;

    fn into_future(self) -> Self::IntoFuture {
        let governor = match self.safety {
            Some(safety) => {
                std::sync::Arc::new(crate::transfer_safety::TransferSafetyGovernor::new(safety))
            }
            None => self.client.inner.transfer_safety.clone(),
        };
        Box::pin(async move {
            self.client
                .upload_file_inner(&self.path, self.handle, governor)
                .await
        })
    }
}

impl Client {
    /// Resolve the checkpoint directory for resumable transfers.
    ///
    /// Uses `ExperimentalFeatures::checkpoint_dir` if set, otherwise
    /// `.ferogram-transfers/` in the current working directory.
    #[cfg(feature = "experimental")]
    fn checkpoint_dir(&self) -> std::path::PathBuf {
        if let Some(dir) = &self.inner.experimental.checkpoint_dir {
            return dir.clone();
        }
        std::path::PathBuf::from(".ferogram-transfers")
    }
    /// Resumable download with persistent checkpoint.
    ///
    /// Requires `features = ["experimental"]` **and**
    /// `ExperimentalFeatures { resumable_transfers: true, .. }` in the client
    /// config.
    ///
    /// On interruption (network error, cancel, crash) the bytes received so far
    /// are flushed to `<checkpoint_dir>/<key>.partial` and the offset is saved
    /// to `<checkpoint_dir>/dl_<key>.json`. On the next call with the same
    /// media the partial bytes are restored into `dest`, the download resumes
    /// from that offset, and all checkpoint files are deleted on success.
    ///
    /// SHA-256 of the complete assembled file is logged on success.
    /// The checkpoint and partial file are deleted automatically on success.
    ///
    /// Falls back to `download` silently if
    /// `resumable_transfers` is `false`.
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// use ferogram::{Client, ExperimentalFeatures, TransferHandle};
    ///
    /// # async fn example(client: Client, media: ferogram_tl_types::enums::MessageMedia) -> anyhow::Result<()> {
    /// // Enable in builder:
    /// // Client::builder()
    /// //     .experimental_features(ExperimentalFeatures {
    /// //         resumable_transfers: true,
    /// //         ..Default::default()
    /// //     })
    ///
    /// let handle = TransferHandle::new();
    /// let mut buf = Vec::new();
    /// client
    ///     .download_resumable(&media, &mut buf, &handle, |p| {
    ///         println!("{:.0}% | {}", p.percent(), p.speed_human());
    ///     })
    ///     .await?;
    /// # Ok(()) }
    /// ```
    #[cfg(feature = "experimental")]
    pub async fn download_resumable(
        &self,
        media: &tl::enums::MessageMedia,
        dest: &mut Vec<u8>,
        handle: &TransferHandle,
        mut on_progress: impl FnMut(TransferProgress) + Send + 'static,
    ) -> Result<u64, InvocationError> {
        use crate::resume::{CheckpointStore, DownloadCheckpoint, download_key, sha256_hex};

        if !self.inner.experimental.resumable_transfers {
            return self
                .download(media, dest as &mut Vec<u8>, Some(handle))
                .await;
        }

        let (loc, dc) = crate::media::location_from_media(media).ok_or_else(|| {
            InvocationError::Deserialize("media has no downloadable location".into())
        })?;
        let total = crate::media::size_from_media(media).unwrap_or(0) as u64;
        let key = download_key(dc, &loc);

        let store = CheckpointStore::open(self.checkpoint_dir())
            .await
            .map_err(InvocationError::Io)?;

        // Restore already-downloaded bytes and determine resume offset.
        let resume_offset: i64 = if let Some(cp) = store.load_download(&key).await {
            let partial_path = store.partial_path(&key);
            match tokio::fs::read(&partial_path).await {
                Ok(bytes) if !bytes.is_empty() => {
                    let restored = bytes.len() as i64;
                    tracing::info!(
                        target: "ferogram::transfer",
                        offset = restored,
                        "download: checkpoint found, restoring partial bytes",
                    );
                    *dest = bytes;
                    // Align down to 1 MB boundary (Telegram requirement).
                    let mb = 1024 * 1024i64;
                    (restored / mb) * mb
                }
                _ => {
                    // Partial file missing or empty; discard checkpoint and restart.
                    tracing::info!(
                        target: "ferogram::transfer",
                        "download: checkpoint found but partial file missing, restarting",
                    );
                    store.delete_download(&key).await;
                    dest.clear();
                    0
                }
            }
        } else {
            dest.clear();
            0
        };

        // Pre-seed handle so progress reflects already-restored bytes.
        handle.set_total(total);
        if resume_offset > 0 {
            handle.add_bytes(dest.len() as u64);
        }
        handle.reset_start();

        let done = Arc::new(AtomicBool::new(false));
        let ctl = handle.clone();
        let done2 = done.clone();

        tokio::spawn(async move {
            loop {
                tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                if done2.load(Ordering::Acquire) || ctl.is_cancelled() {
                    break;
                }
                on_progress(ctl.progress());
            }
        });

        // Download the tail (from resume_offset onward) into a scratch buffer.
        let mut tail: Vec<u8> = Vec::new();
        let result = self
            .download_streaming_on_dc_from(
                loc.clone(),
                dc,
                &mut tail,
                Some(handle),
                resume_offset,
                self.inner.transfer_safety.clone(),
            )
            .await;
        done.store(true, Ordering::Release);

        match result {
            Ok(_) => {
                // Discard overlap: tail may begin before dest.len() due to MB alignment.
                let already = dest.len() as i64;
                let skip = (already - resume_offset).max(0) as usize;
                dest.extend_from_slice(&tail[skip.min(tail.len())..]);

                let n = dest.len() as u64;
                if total > 0 && n != total {
                    tracing::warn!(
                        target: "ferogram::transfer",
                        expected = total,
                        got = n,
                        "download size mismatch",
                    );
                }

                // SHA-256 of the complete assembled file.
                let hash = sha256_hex(dest);
                tracing::info!(
                    target: "ferogram::transfer",
                    sha256 = %hash,
                    bytes = n,
                    "download complete",
                );

                // Clean up.
                store.delete_download(&key).await;
                let _ = tokio::fs::remove_file(store.partial_path(&key)).await;
                Ok(n)
            }
            Err(e) => {
                // Append whatever we got before the error.
                let already = dest.len() as i64;
                let skip = (already - resume_offset).max(0) as usize;
                dest.extend_from_slice(&tail[skip.min(tail.len())..]);

                let offset_now = dest.len() as i64;
                // Flush partial bytes to disk so they survive a restart.
                let partial_path = store.partial_path(&key);
                if let Err(io) = tokio::fs::write(&partial_path, &*dest).await {
                    tracing::warn!(
                        target: "ferogram::transfer",
                        error = %io,
                        "download: failed to write partial file",
                    );
                }
                let cp = DownloadCheckpoint {
                    key: key.clone(),
                    offset: offset_now,
                    total,
                    // No partial hash; SHA-256 is only meaningful on a complete file.
                    sha256_partial: String::new(),
                };
                store.save_download(&cp).await;
                tracing::info!(
                    target: "ferogram::transfer",
                    offset = offset_now,
                    "download interrupted, checkpoint saved",
                );
                Err(e)
            }
        }
    }

    /// Resumable upload with persistent checkpoint.
    ///
    /// Requires `features = ["experimental"]` **and**
    /// `ExperimentalFeatures { resumable_transfers: true, .. }` in the client
    /// config.
    ///
    /// On interruption the upload session state is saved to the configured
    /// checkpoint directory. Telegram upload sessions are valid for ~1 hour;
    /// if the checkpoint is older, a fresh upload starts automatically.
    ///
    /// Falls back to `upload` silently if
    /// `resumable_transfers` is `false`.
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// use ferogram::{Client, ExperimentalFeatures, TransferHandle};
    ///
    /// # async fn example(client: Client) -> anyhow::Result<()> {
    /// // Enable in builder:
    /// // Client::builder()
    /// //     .experimental_features(ExperimentalFeatures {
    /// //         resumable_transfers: true,
    /// //         ..Default::default()
    /// //     })
    ///
    /// let handle = TransferHandle::new();
    /// let data = tokio::fs::read("video.mp4").await?;
    /// let uploaded = client
    ///     .upload_resumable(data, "video.mp4", &handle, |p| {
    ///         println!("{:.0}% | {}", p.percent(), p.speed_human());
    ///     })
    ///     .await?;
    /// # Ok(()) }
    /// ```
    #[cfg(feature = "experimental")]
    pub async fn upload_resumable(
        &self,
        data: Vec<u8>,
        name: &str,
        handle: &TransferHandle,
        mut on_progress: impl FnMut(TransferProgress) + Send + 'static,
    ) -> Result<media::UploadedFile, InvocationError> {
        use crate::resume::{
            CheckpointStore, UPLOAD_SESSION_TTL_MS, UploadCheckpoint, now_ms, upload_key,
        };

        if !self.inner.experimental.resumable_transfers {
            return self
                .upload(std::io::Cursor::new(data), name)
                .handle(handle)
                .await;
        }

        if data.is_empty() {
            return Err(InvocationError::Deserialize(
                "cannot upload empty file".into(),
            ));
        }

        let key = upload_key(&data, name);
        let store = CheckpointStore::open(self.checkpoint_dir())
            .await
            .map_err(InvocationError::Io)?;

        let total = data.len();
        let big = total > crate::media::BIG_FILE_THRESHOLD;
        let (part_size, total_parts) = crate::media::upload_part_size(total);

        let existing = store.load_upload(&key).await;
        let (file_id, start_part, cp_mime) = if let Some(cp) = &existing {
            let age = now_ms().saturating_sub(cp.started_ms);
            if age < UPLOAD_SESSION_TTL_MS
                && cp.total_parts == total_parts
                && cp.part_size == part_size
            {
                tracing::debug!(
                    target: "ferogram::transfer",
                    part = cp.last_part + 1,
                    total_parts,
                    "upload: resuming from checkpoint",
                );
                (
                    cp.file_id,
                    (cp.last_part + 1) as usize,
                    cp.mime_type.clone(),
                )
            } else {
                tracing::debug!(target: "ferogram::transfer", "upload: checkpoint expired or incompatible; restarting from scratch");
                store.delete_upload(&key).await;
                (crate::media::random_file_id_pub(), 0, String::new())
            }
        } else {
            (crate::media::random_file_id_pub(), 0, String::new())
        };

        let resolved_mime = if cp_mime.is_empty() {
            crate::media::resolve_mime_pub(name)
        } else {
            cp_mime
        };

        handle.set_total(total as u64);
        if start_part > 0 {
            handle.add_bytes((start_part * part_size).min(total) as u64);
        }

        let done = Arc::new(AtomicBool::new(false));
        let ctl = handle.clone();
        let done2 = done.clone();

        tokio::spawn(async move {
            loop {
                tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                if done2.load(Ordering::Acquire) || ctl.is_cancelled() {
                    break;
                }
                on_progress(ctl.progress());
            }
        });

        let mut last_good_part: i32 = start_part as i32 - 1;
        let chunks: Vec<&[u8]> = data.chunks(part_size).collect();

        for (i, chunk) in chunks.iter().enumerate() {
            if i < start_part {
                continue;
            }

            handle.poll_pause_cancel().await?;

            let chunk_len = chunk.len();
            let mut delay_ms: u64 = 1000;
            let mut attempt = 0u8;

            loop {
                let res = self
                    .upload_part_pub(big, file_id, i as i32, total_parts, chunk)
                    .await;

                match res {
                    Ok(_) => break,
                    Err(e) if attempt < 5 => {
                        tracing::warn!(
                            target: "ferogram::transfer",
                            part = i,
                            attempt,
                            retry_ms = delay_ms,
                            error = %e,
                            "upload part failed, retrying",
                        );
                        tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;
                        delay_ms = (delay_ms * 2).min(30_000);
                        attempt += 1;
                    }
                    Err(e) => {
                        done.store(true, Ordering::Release);
                        let cp = UploadCheckpoint {
                            key: key.clone(),
                            file_id,
                            last_part: last_good_part,
                            total_parts,
                            part_size,
                            total: total as u64,
                            big,
                            name: name.to_string(),
                            mime_type: resolved_mime.clone(),
                            started_ms: existing
                                .as_ref()
                                .map(|c| c.started_ms)
                                .unwrap_or_else(now_ms),
                        };
                        store.save_upload(&cp).await;
                        tracing::info!(
                            target: "ferogram::transfer",
                            part = last_good_part,
                            "upload interrupted, checkpoint saved",
                        );
                        return Err(e);
                    }
                }
            }

            last_good_part = i as i32;
            handle.add_bytes(chunk_len as u64);

            // Checkpoint every 10 parts.
            if i % 10 == 0 {
                let cp = UploadCheckpoint {
                    key: key.clone(),
                    file_id,
                    last_part: last_good_part,
                    total_parts,
                    part_size,
                    total: total as u64,
                    big,
                    name: name.to_string(),
                    mime_type: resolved_mime.clone(),
                    started_ms: existing
                        .as_ref()
                        .map(|c| c.started_ms)
                        .unwrap_or_else(now_ms),
                };
                store.save_upload(&cp).await;
            }
        }

        done.store(true, Ordering::Release);

        let inner = crate::media::make_input_file_pub(big, file_id, total_parts, name, &data);
        store.delete_upload(&key).await;
        tracing::info!(target: "ferogram::transfer", name, total_parts, "upload complete; checkpoint purged");

        Ok(media::UploadedFile::new(
            inner,
            resolved_mime,
            name.to_string(),
        ))
    }
    /// Upload a file from disk one chunk at a time, without ever loading the full file into memory.
    ///
    /// Reads and sends each part sequentially with no concurrency. RAM usage stays flat at
    /// roughly one chunk size regardless of how large the file is. Good for constrained
    /// environments or when you want predictable memory, but slower than [`upload_file`]
    /// on a fast connection.
    ///
    /// MIME type is sniffed from the first bytes of the file so you do not need to
    /// specify it manually.
    ///
    /// Pass a [`TransferHandle`] if you want to pause, resume, or cancel mid-transfer.
    /// Pass `None` to skip progress tracking entirely.
    ///
    /// [`upload_file`]: Client::upload_file
    /// [`TransferHandle`]: crate::transfer::TransferHandle
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// use ferogram::{Client, TransferHandle};
    ///
    /// # async fn example(client: Client) -> anyhow::Result<()> {
    /// let handle = TransferHandle::new();
    /// let uploaded = client.upload_sequential("big_video.mp4", Some(&handle)).await?;
    /// // Then attach to a message:
    /// // client.send_message(chat, InputMessage::text("").document(uploaded)).await?;
    /// # Ok(()) }
    /// ```
    pub async fn upload_sequential(
        &self,
        path: impl AsRef<std::path::Path>,
        handle: Option<&TransferHandle>,
    ) -> Result<media::UploadedFile, InvocationError> {
        use tokio::io::AsyncReadExt;

        let path = path.as_ref();
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("file");
        let meta = tokio::fs::metadata(path)
            .await
            .map_err(InvocationError::Io)?;
        let total = meta.len() as usize;
        let big = total > media::BIG_FILE_THRESHOLD;
        let (part_size, total_parts) = media::upload_part_size(total);
        let file_id = random_i64_pub();

        // Sniff MIME from first chunk.
        let mut f = tokio::fs::File::open(path)
            .await
            .map_err(InvocationError::Io)?;
        let mut header = vec![0u8; part_size.min(65536)];
        let n = f.read(&mut header).await.map_err(InvocationError::Io)?;
        header.truncate(n);
        let mime_type = media::detect_mime_from_bytes(&header, name);

        // Reopen from start.
        let mut f = tokio::fs::File::open(path)
            .await
            .map_err(InvocationError::Io)?;

        if let Some(h) = handle {
            h.set_total(total as u64);
            h.reset_start();
        }

        let mut part_num = 0i32;
        let mut buf = vec![0u8; part_size];
        loop {
            let mut bytes_read = 0;
            while bytes_read < part_size {
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
            let chunk = &buf[..bytes_read];

            if let Some(h) = handle {
                h.poll_pause_cancel().await?;
            }

            if big {
                self.rpc_transfer_on_dc_pub(
                    0,
                    &tl::functions::upload::SaveBigFilePart {
                        file_id,
                        file_part: part_num,
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
                        file_part: part_num,
                        bytes: chunk.to_vec(),
                    },
                )
                .await?;
            }

            if let Some(h) = handle {
                h.add_bytes(bytes_read as u64);
            }
            part_num += 1;
        }

        // Build InputFile from name (no data slice needed; parts are already uploaded).
        let inner = if big {
            tl::enums::InputFile::Big(tl::types::InputFileBig {
                id: file_id,
                parts: total_parts,
                name: name.to_string(),
            })
        } else {
            tl::enums::InputFile::InputFile(tl::types::InputFile {
                id: file_id,
                parts: total_parts,
                name: name.to_string(),
                md5_checksum: String::new(),
            })
        };

        tracing::info!(
            target: "ferogram::transfer",
            name,
            bytes = total,
            parts = total_parts,
            mime = %mime_type,
            "streamed upload complete",
        );

        Ok(media::UploadedFile::new(inner, mime_type, name.to_string()))
    }
    /// Download message media to any writable sink: a `Vec<u8>`, a file handle, a socket, etc.
    ///
    /// Streams data directly to `dest` without buffering the entire file in memory first.
    /// Returns the total number of bytes written.
    ///
    /// If the media is on a different DC than the current connection, ferogram reconnects
    /// transparently. You do not need to handle DC switching yourself.
    ///
    /// Pass a [`TransferHandle`] to track progress, pause, or cancel. Pass `None` to
    /// skip progress tracking.
    ///
    /// [`AsyncWrite`]: tokio::io::AsyncWrite
    /// [`TransferHandle`]: crate::transfer::TransferHandle
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// # use ferogram::Client;
    /// # async fn ex(client: Client, msg: ferogram::update::IncomingMessage) {
    /// // Download to an in-memory buffer
    /// let mut buf = Vec::new();
    /// client.download(msg.media().unwrap(), &mut buf, None).await.unwrap();
    ///
    /// // Stream directly to a file on disk
    /// let mut file = tokio::fs::File::create("photo.jpg").await.unwrap();
    /// client.download(msg.media().unwrap(), &mut file, None).await.unwrap();
    /// # }
    /// ```
    pub async fn download(
        &self,
        item: &impl media::Downloadable,
        mut dest: impl tokio::io::AsyncWrite + Unpin,
        handle: Option<&crate::transfer::TransferHandle>,
    ) -> Result<u64, InvocationError> {
        let loc = item.to_input_location().ok_or_else(|| {
            InvocationError::Deserialize("item has no downloadable location".into())
        })?;
        let dc = item.dc_id();
        if let Some(h) = handle {
            let total = item.size().unwrap_or(0);
            h.set_total(total as u64);
            h.reset_start();
        }
        self.download_streaming_on_dc(
            loc,
            dc,
            &mut dest,
            handle,
            self.inner.transfer_safety.clone(),
        )
        .await
    }

    /// Like [`download`](Self::download) but with `safety` instead of the
    /// client's default [`TransferSafety`](crate::TransferSafety) for this
    /// one download.
    pub async fn download_with_safety(
        &self,
        item: &impl media::Downloadable,
        mut dest: impl tokio::io::AsyncWrite + Unpin,
        handle: Option<&crate::transfer::TransferHandle>,
        safety: crate::transfer_safety::TransferSafety,
    ) -> Result<u64, InvocationError> {
        let loc = item.to_input_location().ok_or_else(|| {
            InvocationError::Deserialize("item has no downloadable location".into())
        })?;
        let dc = item.dc_id();
        if let Some(h) = handle {
            let total = item.size().unwrap_or(0);
            h.set_total(total as u64);
            h.reset_start();
        }
        self.download_streaming_on_dc(
            loc,
            dc,
            &mut dest,
            handle,
            std::sync::Arc::new(crate::transfer_safety::TransferSafetyGovernor::new(safety)),
        )
        .await
    }

    /// Download message media and save it directly to a file at `path`.
    ///
    /// Creates the file if it does not exist, or truncates it if it does. Data is
    /// streamed to disk without loading everything into memory first.
    ///
    /// For large files (over 10 MB) this uses concurrent workers automatically,
    /// which is significantly faster than the sequential path. You do not need to
    /// configure anything; ferogram picks the worker count based on file size.
    ///
    /// Returns the number of bytes written.
    ///
    /// By default no progress tracking happens. Chain [`.handle(&handle)`] before
    /// `.await` if you want to track progress, pause, or cancel the transfer:
    ///
    /// ```rust,no_run
    /// # use ferogram::{Client, transfer::TransferHandle};
    /// # async fn ex(client: Client, msg: ferogram::update::IncomingMessage) -> anyhow::Result<()> {
    /// let handle = TransferHandle::new();
    /// client.download_file(msg.media().unwrap(), "downloaded.mp4")
    ///     .handle(&handle)
    ///     .await?;
    /// # Ok(()) }
    /// ```
    ///
    /// [`.handle(&handle)`]: DownloadFile::handle
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// # use ferogram::Client;
    /// # async fn ex(client: Client, msg: ferogram::update::IncomingMessage) -> anyhow::Result<()> {
    /// client.download_file(msg.media().unwrap(), "downloaded.mp4").await?;
    /// # Ok(()) }
    /// ```
    pub fn download_file<'a, D: media::Downloadable>(
        &'a self,
        item: &'a D,
        path: impl AsRef<std::path::Path>,
    ) -> DownloadFile<'a, D> {
        DownloadFile {
            client: self,
            item,
            path: path.as_ref().to_path_buf(),
            handle: None,
            safety: None,
        }
    }

    /// Inner implementation behind the [`download_file`] builder.
    ///
    /// Also the shared engine behind [`download_media`](Self::download_media):
    /// both funnel through this once the caller has a concrete
    /// [`Downloadable`](media::Downloadable) in hand, so the
    /// concurrent-vs-sequential decision lives in exactly one place.
    ///
    /// [`download_file`]: Client::download_file
    async fn download_file_inner(
        &self,
        item: &impl media::Downloadable,
        path: &std::path::Path,
        handle: Option<&crate::transfer::TransferHandle>,
        safety: std::sync::Arc<crate::transfer_safety::TransferSafetyGovernor>,
    ) -> Result<u64, InvocationError> {
        let loc = item.to_input_location().ok_or_else(|| {
            InvocationError::Deserialize("item has no downloadable location".into())
        })?;
        let dc = item.dc_id();
        if let Some(size) = item.size()
            && size >= crate::media::DOWNLOAD_CONCURRENT_THRESHOLD
        {
            return self
                .download_media_concurrent_on_dc_to_file_pipelined(
                    loc, size, dc, path, handle, safety,
                )
                .await;
        }
        let mut file = tokio::fs::File::create(path)
            .await
            .map_err(InvocationError::Io)?;
        self.download_streaming_on_dc(loc, dc, &mut file, handle, safety)
            .await
    }

    /// Download `media` to `path`, choosing a specific quality variant when
    /// the media carries alternates (`alt_documents`) - typically a video
    /// with a quality picker in official clients.
    ///
    /// Uses exactly the same engine as [`download_file`](Self::download_file):
    /// the concurrent/pipelined path for files at or above
    /// [`DOWNLOAD_CONCURRENT_THRESHOLD`](crate::media::DOWNLOAD_CONCURRENT_THRESHOLD),
    /// governed by the same [`TransferLimits`](crate::TransferLimits) - Y, X,
    /// and the global connection cap all apply here exactly as they do for
    /// any other download. Quality selection only changes *which* document
    /// gets downloaded, not how.
    ///
    /// Media with no alternates only ever has [`MediaQuality::Original`] to
    /// pick from; every other variant falls back to it automatically. Use
    /// [`crate::media::available_qualities`] to see what's actually on offer
    /// before choosing, if you want to build your own picker.
    ///
    /// # Example
    /// ```rust,no_run
    /// # use ferogram::{Client, MediaQuality};
    /// # async fn ex(client: Client, media: ferogram_tl_types::enums::MessageMedia) -> anyhow::Result<()> {
    /// // Grab the lightest available quality, e.g. for a quick preview.
    /// client.download_media(&media, MediaQuality::Lowest, "preview.mp4", None).await?;
    ///
    /// // Or the best quality Telegram has for this video.
    /// client.download_media(&media, MediaQuality::Highest, "video.mp4", None).await?;
    /// # Ok(()) }
    /// ```
    pub async fn download_media(
        &self,
        media: &tl::enums::MessageMedia,
        quality: crate::media::MediaQuality,
        path: impl AsRef<std::path::Path>,
        handle: Option<&crate::transfer::TransferHandle>,
    ) -> Result<u64, InvocationError> {
        let doc = crate::media::resolve_quality_document(media, quality).ok_or_else(|| {
            InvocationError::Deserialize(
                "media has no downloadable document for the requested quality".into(),
            )
        })?;
        self.download_file_inner(
            &doc,
            path.as_ref(),
            handle,
            self.inner.transfer_safety.clone(),
        )
        .await
    }

    /// Like [`download_media`](Self::download_media) but with `safety`
    /// instead of the client's default [`TransferSafety`](crate::TransferSafety)
    /// for this one download.
    pub async fn download_media_with_safety(
        &self,
        media: &tl::enums::MessageMedia,
        quality: crate::media::MediaQuality,
        path: impl AsRef<std::path::Path>,
        handle: Option<&crate::transfer::TransferHandle>,
        safety: crate::transfer_safety::TransferSafety,
    ) -> Result<u64, InvocationError> {
        let doc = crate::media::resolve_quality_document(media, quality).ok_or_else(|| {
            InvocationError::Deserialize(
                "media has no downloadable document for the requested quality".into(),
            )
        })?;
        self.download_file_inner(
            &doc,
            path.as_ref(),
            handle,
            std::sync::Arc::new(crate::transfer_safety::TransferSafetyGovernor::new(safety)),
        )
        .await
    }

    /// Return a lazy chunk iterator for `media`.
    ///
    /// Useful when you want to process file bytes as they arrive instead of waiting
    /// for the full download to complete. Each call to [`DownloadIter::next`] fetches
    /// the next chunk from Telegram and returns it as a `bytes::Bytes` slice.
    ///
    /// Returns `None` if the media does not have a downloadable location (for example,
    /// a contact card or a venue).
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// # use ferogram::Client;
    /// # async fn ex(client: Client, msg: ferogram::update::IncomingMessage) -> anyhow::Result<()> {
    /// if let Some(mut iter) = client.iter_download(msg.media().unwrap()) {
    ///     while let Some(chunk) = iter.next().await? {
    ///         // process chunk bytes
    ///     }
    /// }
    /// # Ok(()) }
    /// ```
    pub fn iter_download(
        &self,
        item: &impl media::Downloadable,
    ) -> Option<crate::media::DownloadIter> {
        let loc = item.to_input_location()?;
        let dc = item.dc_id();
        Some(crate::media::DownloadIter::new(self.clone(), loc, dc))
    }

    /// Upload from any [`tokio::io::AsyncRead`] source: a file handle, a network stream,
    /// a cursor over bytes in memory, etc.
    ///
    /// Reads the entire source into memory first, then uploads using the optimal part
    /// size. If the data is larger than 10 MB, concurrent workers are used automatically.
    ///
    /// If you already have a path on disk, prefer [`upload_file`] instead. It stats the
    /// file before opening it and avoids the in-memory buffer for large files.
    ///
    /// By default no progress tracking happens. Chain [`.handle(&handle)`] before
    /// `.await` if you want to track progress, pause, or cancel the transfer:
    ///
    /// ```rust,no_run
    /// # use ferogram::{Client, transfer::TransferHandle};
    /// # async fn ex(client: Client) -> anyhow::Result<()> {
    /// let handle = TransferHandle::new();
    /// let bytes = b"hello world".to_vec();
    /// let uploaded = client.upload(std::io::Cursor::new(bytes), "note.txt")
    ///     .handle(&handle)
    ///     .await?;
    /// # Ok(()) }
    /// ```
    ///
    /// [`tokio::io::AsyncRead`]: tokio::io::AsyncRead
    /// [`upload_file`]: Client::upload_file
    /// [`.handle(&handle)`]: Upload::handle
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// use ferogram::Client;
    ///
    /// # async fn example(client: Client) -> anyhow::Result<()> {
    /// let bytes = b"hello world".to_vec();
    /// let uploaded = client.upload(std::io::Cursor::new(bytes), "note.txt").await?;
    /// # Ok(()) }
    /// ```
    pub fn upload<'a, R>(&'a self, source: R, name: &str) -> Upload<'a, R>
    where
        R: tokio::io::AsyncRead + Unpin + Send + 'a,
    {
        Upload {
            client: self,
            source,
            name: name.to_string(),
            handle: None,
            safety: None,
        }
    }

    /// Inner implementation behind the [`upload`] builder.
    ///
    /// [`upload`]: Client::upload
    async fn upload_inner(
        &self,
        mut source: impl tokio::io::AsyncRead + Unpin + Send,
        name: &str,
        handle: Option<&crate::transfer::TransferHandle>,
        safety: std::sync::Arc<crate::transfer_safety::TransferSafetyGovernor>,
    ) -> Result<crate::media::UploadedFile, InvocationError> {
        use tokio::io::AsyncReadExt;
        let mut data = Vec::new();
        source
            .read_to_end(&mut data)
            .await
            .map_err(InvocationError::Io)?;
        if data.len() > crate::media::BIG_FILE_THRESHOLD {
            self.upload_file_concurrent_pipelined_inner(
                std::sync::Arc::new(data),
                name,
                "",
                handle,
                safety,
            )
            .await
        } else {
            self.upload_bytes(&data, name, "", handle, safety).await
        }
    }

    /// Upload a file from disk by path. This is the standard upload method for most use cases.
    ///
    /// Stats the file first so ferogram can pick the right part size without reading
    /// the entire file upfront. For large files (over 10 MB) it streams from disk with
    /// concurrent workers, keeping RAM usage low even for multi-gigabyte files.
    ///
    /// For strict sequential uploads with a fixed memory ceiling, use [`upload_sequential`].
    ///
    /// MIME type is detected automatically from the file name and content.
    ///
    /// By default no progress tracking happens. Chain [`.handle(&handle)`] before
    /// `.await` if you want to track progress, pause, or cancel the transfer:
    ///
    /// ```rust,no_run
    /// # use ferogram::{Client, TransferHandle};
    /// # async fn ex(client: Client) -> anyhow::Result<()> {
    /// let handle = TransferHandle::new();
    /// let uploaded = client.upload_file("photo.jpg")
    ///     .handle(&handle)
    ///     .await?;
    /// # Ok(()) }
    /// ```
    ///
    /// [`upload_sequential`]: Client::upload_sequential
    /// [`.handle(&handle)`]: UploadFile::handle
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// use ferogram::Client;
    ///
    /// # async fn example(client: Client) -> anyhow::Result<()> {
    /// let uploaded = client.upload_file("photo.jpg").await?;
    /// // Then send it as a photo:
    /// // client.send_message(chat, InputMessage::text("").photo(uploaded)).await?;
    /// # Ok(()) }
    /// ```
    pub fn upload_file<'a>(&'a self, path: impl AsRef<std::path::Path>) -> UploadFile<'a> {
        UploadFile {
            client: self,
            path: path.as_ref().to_path_buf(),
            handle: None,
            safety: None,
        }
    }

    /// Inner implementation behind the [`upload_file`] builder.
    ///
    /// [`upload_file`]: Client::upload_file
    async fn upload_file_inner(
        &self,
        path: &std::path::Path,
        handle: Option<&crate::transfer::TransferHandle>,
        safety: std::sync::Arc<crate::transfer_safety::TransferSafetyGovernor>,
    ) -> Result<crate::media::UploadedFile, InvocationError> {
        use tokio::io::AsyncReadExt;
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("file");
        let meta = tokio::fs::metadata(path)
            .await
            .map_err(InvocationError::Io)?;
        let size = meta.len() as usize;
        if size >= crate::media::BIG_FILE_THRESHOLD {
            return self
                .upload_file_concurrent_streaming_pipelined(path, name, "", handle, safety)
                .await;
        }
        let mut file = tokio::fs::File::open(path)
            .await
            .map_err(InvocationError::Io)?;
        let mut data = Vec::with_capacity(size);
        file.read_to_end(&mut data)
            .await
            .map_err(InvocationError::Io)?;
        self.upload_bytes(&data, name, "", handle, safety).await
    }
    /// Get every message in the same media group (album) as `msg_id`, given
    /// any one message from that group.
    pub async fn get_media_group(
        &self,
        peer: impl Into<PeerRef>,
        msg_id: i32,
    ) -> Result<Vec<update::IncomingMessage>, InvocationError> {
        use ferogram_tl_types as tl;
        let peer = peer.into().resolve(self).await?;
        let input_peer = self.inner.peer_cache.read().await.peer_to_input(&peer)?;

        // Fetch the seed message first to get grouped_id
        let seed_ids = vec![tl::enums::InputMessage::Id(tl::types::InputMessageId {
            id: msg_id,
        })];

        let seed_msgs = match &input_peer {
            tl::enums::InputPeer::Channel(c) => {
                let req = tl::functions::channels::GetMessages {
                    channel: tl::enums::InputChannel::InputChannel(tl::types::InputChannel {
                        channel_id: c.channel_id,
                        access_hash: c.access_hash,
                    }),
                    id: seed_ids,
                };
                let body = self.rpc_call_raw(&req).await?;
                let mut cur = Cursor::from_slice(&body);
                match tl::enums::messages::Messages::deserialize(&mut cur)? {
                    tl::enums::messages::Messages::Messages(m) => m.messages,
                    tl::enums::messages::Messages::Slice(m) => m.messages,
                    tl::enums::messages::Messages::ChannelMessages(m) => m.messages,
                    tl::enums::messages::Messages::NotModified(_) => vec![],
                }
            }
            _ => {
                let req = tl::functions::messages::GetMessages { id: seed_ids };
                let body = self.rpc_call_raw(&req).await?;
                let mut cur = Cursor::from_slice(&body);
                match tl::enums::messages::Messages::deserialize(&mut cur)? {
                    tl::enums::messages::Messages::Messages(m) => m.messages,
                    tl::enums::messages::Messages::Slice(m) => m.messages,
                    tl::enums::messages::Messages::ChannelMessages(m) => m.messages,
                    tl::enums::messages::Messages::NotModified(_) => vec![],
                }
            }
        };

        // Extract grouped_id from the seed message
        let grouped_id = seed_msgs.iter().find_map(|m| {
            if let tl::enums::Message::Message(msg) = m {
                msg.grouped_id
            } else {
                None
            }
        });

        // If there's no grouped_id, just return the single message
        let Some(gid) = grouped_id else {
            return Ok(seed_msgs
                .into_iter()
                .map(update::IncomingMessage::from_raw)
                .collect());
        };

        // Fetch a window of messages around msg_id to find all members of the group
        // Albums are always contiguous so a window of ±10 is more than enough
        let window_start = (msg_id - 9).max(1);
        let window_ids: Vec<tl::enums::InputMessage> = (window_start..=msg_id + 9)
            .map(|id| tl::enums::InputMessage::Id(tl::types::InputMessageId { id }))
            .collect();

        let window_msgs = match &input_peer {
            tl::enums::InputPeer::Channel(c) => {
                let req = tl::functions::channels::GetMessages {
                    channel: tl::enums::InputChannel::InputChannel(tl::types::InputChannel {
                        channel_id: c.channel_id,
                        access_hash: c.access_hash,
                    }),
                    id: window_ids,
                };
                let body = self.rpc_call_raw(&req).await?;
                let mut cur = Cursor::from_slice(&body);
                match tl::enums::messages::Messages::deserialize(&mut cur)? {
                    tl::enums::messages::Messages::Messages(m) => m.messages,
                    tl::enums::messages::Messages::Slice(m) => m.messages,
                    tl::enums::messages::Messages::ChannelMessages(m) => m.messages,
                    tl::enums::messages::Messages::NotModified(_) => vec![],
                }
            }
            _ => seed_msgs,
        };

        let group: Vec<update::IncomingMessage> = window_msgs
            .into_iter()
            .filter(|m| {
                if let tl::enums::Message::Message(msg) = m {
                    msg.grouped_id == Some(gid)
                } else {
                    false
                }
            })
            .map(update::IncomingMessage::from_raw)
            .collect();

        Ok(group)
    }

    /// Upload a single part for experimental resumable upload.
    #[cfg(feature = "experimental")]
    pub(crate) async fn upload_part_pub(
        &self,
        big: bool,
        file_id: i64,
        part: i32,
        total_parts: i32,
        data: &[u8],
    ) -> Result<bool, InvocationError> {
        if big {
            self.rpc_call(tl::functions::upload::SaveBigFilePart {
                file_id,
                file_part: part,
                file_total_parts: total_parts,
                bytes: data.to_vec(),
            })
            .await
        } else {
            self.rpc_call(tl::functions::upload::SaveFilePart {
                file_id,
                file_part: part,
                bytes: data.to_vec(),
            })
            .await
        }
    }
}

/// Configuration for experimental high-throughput transfers.
///
/// Both fields are clamped internally: `workers` to 1-[`MAX_GLOBAL_SENDERS`] (12) in
/// `upload_exp`/`download_exp`. Chunk size to 128 KB-[`MAX_PART_SIZE`].
/// Pass `None` to accept the library default.
///
/// [`MAX_GLOBAL_SENDERS`]: crate::media::MAX_GLOBAL_SENDERS
/// [`MAX_PART_SIZE`]: crate::media::MAX_PART_SIZE
#[cfg(feature = "experimental")]
#[derive(Debug, Clone, Default)]
pub struct TransferConfig {
    /// Number of parallel workers. `None` = auto (scales with file size).
    ///
    /// In `upload_exp` / `download_exp` this can go up to
    /// [`MAX_GLOBAL_SENDERS`] (12). All other transfer methods cap at
    /// [`MAX_WORKERS_PER_FILE`] (4).
    ///
    /// [`MAX_GLOBAL_SENDERS`]: crate::media::MAX_GLOBAL_SENDERS
    /// [`MAX_WORKERS_PER_FILE`]: crate::media::MAX_WORKERS_PER_FILE
    pub workers: Option<usize>,
    /// Chunk size in bytes per request. `None` = auto (256 KB or 512 KB).
    pub chunk_size: Option<usize>,
}

#[cfg(feature = "experimental")]
impl Client {
    /// # Warning: bypasses connection safety limits
    ///
    /// **`upload_exp` bypasses ferogram's built-in connection safety limits.**
    ///
    /// - Workers can go up to 12 (the global MTProto connection ceiling).
    ///   If other transfers are running concurrently, they will **block**
    ///   until `upload_exp` releases permits back to the pool.
    /// - Telegram actively rate-limits and **bans** accounts that open too
    ///   many concurrent connections or upload too aggressively. ferogram
    ///   does **not** protect you here. You are fully responsible.
    /// - Do not use this in production user-facing code. It exists for
    ///   benchmarking, internal tooling, and situations where you know
    ///   exactly what you are doing and have tested against your specific
    ///   account and server conditions.
    ///
    /// **Use [`upload_file`] for all normal uploads.** It auto-tunes workers
    /// and chunk size safely and will not get your account rate-limited.
    ///
    /// ---
    ///
    /// Upload a file from disk with manually specified concurrency and chunk size.
    ///
    /// Requires `features = ["experimental"]` in `Cargo.toml`.
    ///
    /// Workers are clamped to 1-[`MAX_GLOBAL_SENDERS`] (12).
    /// Chunk size is clamped to 128 KB-[`MAX_PART_SIZE`], rounded down to the nearest 1 KB.
    /// The file is never fully loaded into memory; it is streamed from disk in parallel.
    ///
    /// [`upload_file`]: Client::upload_file
    /// [`MAX_GLOBAL_SENDERS`]: crate::media::MAX_GLOBAL_SENDERS
    /// [`MAX_PART_SIZE`]: crate::media::MAX_PART_SIZE
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// use ferogram::{Client, TransferHandle, TransferConfig};
    ///
    /// # async fn example(client: Client) -> anyhow::Result<()> {
    /// // WARNING: high worker counts risk rate limits and account bans.
    /// // Only use if you know what you are doing.
    /// let handle = TransferHandle::new();
    /// let uploaded = client
    ///     .upload_exp(
    ///         "big_video.mp4",
    ///         Some(&handle),
    ///         TransferConfig { workers: Some(8), chunk_size: Some(512 * 1024) },
    ///     )
    ///     .await?;
    /// # Ok(()) }
    /// ```
    pub async fn upload_exp(
        &self,
        path: impl AsRef<std::path::Path>,
        handle: Option<&crate::transfer::TransferHandle>,
        config: crate::client::files::TransferConfig,
    ) -> Result<crate::media::UploadedFile, InvocationError> {
        let path = path.as_ref();
        let meta = tokio::fs::metadata(path)
            .await
            .map_err(InvocationError::Io)?;
        let size = meta.len() as usize;

        // exp: auto-tune uses the fixed MAX_WORKERS_PER_FILE ceiling, not the
        // caller's configured TransferLimits - this path exists specifically
        // to bypass configured safety limits, so it must not inherit them.
        let workers = config
            .workers
            .unwrap_or_else(|| {
                crate::media::upload_worker_count(size, crate::media::MAX_WORKERS_PER_FILE)
            })
            .max(1)
            .min(crate::media::MAX_GLOBAL_SENDERS); // exp: up to 12, not the normal 4 ceiling

        let chunk_size = config
            .chunk_size
            .unwrap_or_else(|| crate::media::upload_part_size(size).0)
            .max(128 * 1024)
            .min(crate::media::MAX_PART_SIZE);
        // Round down to nearest 1 KB boundary.
        let chunk_size = (chunk_size / 1024) * 1024;

        self.upload_file_concurrent_streaming_exp(path, workers, chunk_size, handle)
            .await
    }

    /// # Warning: bypasses connection safety limits
    ///
    /// **`download_exp` bypasses ferogram's built-in connection safety limits.**
    ///
    /// - Workers can go up to 12 (the global MTProto connection ceiling).
    ///   If other transfers are running concurrently, they will **block**
    ///   until `download_exp` releases permits back to the pool.
    /// - Telegram actively rate-limits and **bans** accounts that open too
    ///   many concurrent connections or download too aggressively. ferogram
    ///   does **not** protect you here. You are fully responsible.
    /// - Do not use this in production user-facing code. It exists for
    ///   benchmarking, internal tooling, and situations where you know
    ///   exactly what you are doing and have tested against your specific
    ///   account and server conditions.
    ///
    /// **Use [`download`] or [`download_file`] for all normal downloads.**
    /// They auto-tune workers and chunk size safely and will not get your
    /// account rate-limited.
    ///
    /// ---
    ///
    /// Download media with manually specified concurrency and chunk size.
    ///
    /// Requires `features = ["experimental"]` in `Cargo.toml`.
    ///
    /// Workers are clamped to 1-[`MAX_GLOBAL_SENDERS`] (12).
    /// Chunk size is clamped to 128 KB-512 KB (Telegram's hard `GetFile` ceiling),
    /// rounded down to the nearest 1 KB. Assembled bytes are written into `dest`,
    /// which is pre-allocated to the exact file size before downloading begins.
    ///
    /// [`download`]: Client::download
    /// [`download_file`]: Client::download_file
    /// [`MAX_GLOBAL_SENDERS`]: crate::media::MAX_GLOBAL_SENDERS
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// use ferogram::{Client, TransferHandle, TransferConfig};
    ///
    /// # async fn example(client: Client, media: ferogram_tl_types::enums::MessageMedia) -> anyhow::Result<()> {
    /// // WARNING: high worker counts risk rate limits and account bans.
    /// // Only use if you know what you are doing.
    /// let handle = TransferHandle::new();
    /// let mut buf = Vec::new();
    /// client
    ///     .download_exp(
    ///         &media,
    ///         &mut buf,
    ///         Some(&handle),
    ///         TransferConfig { workers: Some(8), chunk_size: Some(512 * 1024) },
    ///     )
    ///     .await?;
    /// # Ok(()) }
    /// ```
    pub async fn download_exp(
        &self,
        media: &tl::enums::MessageMedia,
        dest: &mut Vec<u8>,
        handle: Option<&crate::transfer::TransferHandle>,
        config: crate::client::files::TransferConfig,
    ) -> Result<u64, InvocationError> {
        let (loc, dc) = crate::media::location_from_media(media).ok_or_else(|| {
            InvocationError::Deserialize("media has no downloadable location".into())
        })?;
        let size = crate::media::size_from_media(media).unwrap_or(0);

        // exp: same rationale as upload_exp above - fixed ceiling, not the
        // caller's TransferLimits.
        let workers = config
            .workers
            .unwrap_or_else(|| {
                crate::media::download_worker_count(size, crate::media::MAX_WORKERS_PER_FILE)
            })
            .max(1)
            .min(crate::media::MAX_GLOBAL_SENDERS); // exp: up to 12, not the normal 4 ceiling

        // Telegram's GetFile hard ceiling is 512 KB per request.
        let chunk_size = config
            .chunk_size
            .unwrap_or_else(|| crate::media::download_chunk_size(size) as usize)
            .max(128 * 1024)
            .min(512 * 1024);
        let chunk_size = (chunk_size / 1024) * 1024;

        if let Some(h) = handle {
            h.set_total(size as u64);
            h.reset_start();
        }

        self.download_concurrent_exp(loc, dc, size, dest, workers, chunk_size as i32, handle)
            .await
    }
}
