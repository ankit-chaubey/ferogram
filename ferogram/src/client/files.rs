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

    /// Download media and call `on_progress` once per second while transferring.
    ///
    /// The callback is a plain sync `FnMut(TransferProgress)`. For async work
    /// (editing a Telegram message) use a channel and a separate async task.
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// use ferogram::{Client, TransferHandle};
    ///
    /// # async fn example(client: Client, media: ferogram_tl_types::enums::MessageMedia) -> anyhow::Result<()> {
    /// let handle = TransferHandle::new();
    /// let mut buf = Vec::new();
    /// client
    ///     .download_with_progress(&media, &mut buf, &handle, |p| {
    ///         println!("{:.0}% | {}", p.percent(), p.speed_human());
    ///     })
    ///     .await?;
    /// # Ok(()) }
    /// ```
    pub async fn download_with_progress(
        &self,
        media: &tl::enums::MessageMedia,
        dest: impl tokio::io::AsyncWrite + Unpin,
        handle: &TransferHandle,
        mut on_progress: impl FnMut(TransferProgress) + Send + 'static,
    ) -> Result<u64, InvocationError> {
        let span = tracing::info_span!(
            target: "ferogram::transfer",
            "download",
            total = tracing::field::Empty,
        );
        let _enter = span.enter();

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

        let result = self.download(media, dest, Some(handle)).await;
        done.store(true, Ordering::Release);
        result
    }

    /// Upload from any [`AsyncRead`] source and call `on_progress` once per second.
    ///
    /// Same callback rules as [`download_with_progress`]: sync only.
    /// For async work use a channel in a separate task.
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// use ferogram::{Client, TransferHandle};
    ///
    /// # async fn example(client: Client) -> anyhow::Result<()> {
    /// let handle = TransferHandle::new();
    /// let data = std::io::Cursor::new(vec![0u8; 1024]);
    /// let uploaded = client
    ///     .upload_with_progress(data, "file.bin", &handle, |p| {
    ///         println!("{:.0}% | {}", p.percent(), p.speed_human());
    ///     })
    ///     .await?;
    /// # Ok(()) }
    /// ```
    pub async fn upload_with_progress(
        &self,
        source: impl tokio::io::AsyncRead + Unpin + Send,
        name: &str,
        handle: &TransferHandle,
        mut on_progress: impl FnMut(TransferProgress) + Send + 'static,
    ) -> Result<media::UploadedFile, InvocationError> {
        let span = tracing::info_span!(
            target: "ferogram::transfer",
            "upload",
            name,
            total = tracing::field::Empty,
        );
        let _enter = span.enter();

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

        let result = self.upload(source, name, Some(handle)).await;
        done.store(true, Ordering::Release);
        result
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
    /// Falls back to `download_with_progress` silently if
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
                .download_with_progress(media, dest as &mut Vec<u8>, handle, on_progress)
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
            .download_streaming_on_dc_from(loc.clone(), dc, &mut tail, Some(handle), resume_offset)
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
    /// Falls back to `upload_with_progress` silently if
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
                .upload_with_progress(std::io::Cursor::new(data), name, handle, on_progress)
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
                tracing::info!(
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
                tracing::info!(target: "ferogram::transfer", "upload: checkpoint expired or incompatible, restarting");
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
        tracing::info!(target: "ferogram::transfer", name, total_parts, "upload complete, checkpoint purged");

        Ok(media::UploadedFile::new(
            inner,
            resolved_mime,
            name.to_string(),
        ))
    }
    /// Upload a file from disk by path, streaming chunks without loading the whole
    /// file into memory.
    ///
    /// Unlike `upload_file` (which reads the entire file into a `Vec<u8>` first),
    /// this method reads one chunk at a time, uploads it, and discards it.
    /// Safe to use with files larger than available RAM.
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// use ferogram::{Client, TransferHandle};
    ///
    /// # async fn example(client: Client) -> anyhow::Result<()> {
    /// let handle = TransferHandle::new();
    /// let uploaded = client
    ///     .upload_file_streaming("big_video.mp4", Some(&handle))
    ///     .await?;
    /// # Ok(()) }
    /// ```
    pub async fn upload_file_streaming(
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

    /// Stream-upload a file from disk with a per-second progress callback.
    ///
    /// Same as `upload_file_streaming` but calls `on_progress` every second.
    /// For async work (editing a Telegram message) pair with a channel.
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// use ferogram::{Client, TransferHandle};
    ///
    /// # async fn example(client: Client) -> anyhow::Result<()> {
    /// let handle = TransferHandle::new();
    /// let uploaded = client
    ///     .upload_file_streaming_with_progress("big_video.mp4", &handle, |p| {
    ///         println!("{:.0}% | {}", p.percent(), p.speed_human());
    ///     })
    ///     .await?;
    /// # Ok(()) }
    /// ```
    pub async fn upload_file_streaming_with_progress(
        &self,
        path: impl AsRef<std::path::Path>,
        handle: &TransferHandle,
        mut on_progress: impl FnMut(TransferProgress) + Send + 'static,
    ) -> Result<media::UploadedFile, InvocationError> {
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

        let result = self.upload_file_streaming(path, Some(handle)).await;
        done.store(true, Ordering::Release);
        result
    }

    /// Download media to a file path with a per-second progress callback.
    ///
    /// Streams directly to disk; no memory buffer. Safe for large files.
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// use ferogram::{Client, TransferHandle};
    ///
    /// # async fn example(client: Client, media: ferogram_tl_types::enums::MessageMedia) -> anyhow::Result<()> {
    /// let handle = TransferHandle::new();
    /// client
    ///     .download_file_with_progress(&media, "video.mp4", &handle, |p| {
    ///         println!("{:.0}% | {}", p.percent(), p.speed_human());
    ///     })
    ///     .await?;
    /// # Ok(()) }
    /// ```
    pub async fn download_file_with_progress(
        &self,
        media: &tl::enums::MessageMedia,
        path: impl AsRef<std::path::Path>,
        handle: &TransferHandle,
        mut on_progress: impl FnMut(TransferProgress) + Send + 'static,
    ) -> Result<u64, InvocationError> {
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

        let result = self.download_file(media, path, Some(handle)).await;
        done.store(true, Ordering::Release);
        result
    }
}
