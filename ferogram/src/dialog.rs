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

use std::collections::VecDeque;

use ferogram_tl_types as tl;

use crate::Client;
use crate::errors::InvocationError;
use crate::peer_ref::PeerRef;
use crate::update;

/// A Telegram dialog (chat, user, channel).
#[derive(Debug, Clone)]
pub struct Dialog {
    pub raw: tl::enums::Dialog,
    pub message: Option<tl::enums::Message>,
    pub entity: Option<tl::enums::User>,
    pub chat: Option<tl::enums::Chat>,
}

impl Dialog {
    /// The dialog's display title.
    pub fn title(&self) -> String {
        if let Some(tl::enums::User::User(u)) = &self.entity {
            let first = u.first_name.as_deref().unwrap_or("");
            let last = u.last_name.as_deref().unwrap_or("");
            let name = format!("{first} {last}").trim().to_string();
            if !name.is_empty() {
                return name;
            }
        }
        if let Some(chat) = &self.chat {
            return match chat {
                tl::enums::Chat::Chat(c) => c.title.clone(),
                tl::enums::Chat::Forbidden(c) => c.title.clone(),
                tl::enums::Chat::Channel(c) => c.title.clone(),
                tl::enums::Chat::ChannelForbidden(c) => c.title.clone(),
                tl::enums::Chat::Empty(_) => "(empty)".into(),
            };
        }
        "(Unknown)".to_string()
    }

    /// Peer of this dialog.
    pub fn peer(&self) -> Option<&tl::enums::Peer> {
        match &self.raw {
            tl::enums::Dialog::Dialog(d) => Some(&d.peer),
            tl::enums::Dialog::Folder(_) => None,
        }
    }

    /// Unread message count.
    pub fn unread_count(&self) -> i32 {
        match &self.raw {
            tl::enums::Dialog::Dialog(d) => d.unread_count,
            _ => 0,
        }
    }

    /// ID of the top message.
    pub fn top_message(&self) -> i32 {
        match &self.raw {
            tl::enums::Dialog::Dialog(d) => d.top_message,
            _ => 0,
        }
    }
}

/// Cursor-based iterator over dialogs. Created by [`Client::iter_dialogs`].
pub struct DialogIter {
    pub(crate) offset_date: i32,
    pub(crate) offset_id: i32,
    pub(crate) offset_peer: tl::enums::InputPeer,
    pub(crate) done: bool,
    pub(crate) buffer: VecDeque<Dialog>,
    /// Total dialog count as reported by the first server response.
    /// `None` until the first page is fetched.
    pub total: Option<i32>,
}

impl DialogIter {
    const PAGE_SIZE: i32 = 100;

    /// Total number of dialogs as reported by the server on the first page fetch.
    ///
    /// Returns `None` before the first [`next`](Self::next) call, and `None` for
    /// accounts with fewer dialogs than `PAGE_SIZE` (where the server returns
    /// `messages.Dialogs` instead of `messages.DialogsSlice`).
    pub fn total(&self) -> Option<i32> {
        self.total
    }

    /// Fetch the next dialog. Returns `None` when all dialogs have been yielded.
    pub async fn next(&mut self, client: &Client) -> Result<Option<Dialog>, InvocationError> {
        if let Some(d) = self.buffer.pop_front() {
            return Ok(Some(d));
        }
        if self.done {
            return Ok(None);
        }

        let req = tl::functions::messages::GetDialogs {
            exclude_pinned: false,
            folder_id: None,
            offset_date: self.offset_date,
            offset_id: self.offset_id,
            offset_peer: self.offset_peer.clone(),
            limit: Self::PAGE_SIZE,
            hash: 0,
        };

        let (dialogs, count): (Vec<crate::Dialog>, Option<i32>) =
            client.get_dialogs_raw_with_count(req).await?;
        // Populate total from the first response (messages.DialogsSlice carries a count).
        if self.total.is_none() {
            self.total = count;
        }
        if dialogs.is_empty() || dialogs.len() < Self::PAGE_SIZE as usize {
            self.done = true;
        }

        // Prepare cursor for next page
        if let Some(last) = dialogs.last() {
            self.offset_date = last
                .message
                .as_ref()
                .map(|m| match m {
                    tl::enums::Message::Message(x) => x.date,
                    tl::enums::Message::Service(x) => x.date,
                    _ => 0,
                })
                .unwrap_or(0);
            self.offset_id = last.top_message();
            if let Some(peer) = last.peer() {
                self.offset_peer = client.inner.peer_cache.read().await.peer_to_input(peer)?;
            }
        }

        self.buffer.extend(dialogs);
        Ok(self.buffer.pop_front())
    }
}

/// Cursor-based iterator over message history. Created by [`Client::iter_messages`].
pub struct MessageIter {
    pub(crate) unresolved: Option<PeerRef>,
    pub(crate) peer: Option<tl::enums::Peer>,
    pub(crate) offset_id: i32,
    pub(crate) done: bool,
    pub(crate) buffer: VecDeque<update::IncomingMessage>,
    /// Total message count from the first server response (messages.Slice).
    /// `None` until the first page is fetched, `None` for `messages.Messages`
    /// (which returns an exact slice with no separate count).
    pub total: Option<i32>,
}

impl MessageIter {
    const PAGE_SIZE: i32 = 100;

    /// Total message count from the first server response.
    ///
    /// Returns `None` before the first [`next`](Self::next) call, or for chats
    /// where the server returns an exact (non-slice) response.
    pub fn total(&self) -> Option<i32> {
        self.total
    }

    /// Fetch the next message (newest first). Returns `None` when all messages have been yielded.
    pub async fn next(
        &mut self,
        client: &Client,
    ) -> Result<Option<update::IncomingMessage>, InvocationError> {
        if let Some(m) = self.buffer.pop_front() {
            return Ok(Some(m));
        }
        if self.done {
            return Ok(None);
        }

        // Resolve PeerRef on first call, then reuse the cached Peer.
        let peer = if let Some(p) = &self.peer {
            p.clone()
        } else {
            let pr = self.unresolved.take().expect("MessageIter: peer not set");
            let p = pr.resolve(client).await?;
            self.peer = Some(p.clone());
            p
        };

        let input_peer = client.inner.peer_cache.read().await.peer_to_input(&peer)?;
        let (page, count): (Vec<crate::update::IncomingMessage>, Option<i32>) = client
            .get_messages_with_count(input_peer, Self::PAGE_SIZE, self.offset_id)
            .await?;

        if self.total.is_none() {
            self.total = count;
        }

        if page.is_empty() || page.len() < Self::PAGE_SIZE as usize {
            self.done = true;
        }
        if let Some(last) = page.last() {
            self.offset_id = last.id();
        }

        self.buffer.extend(page);
        Ok(self.buffer.pop_front())
    }
}
