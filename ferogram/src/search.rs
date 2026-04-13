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

//! Fluent search builders.
//!
//! # In-chat search
//! ```rust,no_run
//! # async fn f(client: ferogram::Client, peer: ferogram_tl_types::enums::Peer)
//! # -> Result<(), Box<dyn std::error::Error>> {
//! let results = client
//! .search(peer, "hello world")
//! .min_date(1_700_000_000)
//! .max_date(1_720_000_000)
//! .filter(ferogram_tl_types::enums::MessagesFilter::InputMessagesFilterPhotos)
//! .limit(50)
//! .fetch(&client)
//! .await?;
//! # Ok(()) }
//! ```
//!
//! # Global search
//! ```rust,no_run
//! # async fn f(client: ferogram::Client)
//! # -> Result<(), Box<dyn std::error::Error>> {
//! let results = client
//! .search_global_builder("rust async")
//! .broadcasts_only(true)
//! .min_date(1_700_000_000)
//! .limit(30)
//! .fetch(&client)
//! .await?;
//! # Ok(()) }
//! ```

use ferogram_tl_types::{self as tl, Cursor, Deserializable};

use crate::{Client, InvocationError, PeerRef, update};

/// Fluent builder for `messages.search` (in-chat message search).
///
/// Created by [`Client::search`]. All setters are chainable; call
/// [`fetch`] to execute.
///
/// [`fetch`]: SearchBuilder::fetch
pub struct SearchBuilder {
    peer: PeerRef,
    query: String,
    filter: tl::enums::MessagesFilter,
    min_date: i32,
    max_date: i32,
    offset_id: i32,
    add_offset: i32,
    limit: i32,
    max_id: i32,
    min_id: i32,
    from_id: Option<tl::enums::InputPeer>,
    top_msg_id: Option<i32>,
}

impl SearchBuilder {
    pub(crate) fn new(peer: PeerRef, query: String) -> Self {
        Self {
            peer,
            query,
            filter: tl::enums::MessagesFilter::InputMessagesFilterEmpty,
            min_date: 0,
            max_date: 0,
            offset_id: 0,
            add_offset: 0,
            limit: 100,
            max_id: 0,
            min_id: 0,
            from_id: None,
            top_msg_id: None,
        }
    }

    /// Only return messages on or after this Unix timestamp.
    pub fn min_date(mut self, ts: i32) -> Self {
        self.min_date = ts;
        self
    }

    /// Only return messages on or before this Unix timestamp.
    pub fn max_date(mut self, ts: i32) -> Self {
        self.max_date = ts;
        self
    }

    /// Apply a `MessagesFilter` (e.g. photos only, video only, etc.).
    pub fn filter(mut self, f: tl::enums::MessagesFilter) -> Self {
        self.filter = f;
        self
    }

    /// Maximum number of messages to return (default 100).
    pub fn limit(mut self, n: i32) -> Self {
        self.limit = n;
        self
    }

    /// Start from this message ID (for pagination).
    pub fn offset_id(mut self, id: i32) -> Self {
        self.offset_id = id;
        self
    }

    /// Additional offset for fine-grained pagination.
    pub fn add_offset(mut self, off: i32) -> Self {
        self.add_offset = off;
        self
    }

    /// Only return messages with an ID ≤ `max_id`.
    pub fn max_id(mut self, id: i32) -> Self {
        self.max_id = id;
        self
    }

    /// Only return messages with an ID ≥ `min_id`.
    pub fn min_id(mut self, id: i32) -> Self {
        self.min_id = id;
        self
    }

    /// Restrict to messages sent by this peer (resolved against the cache).
    /// Only return messages sent by the logged-in user.
    pub fn sent_by_self(mut self) -> Self {
        self.from_id = Some(tl::enums::InputPeer::PeerSelf);
        self
    }

    /// Only return messages sent by a specific peer.
    pub fn from_peer(mut self, peer: tl::enums::InputPeer) -> Self {
        self.from_id = Some(peer);
        self
    }

    /// Restrict search to a specific forum topic.
    pub fn top_msg_id(mut self, id: i32) -> Self {
        self.top_msg_id = Some(id);
        self
    }

