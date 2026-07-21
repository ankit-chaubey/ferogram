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

use std::collections::{HashMap, HashSet, VecDeque};
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

    /// Whether this dialog belongs to the given chat folder.
    ///
    /// `exclude_peers` always loses, `pinned_peers`/`include_peers` always
    /// win, and everything else falls back to the contacts/non_contacts/
    /// groups/broadcasts/bots flags plus exclude_muted/read/archived. The
    /// "All Chats" default filter always matches; folders (no `Peer`) never do.
    ///
    /// Takes a pre-flattened filter rather than the raw `tl::enums::DialogFilter`
    /// - build one once per filter with [`FlattenedDialogFilter::from`] and
    ///   reuse it across every dialog in the scroll, rather than re-scanning
    ///   the filter's peer lists on every single call.
    pub fn matches_filter(&self, filter: &FlattenedDialogFilter) -> bool {
        if filter.is_default {
            return true;
        }
        let Some(peer) = self.peer() else {
            return false;
        };
        let id = peer_id(peer);

        if filter.exclude_peers.contains(&id) {
            return false;
        }
        if filter.pinned_peers.contains_key(&id) || filter.include_peers.contains(&id) {
            return true;
        }

        if filter.exclude_archived
            && let tl::enums::Dialog::Dialog(d) = &self.raw
            && d.folder_id == Some(1)
        {
            return false;
        }
        if filter.exclude_read && self.unread_count() == 0 {
            return false;
        }
        if filter.exclude_muted
            && let tl::enums::Dialog::Dialog(d) = &self.raw
        {
            let tl::enums::PeerNotifySettings::PeerNotifySettings(n) = &d.notify_settings;
            if let Some(until) = n.mute_until
                && until > chrono::Utc::now().timestamp() as i32
            {
                return false;
            }
        }

        let is_bot = matches!(&self.entity, Some(tl::enums::User::User(u)) if u.bot);
        let is_contact = matches!(&self.entity, Some(tl::enums::User::User(u)) if u.contact);
        let is_broadcast = matches!(&self.chat, Some(tl::enums::Chat::Channel(c)) if c.broadcast);
        let is_group = self.chat.is_some() && !is_broadcast;

        (filter.include_bots && is_bot)
            || (filter.include_contacts && self.entity.is_some() && is_contact && !is_bot)
            || (filter.include_non_contacts && self.entity.is_some() && !is_contact && !is_bot)
            || (filter.include_groups && is_group)
            || (filter.include_broadcasts && is_broadcast)
    }
}

fn peer_id(peer: &tl::enums::Peer) -> i64 {
    match peer {
        tl::enums::Peer::User(p) => p.user_id,
        tl::enums::Peer::Chat(p) => p.chat_id,
        tl::enums::Peer::Channel(p) => p.channel_id,
    }
}

fn input_peer_id(peer: &tl::enums::InputPeer) -> Option<i64> {
    match peer {
        tl::enums::InputPeer::User(p) => Some(p.user_id),
        tl::enums::InputPeer::Chat(p) => Some(p.chat_id),
        tl::enums::InputPeer::Channel(p) => Some(p.channel_id),
        _ => None,
    }
}

fn peer_id_set(list: &[tl::enums::InputPeer]) -> HashSet<i64> {
    list.iter().filter_map(input_peer_id).collect()
}

/// Peer id -> position in the list, so callers can sort a folder's pinned
/// chats to match Telegram's own per-folder pin order instead of whatever
/// order the server happens to stream dialogs in.
fn peer_id_index(list: &[tl::enums::InputPeer]) -> HashMap<i64, usize> {
    list.iter()
        .filter_map(input_peer_id)
        .enumerate()
        .map(|(i, id)| (id, i))
        .collect()
}

/// Flattened, cheap-to-query form of a `tl::enums::DialogFilter`.
///
/// Checking `pinned_peers`/`include_peers`/`exclude_peers` membership
/// against the raw `Vec<InputPeer>` on every dialog in a scroll is an O(n·m)
/// linear scan; building this once per filter and reusing it makes each
/// check O(1) instead. Get one with [`FlattenedDialogFilter::from`].
#[derive(Debug, Clone, Default)]
pub struct FlattenedDialogFilter {
    /// Whether this is the "All Chats" pseudo-filter - always matches, and
    /// every other field here is meaningless when this is set.
    pub is_default: bool,
    pub include_contacts: bool,
    pub include_non_contacts: bool,
    pub include_groups: bool,
    pub include_broadcasts: bool,
    pub include_bots: bool,
    pub exclude_muted: bool,
    pub exclude_read: bool,
    pub exclude_archived: bool,
    pub include_peers: HashSet<i64>,
    pub exclude_peers: HashSet<i64>,
    /// Peer id -> pin position within this folder.
    pub pinned_peers: HashMap<i64, usize>,
}

