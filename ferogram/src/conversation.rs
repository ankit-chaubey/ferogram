// Copyright (c) Ankit Chaubey <ankitchaubey.dev@gmail.com>
// SPDX-License-Identifier: MIT OR Apache-2.0
//
// ferogram: async Telegram MTProto client in Rust
// https://github.com/ankit-chaubey/ferogram
//
// If you use or modify this code, keep this notice at the top of your file
// and include the LICENSE-MIT or LICENSE-APACHE file from this repository:
// https://github.com/ankit-chaubey/ferogram

use std::time::Duration;

use tokio::time::timeout;

use crate::update::{self, CallbackQuery, IncomingMessage};
use crate::{Client, InvocationError, PeerRef, UpdateStream};
use ferogram_tl_types as tl;

/// Error returned by [`Conversation`] methods.
#[derive(Debug)]
pub enum ConversationError {
    /// No response arrived within the allotted time.
    Timeout(Duration),
    /// The update stream was closed unexpectedly.
    StreamClosed,
    /// An underlying Telegram API error.
    Invocation(InvocationError),
}

impl std::fmt::Display for ConversationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Timeout(d) => write!(f, "conversation timed out after {d:?}"),
            Self::StreamClosed => write!(f, "update stream closed"),
            Self::Invocation(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for ConversationError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        if let Self::Invocation(e) = self {
            Some(e)
        } else {
            None
        }
    }
}

impl From<InvocationError> for ConversationError {
    fn from(e: InvocationError) -> Self {
        Self::Invocation(e)
    }
}

/// A stateful conversation with a single peer.
///
/// Wraps a [`&mut UpdateStream`](UpdateStream) for the conversation's lifetime.
/// Updates from other peers are buffered; retrieve them with
/// [`drain_buffered`](Self::drain_buffered).
pub struct Conversation<'a> {
    client: Client,
    peer: tl::enums::Peer,
    stream: &'a mut UpdateStream,
    buffered: Vec<update::Update>,
}

impl<'a> Conversation<'a> {
    /// Open a conversation with `peer` using an existing update stream.
    pub async fn new(
        client: &Client,
        stream: &'a mut UpdateStream,
        peer: impl Into<PeerRef>,
    ) -> Result<Self, ConversationError> {
        let peer = peer.into().resolve(client).await?;
        Ok(Self {
            client: client.clone(),
            peer,
            stream,
            buffered: Vec::new(),
        })
    }

    // Sending

    /// Send a message to the conversation peer.
    pub async fn ask(&self, text: impl Into<String>) -> Result<IncomingMessage, ConversationError> {
        let s: String = text.into();
        Ok(self
            .client
            .send_message_to_peer(self.peer.clone(), &s)
            .await?)
    }

    /// Send a message (alias for [`ask`](Self::ask)).
    pub async fn respond(
        &self,
        text: impl Into<String>,
    ) -> Result<IncomingMessage, ConversationError> {
        self.ask(text).await
    }

    // Waiting

    /// Wait for the next message from this peer within `deadline`.
    ///
    /// Non-matching updates are buffered in [`drain_buffered`](Self::drain_buffered).
    pub async fn get_response(
        &mut self,
        deadline: Duration,
    ) -> Result<IncomingMessage, ConversationError> {
        let start = tokio::time::Instant::now();
        loop {
            let remaining = deadline.checked_sub(start.elapsed()).unwrap_or_default();
            if remaining.is_zero() {
                return Err(ConversationError::Timeout(deadline));
            }
            match timeout(remaining, self.stream.next()).await {
                Err(_) => return Err(ConversationError::Timeout(deadline)),
                Ok(None) => return Err(ConversationError::StreamClosed),
                Ok(Some(upd)) => match upd {
                    update::Update::NewMessage(ref msg)
                        if peer_matches(msg.peer_id(), &self.peer) =>
                    {
                        return Ok(msg.clone());
                    }
                    other => self.buffered.push(other),
                },
            }
        }
    }

    /// Wait for the peer to click an inline button within `deadline`.
    pub async fn wait_click(
        &mut self,
        deadline: Duration,
    ) -> Result<CallbackQuery, ConversationError> {
        let start = tokio::time::Instant::now();
        loop {
            let remaining = deadline.checked_sub(start.elapsed()).unwrap_or_default();
            if remaining.is_zero() {
                return Err(ConversationError::Timeout(deadline));
            }
            match timeout(remaining, self.stream.next()).await {
                Err(_) => return Err(ConversationError::Timeout(deadline)),
                Ok(None) => return Err(ConversationError::StreamClosed),
                Ok(Some(upd)) => match upd {
                    update::Update::CallbackQuery(ref cb) if cb_peer_matches(cb, &self.peer) => {
                        return Ok(cb.clone());
                    }
                    other => self.buffered.push(other),
                },
            }
        }
    }

    /// Wait for a read receipt (any non-message update from peer) within `deadline`.
    pub async fn wait_read(&mut self, deadline: Duration) -> Result<(), ConversationError> {
        let start = tokio::time::Instant::now();
        loop {
            let remaining = deadline.checked_sub(start.elapsed()).unwrap_or_default();
            if remaining.is_zero() {
                return Err(ConversationError::Timeout(deadline));
            }
            match timeout(remaining, self.stream.next()).await {
                Err(_) => return Err(ConversationError::Timeout(deadline)),
                Ok(None) => return Err(ConversationError::StreamClosed),
                Ok(Some(upd)) => {
                    if let update::Update::Raw(_) = &upd {
                        return Ok(());
                    }
                    self.buffered.push(upd);
                }
            }
        }
    }

    /// Ask a question and immediately wait for the reply.
    pub async fn ask_and_wait(
        &mut self,
        text: impl Into<String>,
        deadline: Duration,
    ) -> Result<IncomingMessage, ConversationError> {
        self.ask(text).await?;
        self.get_response(deadline).await
    }

    // Introspection

    /// The resolved peer for this conversation.
    pub fn peer(&self) -> &tl::enums::Peer {
        &self.peer
    }

    /// Drain updates buffered from other peers while we were waiting.
    ///
    /// Process these after the conversation to avoid missing events.
    pub fn drain_buffered(&mut self) -> Vec<update::Update> {
        std::mem::take(&mut self.buffered)
    }
}

// Helpers

fn peer_matches(msg_peer: Option<&tl::enums::Peer>, conv_peer: &tl::enums::Peer) -> bool {
    match (msg_peer, conv_peer) {
        (Some(tl::enums::Peer::User(a)), tl::enums::Peer::User(b)) => a.user_id == b.user_id,
        (Some(tl::enums::Peer::Chat(a)), tl::enums::Peer::Chat(b)) => a.chat_id == b.chat_id,
        (Some(tl::enums::Peer::Channel(a)), tl::enums::Peer::Channel(b)) => {
            a.channel_id == b.channel_id
        }
        _ => false,
    }
}

fn cb_peer_matches(cb: &CallbackQuery, conv_peer: &tl::enums::Peer) -> bool {
    match (&cb.chat_peer, conv_peer) {
        (Some(tl::enums::Peer::User(a)), tl::enums::Peer::User(b)) => a.user_id == b.user_id,
        (Some(tl::enums::Peer::Chat(a)), tl::enums::Peer::Chat(b)) => a.chat_id == b.chat_id,
        (Some(tl::enums::Peer::Channel(a)), tl::enums::Peer::Channel(b)) => {
            a.channel_id == b.channel_id
        }
        _ => {
            if let tl::enums::Peer::User(u) = conv_peer {
                cb.user_id == u.user_id
            } else {
                false
            }
        }
    }
}