    /// Execute the search and return matching messages.
    pub async fn fetch(
        self,
        client: &Client,
    ) -> Result<Vec<update::IncomingMessage>, InvocationError> {
        let peer = self.peer.resolve(client).await?;
        let input_peer = client.inner.peer_cache.peer_to_input(&peer);
        let req = tl::functions::messages::Search {
            peer: input_peer,
            q: self.query,
            from_id: self.from_id,
            saved_peer_id: None,
            saved_reaction: None,
            top_msg_id: self.top_msg_id,
            filter: self.filter,
            min_date: self.min_date,
            max_date: self.max_date,
            offset_id: self.offset_id,
            add_offset: self.add_offset,
            limit: self.limit,
            max_id: self.max_id,
            min_id: self.min_id,
            hash: 0,
        };
        let body = client.rpc_call_raw(&req).await?;
        let mut cur = Cursor::from_slice(&body);
        let msgs = match tl::enums::messages::Messages::deserialize(&mut cur)? {
            tl::enums::messages::Messages::Messages(m) => m.messages,
            tl::enums::messages::Messages::Slice(m) => m.messages,
            tl::enums::messages::Messages::ChannelMessages(m) => m.messages,
            tl::enums::messages::Messages::NotModified(_) => vec![],
        };
        Ok(msgs
            .into_iter()
            .map(update::IncomingMessage::from_raw)
            .collect())
    }
}

/// Fluent builder for `messages.searchGlobal` (cross-chat search).
///
/// Created by [`Client::search_global_builder`]. All setters are chainable;
/// call [`fetch`] to execute.
///
/// [`fetch`]: GlobalSearchBuilder::fetch
pub struct GlobalSearchBuilder {
    query: String,
    filter: tl::enums::MessagesFilter,
    min_date: i32,
    max_date: i32,
    offset_rate: i32,
    offset_id: i32,
    limit: i32,
    folder_id: Option<i32>,
    broadcasts_only: bool,
    groups_only: bool,
    users_only: bool,
}

impl GlobalSearchBuilder {
    pub(crate) fn new(query: String) -> Self {
        Self {
            query,
            filter: tl::enums::MessagesFilter::InputMessagesFilterEmpty,
            min_date: 0,
            max_date: 0,
            offset_rate: 0,
            offset_id: 0,
            limit: 100,
            folder_id: None,
            broadcasts_only: false,
            groups_only: false,
            users_only: false,
        }
    }

    /// Restrict to a specific dialog folder.
    pub fn folder_id(mut self, id: i32) -> Self {
        self.folder_id = Some(id);
        self
    }

    /// Only return results from broadcast channels.
    pub fn broadcasts_only(mut self, v: bool) -> Self {
        self.broadcasts_only = v;
        self
    }

    /// Only return results from groups/supergroups.
    pub fn groups_only(mut self, v: bool) -> Self {
        self.groups_only = v;
        self
    }

    /// Only return results from private chats / bots.
    pub fn users_only(mut self, v: bool) -> Self {
        self.users_only = v;
        self
    }

    /// Apply a `MessagesFilter` (e.g. photos, video, etc.).
    pub fn filter(mut self, f: tl::enums::MessagesFilter) -> Self {
        self.filter = f;
        self
    }

    /// Only return messages on or after this Unix timestamp.
    pub fn min_date(mut self, ts: i32) -> Self {
        self.min_date = ts;
        self
    }

    /// Only return messages on or before this Unix timestamp.
    pub fn max_date(mut self, ts: i32) -> Self {
        self.max_date = ts;
        self
    }

    /// Pagination: rate from the previous response's last message.
    pub fn offset_rate(mut self, r: i32) -> Self {
        self.offset_rate = r;
        self
    }

    /// Pagination: start from this message ID.
    pub fn offset_id(mut self, id: i32) -> Self {
        self.offset_id = id;
        self
    }

    /// Maximum number of messages to return (default 100).
    pub fn limit(mut self, n: i32) -> Self {
        self.limit = n;
        self
    }

    /// Execute the global search and return matching messages.
    pub async fn fetch(
        self,
        client: &Client,
    ) -> Result<Vec<update::IncomingMessage>, InvocationError> {
        let req = tl::functions::messages::SearchGlobal {
            broadcasts_only: self.broadcasts_only,
            groups_only: self.groups_only,
            users_only: self.users_only,
            folder_id: self.folder_id,
            q: self.query,
            filter: self.filter,
            min_date: self.min_date,
            max_date: self.max_date,
            offset_rate: self.offset_rate,
            offset_peer: tl::enums::InputPeer::Empty,
            offset_id: self.offset_id,
            limit: self.limit,
        };
        let body = client.rpc_call_raw(&req).await?;
        let mut cur = Cursor::from_slice(&body);
        let msgs = match tl::enums::messages::Messages::deserialize(&mut cur)? {
            tl::enums::messages::Messages::Messages(m) => m.messages,
            tl::enums::messages::Messages::Slice(m) => m.messages,
            tl::enums::messages::Messages::ChannelMessages(m) => m.messages,
            tl::enums::messages::Messages::NotModified(_) => vec![],
        };
        Ok(msgs
            .into_iter()
            .map(update::IncomingMessage::from_raw)
            .collect())
    }
}
