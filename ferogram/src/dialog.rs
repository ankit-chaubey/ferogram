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
use std::pin::Pin;
use std::task::{Context, Poll};

use ferogram_tl_types as tl;
use futures::stream::{Stream, try_unfold};
use serde::{Deserialize, Serialize};
use tl::Serializable;

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
                tl::enums::Chat::Community(c) => c.title.clone(),
                tl::enums::Chat::CommunityForbidden(c) => c.title.clone(),
            };
        }
        "(Unknown)".to_string()
    }

    /// Peer of this dialog.
    ///
    /// `None` for folders and for community dialogs - communities are
    /// addressed by `community_id` (see `dialogCommunity`), not by a
    /// `Peer`, since layer 228 didn't add a corresponding `Peer::Community`
    /// variant.
    pub fn peer(&self) -> Option<&tl::enums::Peer> {
        match &self.raw {
            tl::enums::Dialog::Dialog(d) => Some(&d.peer),
            tl::enums::Dialog::Folder(_) => None,
            tl::enums::Dialog::Community(_) => None,
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

/// Options for [`crate::Client::get_dialogs`]. Also used by [`DialogIter`]
/// for `exclude_pinned`/`folder_id` (via its own builder methods - `limit`
/// isn't meaningful there, `DialogIter` always pages at its own fixed size).
///
/// `limit` has no sensible default, so build this via `10.into()` (see the
/// `From<i32>` impl below) rather than `GetDialogsOptions::default()`
/// directly unless you're also setting `limit` explicitly.
///
/// `offset_date`/`offset_id`/`offset_peer`/`hash` aren't fields here -
/// the offsets are `DialogIter`'s job across pages, and `hash` intentionally
/// stays out of caller control since a stale-but-matching value silently
/// returns an empty result via `messages.Dialogs::NotModified`.
#[derive(Default, Clone, Copy)]
pub struct GetDialogsOptions {
    /// Fetch up to this many dialogs, most recent first.
    pub limit: i32,
    /// Skip pinned dialogs entirely.
    pub exclude_pinned: bool,
    /// Only return dialogs in this folder (0 = default folder, 1 = Archive).
    pub folder_id: Option<i32>,
}

/// `get_dialogs(10)` still works unchanged - a bare `i32` is sugar for
/// `GetDialogsOptions { limit, ..Default::default() }`.
impl From<i32> for GetDialogsOptions {
    fn from(limit: i32) -> Self {
        Self {
            limit,
            ..Default::default()
        }
    }
}

/// Serializable snapshot of a [`DialogIter`]'s position, for resuming
/// pagination across app restarts (e.g. a chat list the user scrolled
/// partway through, backgrounded, and reopened later).
///
/// Captures the offset triple Telegram expects plus the filters the
/// iterator was created with, so resuming can't silently drop
/// `exclude_pinned`/`folder_id`. Also carries `total` (the dialog count
/// known at save time) so a UI can render e.g. "250 of 1,200" immediately
/// on resume, without waiting on a page fetch to refill it. Does not
/// capture `done` - that's re-derived from the next page fetch either way.
///
/// Get one from a live iterator with [`DialogIter::cursor`], persist it
/// however you like (it's `Serialize`/`Deserialize`), and resume with
/// [`Client::iter_dialogs_from`](crate::Client::iter_dialogs_from).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DialogCursor {
    pub(crate) offset_date: i32,
    pub(crate) offset_id: i32,
    // `InputPeer` has no serde impl of its own; this reuses the same raw
    // TL binary encoding ferogram already sends on the wire, so it stays
    // correct across TL layer bumps without needing a second codec here.
    pub(crate) offset_peer: Vec<u8>,
    pub(crate) exclude_pinned: bool,
    pub(crate) folder_id: Option<i32>,
    pub(crate) total: Option<i32>,
}

impl Default for DialogCursor {
    /// The starting cursor: first page, default filters (everything, folder 0).
    fn default() -> Self {
        Self {
            offset_date: 0,
            offset_id: 0,
            offset_peer: tl::enums::InputPeer::Empty.to_bytes(),
            exclude_pinned: false,
            folder_id: None,
            total: None,
        }
    }
}

impl DialogCursor {
    /// The `exclude_pinned` filter this cursor was saved with.
    ///
    /// Unlike the position fields, this is intentionally public - filters
    /// describe *what* a saved scroll was over, not opaque server-side
    /// position, so it's fine (and often useful, e.g. for a "resuming your
    /// Archive scroll" label) to inspect before calling
    /// [`Client::iter_dialogs_from`](crate::Client::iter_dialogs_from).
    pub fn exclude_pinned(&self) -> bool {
        self.exclude_pinned
    }

    /// The `folder_id` filter this cursor was saved with.
    pub fn folder_id(&self) -> Option<i32> {
        self.folder_id
    }

    /// The dialog count known at save time, if any - mirrors
    /// [`DialogIter::total`]. `None` if the cursor was saved before the
    /// first page fetch (server hadn't reported a count yet), or for
    /// accounts with fewer dialogs than a single page.
    pub fn total(&self) -> Option<i32> {
        self.total
    }
}

/// Cursor-based iterator over dialogs. Created by [`Client::iter_dialogs`].
pub struct DialogIter {
    pub(crate) offset_date: i32,
    pub(crate) offset_id: i32,
    pub(crate) offset_peer: tl::enums::InputPeer,
    pub(crate) exclude_pinned: bool,
    pub(crate) folder_id: Option<i32>,
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

    /// Skip pinned dialogs entirely.
    pub fn exclude_pinned(mut self, v: bool) -> Self {
        self.exclude_pinned = v;
        self
    }

    /// Only iterate dialogs in this folder (0 = default folder, 1 = Archive).
    pub fn folder_id(mut self, id: Option<i32>) -> Self {
        self.folder_id = id;
        self
    }

    /// Snapshot the current position as a serializable [`DialogCursor`].
    ///
    /// Safe to call at any point, including before the first [`next`](Self::next)
    /// call (yields the starting cursor) and after iteration finishes (yields
    /// a cursor that, if resumed, will fetch an empty final page - cheap, not
    /// an error). Persist the result and hand it to
    /// [`Client::iter_dialogs_from`](crate::Client::iter_dialogs_from) later
    /// to pick up exactly where this iterator left off.
    pub fn cursor(&self) -> DialogCursor {
        DialogCursor {
            offset_date: self.offset_date,
            offset_id: self.offset_id,
            offset_peer: self.offset_peer.to_bytes(),
            exclude_pinned: self.exclude_pinned,
            folder_id: self.folder_id,
            total: self.total,
        }
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
            exclude_pinned: self.exclude_pinned,
            folder_id: self.folder_id,
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

/// A boxed, nameable [`futures::Stream`] over dialogs, giving access to
/// `StreamExt`/`TryStreamExt` combinators (`.map()`, `.take()`,
/// `.try_for_each()`, etc.). Created by [`Client::stream_dialogs`].
///
/// This wraps a [`DialogIter`] internally via `futures::stream::try_unfold` -
/// `DialogIter` and its `next` method are untouched, so existing manual-loop
/// callers see no change. The `Box` is what makes the type nameable at all
/// (the raw `try_unfold` return type can't be written down) and sidesteps
/// hand-written pin projection, at the cost of one heap allocation for the
/// whole stream (not one per item).
pub struct DialogsStream {
    inner: Pin<Box<dyn Stream<Item = Result<Dialog, InvocationError>> + Send>>,
}

impl DialogsStream {
    pub(crate) fn new(client: Client, iter: DialogIter) -> Self {
        let raw = try_unfold((client, iter), |(client, mut iter)| async move {
            match iter.next(&client).await? {
                Some(dialog) => Ok(Some((dialog, (client, iter)))),
                None => Ok(None),
            }
        });
        Self {
            inner: Box::pin(raw),
        }
    }
}

impl Stream for DialogsStream {
    type Item = Result<Dialog, InvocationError>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.inner.as_mut().poll_next(cx)
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
