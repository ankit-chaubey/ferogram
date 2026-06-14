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

//! Transfer progress tracking, pause/resume/cancel controls, and typed transfer errors.
//!
//! # Typed errors
//!
//! Transfer operations return `Result<_, TransferError>` so callers can tell a
//! user cancellation apart from a network failure without matching on error strings:
//!
//! ```rust,no_run
//! use ferogram::{Client, TransferHandle, TransferError, InvocationErrorExt};
//!
//! # async fn example(client: Client, media: ferogram_tl_types::enums::MessageMedia) -> anyhow::Result<()> {
//! let handle = TransferHandle::new();
//! let mut buf = Vec::new();
//! match client.download_with_progress(&media, &mut buf, &handle, |_| {}).await {
//!     Ok(_) => {}
//!     Err(e) if matches!(e.kind(), ferogram::ErrorKind::Transfer) => println!("transfer error: {}", e.friendly()),
//!     Err(e) => println!("other: {e}"),
//! }
//! # Ok(()) }
//! ```
//!
//! `TransferError` implements `From<TransferError> for InvocationError`, so
//! existing call sites using `?` on `Result<_, InvocationError>` compile without changes.
//!
//! # Example
//!
//! ```rust,no_run
//! use ferogram::{Client, TransferHandle};
//!
//! # async fn example(client: Client) -> anyhow::Result<()> {
//! let handle = TransferHandle::new();
//! let ctl = handle.clone();
//!
//! // Spawn a task that cancels after 5 seconds
//! tokio::spawn(async move {
//!     tokio::time::sleep(std::time::Duration::from_secs(5)).await;
//!     ctl.cancel();
//! });
//!
//! let uploaded = client
//!     .upload_file_with_handle("/tmp/video.mp4", Some(&handle))
//!     .await?;
//! # Ok(()) }
//! ```

use std::fmt;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

// TransferError

/// Typed error returned by upload and download operations.
///
/// Unlike [`crate::InvocationError`], this lets callers tell a user-initiated
/// cancel apart from a network failure, a rate limit, or an RPC error without
/// matching on error message strings.
#[derive(Debug)]
#[non_exhaustive]
pub enum TransferError {
    /// The transfer was cancelled by the caller via [`TransferHandle::cancel`].
    Cancelled,
    /// A network or I/O error occurred.
    Network(std::io::Error),
    /// Telegram returned an RPC error during the transfer.
    Rpc {
        /// HTTP-style error code (e.g. 400, 420, 500).
        code: i32,
        /// Telegram error name (e.g. `FILE_PART_INVALID`).
        name: String,
    },
    /// Telegram rate-limited the transfer. Retry after `seconds`.
    FloodWait { seconds: u64 },
    /// The checkpoint store could not be opened or written.
    Checkpoint(std::io::Error),
    /// Any other error not covered above.
    Other(crate::InvocationError),
}