impl From<&tl::enums::DialogFilter> for FlattenedDialogFilter {
    fn from(filter: &tl::enums::DialogFilter) -> Self {
        match filter {
            tl::enums::DialogFilter::Default => Self {
                is_default: true,
                ..Self::default()
            },
            // Chatlist (shared folder link) only ever includes chats via its
            // explicit lists - no category flags, no exclude_peers, no
            // exclude_muted/read/archived toggles. Leaving those at their
            // `Default::default()` (false / empty) reproduces exactly that:
            // nothing gets excluded, nothing gets auto-included by category.
            tl::enums::DialogFilter::Chatlist(f) => Self {
                include_peers: peer_id_set(&f.include_peers),
                pinned_peers: peer_id_index(&f.pinned_peers),
                ..Self::default()
            },
            tl::enums::DialogFilter::DialogFilter(f) => Self {
                is_default: false,
                include_contacts: f.contacts,
                include_non_contacts: f.non_contacts,
                include_groups: f.groups,
                include_broadcasts: f.broadcasts,
                include_bots: f.bots,
                exclude_muted: f.exclude_muted,
                exclude_read: f.exclude_read,
                exclude_archived: f.exclude_archived,
                include_peers: peer_id_set(&f.include_peers),
                exclude_peers: peer_id_set(&f.exclude_peers),
                pinned_peers: peer_id_index(&f.pinned_peers),
            },
        }
    }
}

/// A filter's own id (`0` for the "All Chats" pseudo-filter, which Telegram
/// doesn't actually include in `getDialogFilters`' response).
pub(crate) fn dialog_filter_id(filter: &tl::enums::DialogFilter) -> i32 {
    match filter {
        tl::enums::DialogFilter::Default => 0,
        tl::enums::DialogFilter::DialogFilter(f) => f.id,
        tl::enums::DialogFilter::Chatlist(f) => f.id,
    }
}

/// Whether a filter can include Archive dialogs at all, so callers can skip
/// paging Archive entirely instead of fetching it just to discard it.
///
/// "All Chats" (`Default`) never does - that's the same folder-0/folder-1
/// split as the regular chat list, Archive only shows up if you open it
/// explicitly. Custom filters can pull in archived chats unless they were
/// built with "Exclude archived chats" on.
pub(crate) fn scan_archive_for(filter: &FlattenedDialogFilter) -> bool {
    !filter.is_default && !filter.exclude_archived
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
    /// Build a cursor directly - e.g. resuming from a position you tracked
    /// yourself, rather than one saved earlier via [`DialogIter::cursor`].
    pub fn new(
        offset_date: i32,
        offset_id: i32,
        offset_peer: tl::enums::InputPeer,
        exclude_pinned: bool,
        folder_id: Option<i32>,
    ) -> Self {
        Self {
            offset_date,
            offset_id,
            offset_peer: offset_peer.to_bytes(),
            exclude_pinned,
            folder_id,
            total: None,
        }
    }

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

    /// Wrap an arbitrary dialog stream - used for combinations `DialogIter`
    /// alone can't express, e.g. chaining folders together for
    /// [`Client::stream_dialogs_in_filter`](crate::Client::stream_dialogs_in_filter).
    pub(crate) fn boxed(
        inner: impl Stream<Item = Result<Dialog, InvocationError>> + Send + 'static,
    ) -> Self {
        Self {
            inner: Box::pin(inner),
        }
    }
}

/// Cursor-based iterator over dialogs in one chat folder (`DialogFilter`).
/// Created by [`Client::iter_dialogs_in_filter`].
///
/// A folder's chats can live in either the main list or Archive, so this
/// pages through the main list first, then Archive (skipped entirely for
/// filters that can't include it - see [`scan_archive_for`]), yielding only
/// dialogs [`Dialog::matches_filter`] accepts.
pub struct DialogFilterIter {
    pub(crate) filter_id: i32,
    pub(crate) filter: FlattenedDialogFilter,
    pub(crate) main: DialogIter,
    pub(crate) archived: DialogIter,
    pub(crate) scan_archive: bool,
    pub(crate) in_archive: bool,
}

impl DialogFilterIter {
    /// Fetch the next matching dialog. Returns `None` once both folders
    /// (or just the main one, if this filter can't include Archive) are exhausted.
    pub async fn next(&mut self, client: &Client) -> Result<Option<Dialog>, InvocationError> {
        loop {
            let next = if !self.in_archive {
                match self.main.next(client).await? {
                    Some(d) => Some(d),
                    None => {
                        if !self.scan_archive {
                            return Ok(None);
                        }
                        self.in_archive = true;
                        continue;
                    }
                }
            } else {
                self.archived.next(client).await?
            };

            match next {
                Some(d) if d.matches_filter(&self.filter) => return Ok(Some(d)),
                Some(_) => continue,
                None => return Ok(None),
            }
        }
    }

    /// Snapshot the current position as a serializable [`DialogFilterCursor`].
    /// Persist it and hand it to
    /// [`Client::iter_dialogs_in_filter_from`](crate::Client::iter_dialogs_in_filter_from)
    /// later to resume this exact scroll.
    pub fn cursor(&self) -> DialogFilterCursor {
        DialogFilterCursor {
            filter_id: self.filter_id,
            main: self.main.cursor(),
            archived: self.archived.cursor(),
            in_archive: self.in_archive,
        }
    }
}

/// Serializable snapshot of a [`DialogFilterIter`]'s position. Get one via
/// [`DialogFilterIter::cursor`], resume with
/// [`Client::iter_dialogs_in_filter_from`](crate::Client::iter_dialogs_in_filter_from).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DialogFilterCursor {
    pub(crate) filter_id: i32,
    pub(crate) main: DialogCursor,
    pub(crate) archived: DialogCursor,
    pub(crate) in_archive: bool,
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
