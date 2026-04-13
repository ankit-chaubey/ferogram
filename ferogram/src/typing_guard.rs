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

//! RAII typing indicator guard.
//!
//! [`TypingGuard`] automatically cancels the "typing..." chat action when
//! dropped, eliminating the need to remember to call `send_chat_action`
//! with `SendMessageAction::SendMessageCancelAction` manually.
//!
//! # Example
//! ```rust,no_run
//! use ferogram::{Client, TypingGuard};
//! use ferogram_tl_types as tl;
//!
//! async fn handle(client: Client, peer: tl::enums::Peer) {
//! // Typing indicator is sent immediately and auto-cancelled on drop.
//! let _typing = TypingGuard::start(&client, peer.clone(),
//!     tl::enums::SendMessageAction::SendMessageTypingAction).await.unwrap();
//!
//! do_expensive_work().await;
//! // `_typing` is dropped here: Telegram sees the typing stop.
//! }
//! # async fn do_expensive_work() {}
//! ```

use crate::{Client, InvocationError, PeerRef};
use ferogram_tl_types as tl;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Notify;
use tokio::task::JoinHandle;

// TypingGuard

/// Scoped typing indicator.  Keeps the action alive by re-sending it every
/// ~4 seconds (Telegram drops the indicator after ~5 s).
///
/// Drop this guard to cancel the action immediately.
pub struct TypingGuard {
    stop: Arc<Notify>,
    task: Option<JoinHandle<()>>,
}

impl TypingGuard {
    /// Send `action` to `peer` and keep repeating it until the guard is dropped.
    pub async fn start(
        client: &Client,
        peer: impl Into<PeerRef>,
        action: tl::enums::SendMessageAction,
    ) -> Result<Self, InvocationError> {
        let peer = peer.into().resolve(client).await?;
        Self::start_ex(client, peer, action, None, Duration::from_secs(4)).await
    }

    /// Like [`start`](Self::start) but also accepts a forum **topic id**
    /// (`top_msg_id`) and a custom **repeat delay**.
    ///
    /// # Arguments
    /// * `topic_id`    : `Some(msg_id)` for a forum topic thread; `None` for
    ///   the main chat.
    /// * `repeat_delay`: How often to re-send the action to keep it alive.
    ///   Telegram drops the indicator after ~5 s; ≤ 4 s is
    ///   recommended.
    pub async fn start_ex(
        client: &Client,
        peer: tl::enums::Peer,
        action: tl::enums::SendMessageAction,
        topic_id: Option<i32>,
        repeat_delay: Duration,
    ) -> Result<Self, InvocationError> {
        // Send once immediately so the indicator appears without delay.
        client
            .send_chat_action_ex(peer.clone(), action.clone(), topic_id)
            .await?;

        let stop = Arc::new(Notify::new());
        let stop2 = stop.clone();
        let client = client.clone();

        let task = tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = tokio::time::sleep(repeat_delay) => {
                        if let Err(e) = client.send_chat_action_ex(peer.clone(), action.clone(), topic_id).await {
                            tracing::warn!("[typing_guard] Failed to refresh typing action: {e}");
                            break;
                        }
                    }
                    _ = stop2.notified() => break,
                }
            }
            // Cancel the action
            let cancel = tl::enums::SendMessageAction::SendMessageCancelAction;
            let _ = client
                .send_chat_action_ex(peer.clone(), cancel, topic_id)
                .await;
        });

        Ok(Self {
            stop,
            task: Some(task),
        })
    }

    /// Cancel the typing indicator immediately without waiting for the drop.
    pub fn cancel(&mut self) {
        self.stop.notify_one();
    }
}

impl Drop for TypingGuard {
    fn drop(&mut self) {
        self.stop.notify_one();
        if let Some(t) = self.task.take() {
            t.abort();
        }
    }
}

// Client extension

impl Client {
    /// Start a scoped typing indicator that auto-cancels when dropped.
    ///
    /// A convenience wrapper around [`TypingGuard::start`].
    pub async fn typing(&self, peer: impl Into<PeerRef>) -> Result<TypingGuard, InvocationError> {
        TypingGuard::start(
            self,
            peer,
            tl::enums::SendMessageAction::SendMessageTypingAction,
        )
        .await
    }

    /// Start a scoped typing indicator in a **forum topic** thread.
    ///
    /// `topic_id` is the `top_msg_id` of the forum topic.
    pub async fn typing_in_topic(
        &self,
        peer: impl Into<PeerRef>,
        topic_id: i32,
    ) -> Result<TypingGuard, InvocationError> {
        let peer = peer.into().resolve(self).await?;
        TypingGuard::start_ex(
            self,
            peer,
            tl::enums::SendMessageAction::SendMessageTypingAction,
            Some(topic_id),
            std::time::Duration::from_secs(4),
        )
        .await
    }

    /// Start a scoped "uploading document" action that auto-cancels when dropped.
    pub async fn uploading_document(
        &self,
        peer: impl Into<PeerRef>,
    ) -> Result<TypingGuard, InvocationError> {
        TypingGuard::start(
            self,
            peer,
            tl::enums::SendMessageAction::SendMessageUploadDocumentAction(
                tl::types::SendMessageUploadDocumentAction { progress: 0 },
            ),
        )
        .await
    }

    /// Start a scoped "recording video" action that auto-cancels when dropped.
    pub async fn recording_video(
        &self,
        peer: impl Into<PeerRef>,
    ) -> Result<TypingGuard, InvocationError> {
        TypingGuard::start(
            self,
            peer,
            tl::enums::SendMessageAction::SendMessageRecordVideoAction,
        )
        .await
    }

    /// Send a chat action with optional forum topic support (internal helper).
    pub(crate) async fn send_chat_action_ex(
        &self,
        peer: tl::enums::Peer,
        action: tl::enums::SendMessageAction,
        topic_id: Option<i32>,
    ) -> Result<(), InvocationError> {
        let input_peer = self.inner.peer_cache.peer_to_input(&peer);
        let req = tl::functions::messages::SetTyping {
            peer: input_peer,
            top_msg_id: topic_id,
            action,
        };
        self.rpc_write(&req).await
    }
}