impl fmt::Display for TransferError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Cancelled => write!(f, "transfer cancelled by caller"),
            Self::Network(e) => write!(f, "network error: {e}"),
            Self::Rpc { code, name } => write!(f, "Telegram error ({code}): {name}"),
            Self::FloodWait { seconds } => {
                write!(
                    f,
                    "Telegram rate limit reached. Retry after {seconds} seconds."
                )
            }
            Self::Checkpoint(e) => write!(f, "checkpoint I/O error: {e}"),
            Self::Other(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for TransferError {}

impl From<TransferError> for crate::InvocationError {
    fn from(e: TransferError) -> Self {
        match e {
            TransferError::Cancelled => {
                crate::InvocationError::Deserialize("transfer cancelled by caller".into())
            }
            TransferError::Network(io) => crate::InvocationError::Io(io),
            TransferError::Checkpoint(io) => crate::InvocationError::Io(io),
            TransferError::Rpc { code, name } => crate::InvocationError::Rpc(crate::RpcError {
                code,
                name,
                value: None,
            }),
            TransferError::FloodWait { seconds } => crate::InvocationError::Rpc(crate::RpcError {
                code: 420,
                name: format!("FLOOD_WAIT_{seconds}"),
                value: Some(seconds as u32),
            }),
            TransferError::Other(e) => e,
        }
    }
}

impl From<crate::InvocationError> for TransferError {
    fn from(e: crate::InvocationError) -> Self {
        match &e {
            crate::InvocationError::Io(_) => {
                if let crate::InvocationError::Io(io) = e {
                    return TransferError::Network(io);
                }
                unreachable!()
            }
            crate::InvocationError::Dropped => TransferError::Network(std::io::Error::new(
                std::io::ErrorKind::ConnectionReset,
                "connection dropped",
            )),
            crate::InvocationError::Rpc(rpc) => {
                if rpc.code == 420 {
                    return TransferError::FloodWait {
                        seconds: rpc.value.unwrap_or(0) as u64,
                    };
                }
                TransferError::Rpc {
                    code: rpc.code,
                    name: rpc.name.clone(),
                }
            }
            crate::InvocationError::Deserialize(s) if s.contains("cancel") => {
                TransferError::Cancelled
            }
            _ => TransferError::Other(e),
        }
    }
}

/// Shared control block for a running transfer.
///
/// Clone freely; all clones point to the same transfer.
/// Use [`TransferHandle::pause`], [`TransferHandle::resume`], and
/// [`TransferHandle::cancel`] from any thread or task.
#[derive(Clone, Debug)]
pub struct TransferHandle {
    inner: Arc<TransferState>,
}

#[derive(Debug)]
struct TransferState {
    paused: AtomicBool,
    cancelled: AtomicBool,
    bytes_done: AtomicU64,
    total: AtomicU64,
    start_ms: AtomicU64, // unix ms when transfer started
}

impl TransferHandle {
    /// Create a new, unstarted transfer handle.
    pub fn new() -> Self {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;
        Self {
            inner: Arc::new(TransferState {
                paused: AtomicBool::new(false),
                cancelled: AtomicBool::new(false),
                bytes_done: AtomicU64::new(0),
                total: AtomicU64::new(0),
                start_ms: AtomicU64::new(now),
            }),
        }
    }

    // Control API

    /// Pause the transfer. The in-progress chunk finishes, then the worker waits.
    pub fn pause(&self) {
        self.inner.paused.store(true, Ordering::Release);
    }

    /// Resume a paused transfer.
    pub fn resume(&self) {
        self.inner.paused.store(false, Ordering::Release);
    }

    /// Cancel the transfer. The worker returns `Err(TransferCancelled)` after
    /// the current chunk.
    pub fn cancel(&self) {
        self.inner.cancelled.store(true, Ordering::Release);
    }

    // State queries

    pub fn is_paused(&self) -> bool {
        self.inner.paused.load(Ordering::Acquire)
    }

    pub fn is_cancelled(&self) -> bool {
        self.inner.cancelled.load(Ordering::Acquire)
    }

    /// Current progress snapshot.
    pub fn progress(&self) -> TransferProgress {
        let done = self.inner.bytes_done.load(Ordering::Relaxed);
        let total = self.inner.total.load(Ordering::Relaxed);
        let start_ms = self.inner.start_ms.load(Ordering::Relaxed);
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;
        let elapsed_ms = now_ms.saturating_sub(start_ms).max(1);
        TransferProgress {
            done,
            total,
            elapsed_ms,
        }
    }

    // Internal helpers (used by upload/download impls)

    pub(crate) fn set_total(&self, total: u64) {
        self.inner.total.store(total, Ordering::Relaxed);
    }

    pub(crate) fn add_bytes(&self, n: u64) {
        self.inner.bytes_done.fetch_add(n, Ordering::Relaxed);
    }

    pub(crate) fn reset_start(&self) {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;
        self.inner.start_ms.store(now, Ordering::Relaxed);
    }

    /// Poll pause flag; yields the executor while paused.
    ///
    /// Returns `Err(TransferError::Cancelled)` if [`cancel`](TransferHandle::cancel)
    /// was called. Callers convert with `?`; `TransferError` implements
    /// `From<TransferError> for InvocationError` so existing signatures compile unchanged.
    pub async fn poll_pause_cancel(&self) -> Result<(), TransferError> {
        loop {
            if self.is_cancelled() {
                tracing::debug!(target: "ferogram::transfer", "transfer cancelled by caller");
                return Err(TransferError::Cancelled);
            }
            if !self.is_paused() {
                return Ok(());
            }
            tracing::trace!(target: "ferogram::transfer", "transfer paused, waiting");
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
    }
}

impl Default for TransferHandle {
    fn default() -> Self {
        Self::new()
    }
}

/// A snapshot of transfer progress at a single point in time.
#[derive(Debug, Clone, Copy)]
pub struct TransferProgress {
    /// Bytes transferred so far.
    pub done: u64,
    /// Total bytes (0 if unknown).
    pub total: u64,
    /// Milliseconds elapsed since transfer started.
    pub elapsed_ms: u64,
}

impl TransferProgress {
    /// Completion percentage (0.0-100.0). Returns 0.0 if total is unknown.
    pub fn percent(&self) -> f64 {
        if self.total == 0 {
            return 0.0;
        }
        (self.done as f64 / self.total as f64 * 100.0).min(100.0)
    }

    /// Transfer speed in bytes per second.
    pub fn speed_bps(&self) -> u64 {
        let elapsed_s = self.elapsed_ms.max(1) as f64 / 1000.0;
        (self.done as f64 / elapsed_s) as u64
    }

    /// Estimated seconds remaining. Returns 0 if speed is 0 or done = total.
    pub fn eta_secs(&self) -> u64 {
        if self.total == 0 || self.done >= self.total {
            return 0;
        }
        let remaining = self.total - self.done;
        let speed = self.speed_bps().max(1);
        remaining / speed
    }

    /// Human-readable speed string, e.g. `"1.4 MB/s"`.
    pub fn speed_human(&self) -> String {
        let bps = self.speed_bps();
        if bps >= 1024 * 1024 {
            format!("{:.1} MB/s", bps as f64 / (1024.0 * 1024.0))
        } else if bps >= 1024 {
            format!("{:.1} KB/s", bps as f64 / 1024.0)
        } else {
            format!("{bps} B/s")
        }
    }

    /// Human-readable progress string, e.g. `"12.3 MB / 50.0 MB"`.
    pub fn bytes_human(&self) -> String {
        format!("{} / {}", fmt_bytes(self.done), fmt_bytes(self.total))
    }
}

fn fmt_bytes(b: u64) -> String {
    if b >= 1024 * 1024 * 1024 {
        format!("{:.1} GB", b as f64 / (1024.0 * 1024.0 * 1024.0))
    } else if b >= 1024 * 1024 {
        format!("{:.1} MB", b as f64 / (1024.0 * 1024.0))
    } else if b >= 1024 {
        format!("{:.1} KB", b as f64 / 1024.0)
    } else {
        format!("{b} B")
    }
}
