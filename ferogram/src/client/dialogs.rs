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
use super::updates_entities;
use crate::*;
#[allow(unused_imports)]
use crate::{
    InputMessage, InvocationError, PeerRef,
    dialog::{Dialog, DialogIter, MessageIter},
    inline_iter, media, participants, search, update,
};
use ferogram_tl_types::{Cursor, Deserializable};
use futures::stream::try_unfold;
use std::collections::{HashMap, VecDeque};

impl Client {
    /// Fetch dialogs, most recent first. Populates entity/message.
    ///
    /// Accepts a bare `i32` limit (`get_dialogs(10)`) or a
    /// [`GetDialogsOptions`](crate::dialog::GetDialogsOptions) for
    /// `exclude_pinned`/`folder_id` too.
    pub async fn get_dialogs(
        &self,
        opts: impl Into<crate::dialog::GetDialogsOptions>,
    ) -> Result<Vec<Dialog>, InvocationError> {
        let opts = opts.into();
        let req = tl::functions::messages::GetDialogs {
            exclude_pinned: opts.exclude_pinned,
            folder_id: opts.folder_id,
            offset_date: 0,
            offset_id: 0,
            offset_peer: tl::enums::InputPeer::Empty,
            limit: opts.limit,
            hash: 0,
        };

        let body: Vec<u8> = self.rpc_call_raw(&req).await?;
        let raw = match tl::enums::messages::Dialogs::from_bytes_exact(&body)? {
            tl::enums::messages::Dialogs::Dialogs(d) => d,
            tl::enums::messages::Dialogs::Slice(d) => tl::types::messages::Dialogs {
                dialogs: d.dialogs,
                messages: d.messages,
                chats: d.chats,
                users: d.users,
            },
            tl::enums::messages::Dialogs::NotModified(_) => return Ok(vec![]),
        };

        // Build message map
        let msg_map: HashMap<i32, tl::enums::Message> = raw
            .messages
            .into_iter()
            .map(|m| {
                let id = match &m {
                    tl::enums::Message::Message(x) => x.id,
                    tl::enums::Message::Service(x) => x.id,
                    tl::enums::Message::Empty(x) => x.id,
                };
                (id, m)
            })
            .collect();

        // Build user map
        let user_map: HashMap<i64, tl::enums::User> = raw
            .users
            .into_iter()
            .filter_map(|u| {
                if let tl::enums::User::User(ref uu) = u {
                    Some((uu.id, u))
                } else {
                    None
                }
            })
            .collect();

        // Build chat map
        let chat_map: HashMap<i64, tl::enums::Chat> = raw
            .chats
            .into_iter()
            .map(|c| {
                let id = match &c {
                    tl::enums::Chat::Chat(x) => x.id,
                    tl::enums::Chat::Forbidden(x) => x.id,
                    tl::enums::Chat::Channel(x) => x.id,
                    tl::enums::Chat::ChannelForbidden(x) => x.id,
                    tl::enums::Chat::Empty(x) => x.id,
                    tl::enums::Chat::Community(x) => x.id,
                    tl::enums::Chat::CommunityForbidden(x) => x.id,
                };
                (id, c)
            })
            .collect();

        // Cache peers for future access_hash lookups
        {
            let u_list: Vec<tl::enums::User> = user_map.values().cloned().collect();
            let c_list: Vec<tl::enums::Chat> = chat_map.values().cloned().collect();
            self.cache_users_and_chats(&u_list, &c_list).await;
        }

        let result = raw
            .dialogs
            .into_iter()
            .map(|d| {
                let top_id = match &d {
                    tl::enums::Dialog::Dialog(x) => x.top_message,
                    _ => 0,
                };
                let peer = match &d {
                    tl::enums::Dialog::Dialog(x) => Some(&x.peer),
                    _ => None,
                };

                let message = msg_map.get(&top_id).cloned();
                let entity = peer.and_then(|p| match p {
                    tl::enums::Peer::User(u) => user_map.get(&u.user_id).cloned(),
                    _ => None,
                });
                // Communities aren't addressed through a `Peer`, so they need
                // their own lookup by `community_id` instead of falling
                // through the `peer`-based chat/channel match below.
                let chat = match &d {
                    tl::enums::Dialog::Community(c) => chat_map.get(&c.community_id).cloned(),
                    _ => peer.and_then(|p| match p {
                        tl::enums::Peer::Chat(c) => chat_map.get(&c.chat_id).cloned(),
                        tl::enums::Peer::Channel(c) => chat_map.get(&c.channel_id).cloned(),
                        _ => None,
                    }),
                };

                Dialog {
                    raw: d,
                    message,
                    entity,
                    chat,
                }
            })
            .collect();

        Ok(result)
    }

    /// Remove a dialog from your chat list, clearing its history on your
    /// side. For a group or channel this also makes you leave it.
    pub async fn delete_dialog(&self, peer: impl Into<PeerRef>) -> Result<(), InvocationError> {
        let peer = peer.into().resolve(self).await?;
        let input_peer = self.inner.peer_cache.read().await.peer_to_input(&peer)?;
        let req = tl::functions::messages::DeleteHistory {
            just_clear: false,
            revoke: false,
            peer: input_peer,
            max_id: 0,
            min_date: None,
            max_date: None,
        };
        self.rpc_write(&req).await
    }

    /// Mark all messages in a chat as read.
    pub async fn mark_read(&self, peer: impl Into<PeerRef>) -> Result<(), InvocationError> {
        let peer = peer.into().resolve(self).await?;
        let input_peer = self.inner.peer_cache.read().await.peer_to_input(&peer)?;
        match &input_peer {
            tl::enums::InputPeer::Channel(c) => {
                let req = tl::functions::channels::ReadHistory {
                    channel: tl::enums::InputChannel::InputChannel(tl::types::InputChannel {
                        channel_id: c.channel_id,
                        access_hash: c.access_hash,
                    }),
                    max_id: 0,
                };
                self.rpc_call_raw(&req).await?;
            }
            _ => {
                let req = tl::functions::messages::ReadHistory {
                    peer: input_peer,
                    max_id: 0,
                };
                self.rpc_call_raw(&req).await?;
            }
        }
        Ok(())
    }

    /// Clear unread mention markers.
    pub async fn clear_mentions(&self, peer: impl Into<PeerRef>) -> Result<(), InvocationError> {
        let peer = peer.into().resolve(self).await?;
        let input_peer = self.inner.peer_cache.read().await.peer_to_input(&peer)?;
        let req = tl::functions::messages::ReadMentions {
            peer: input_peer,
            top_msg_id: None,
        };
        self.rpc_write(&req).await
    }

    /// Join a public chat or channel by username/peer.
    pub async fn join_chat(&self, peer: impl Into<PeerRef>) -> Result<(), InvocationError> {
        let peer = peer.into().resolve(self).await?;
        let input_peer = self.inner.peer_cache.read().await.peer_to_input(&peer)?;
        match input_peer {
            tl::enums::InputPeer::Channel(c) => {
                let req = tl::functions::channels::JoinChannel {
                    channel: tl::enums::InputChannel::InputChannel(tl::types::InputChannel {
                        channel_id: c.channel_id,
                        access_hash: c.access_hash,
                    }),
                };
                let body: Vec<u8> = self.rpc_call_raw(&req).await?;
                let mut cur = Cursor::from_slice(&body);
                match tl::enums::messages::ChatInviteJoinResult::deserialize(&mut cur)? {
                    tl::enums::messages::ChatInviteJoinResult::Ok(ok) => {
                        let (users, chats) = updates_entities(&ok.updates);
                        self.cache_users_and_chats(&users, &chats).await;
                    }
                    // WebView (e.g. paid channels) needs a bot flow we don't
                    // support yet - cache users and move on, we already have the peer.
                    tl::enums::messages::ChatInviteJoinResult::WebView(wv) => {
                        self.cache_users_slice(&wv.users).await;
                    }
                }
            }
            tl::enums::InputPeer::Chat(c) => {
                let req = tl::functions::messages::AddChatUser {
                    chat_id: c.chat_id,
                    user_id: tl::enums::InputUser::UserSelf,
                    fwd_limit: 0,
                };
                self.rpc_call_raw(&req).await?;
            }
            _ => {
                return Err(InvocationError::Deserialize(
                    "cannot join this peer type".into(),
                ));
            }
        }
        Ok(())
    }

    /// Extract hash from `https://t.me/+HASH` or `https://t.me/joinchat/HASH`.
    ///
    /// Delegates to [`PeerRef::parse_invite_hash`], which also handles
    /// `tg://join?invite=HASH`.
    pub fn parse_invite_hash(link: &str) -> Option<&str> {
        PeerRef::parse_invite_hash(link)
    }

    /// Fetch dialogs, page by page, without loading them all at once.
    ///
    /// Returns a [`DialogIter`] that can be advanced with [`DialogIter::next`].
    ///
    /// # Example
    /// ```rust,no_run
    /// # async fn f(client: ferogram::Client) -> Result<(), Box<dyn std::error::Error>> {
    /// let mut iter = client.iter_dialogs();
    /// while let Some(dialog) = iter.next(&client).await? {
    /// println!("{}", dialog.title());
    /// }
    /// # Ok(()) }
    /// ```
    pub fn iter_dialogs(&self) -> DialogIter {
        DialogIter {
            offset_date: 0,
            offset_id: 0,
            offset_peer: tl::enums::InputPeer::Empty,
            exclude_pinned: false,
            folder_id: None,
            done: false,
            buffer: VecDeque::new(),
            total: None,
        }
    }

    /// Resume paging dialogs from a [`DialogCursor`] saved earlier with
    /// [`DialogIter::cursor`] - e.g. after the app was backgrounded or
    /// restarted mid-scroll.
    ///
    /// # Example
    /// ```rust,no_run
    /// # use ferogram::dialog::DialogCursor;
    /// # async fn f(client: ferogram::Client, saved: DialogCursor) -> Result<(), Box<dyn std::error::Error>> {
    /// let mut iter = client.iter_dialogs_from(saved)?;
    /// while let Some(dialog) = iter.next(&client).await? {
    ///     println!("{}", dialog.title());
    /// }
    /// # Ok(()) }
    /// ```
    pub fn iter_dialogs_from(
        &self,
        cursor: crate::dialog::DialogCursor,
    ) -> Result<DialogIter, InvocationError> {
        let mut buf = Cursor::from_slice(&cursor.offset_peer);
        let offset_peer = tl::enums::InputPeer::deserialize(&mut buf)?;
        Ok(DialogIter {
            offset_date: cursor.offset_date,
            offset_id: cursor.offset_id,
            offset_peer,
            exclude_pinned: cursor.exclude_pinned,
            folder_id: cursor.folder_id,
            done: false,
            buffer: VecDeque::new(),
            total: cursor.total,
        })
    }

    /// Stream dialogs via `futures::Stream` - lets you use `StreamExt`/
    /// `TryStreamExt` combinators (`.map()`, `.take()`, `.try_for_each()`,
    /// etc.) instead of a manual `while let` loop.
    ///
    /// Accepts a bare `i32` or [`GetDialogsOptions`](crate::dialog::GetDialogsOptions)
    /// like [`Client::get_dialogs`] - `limit` is ignored here the same way
    /// it's ignored by [`DialogIter`](crate::dialog::DialogIter), which this
    /// streams from internally; only `exclude_pinned`/`folder_id` apply.
    ///
    /// # Example
    /// ```rust,no_run
    /// # use futures::TryStreamExt;
    /// # async fn f(client: ferogram::Client) -> Result<(), Box<dyn std::error::Error>> {
    /// let mut stream = client.stream_dialogs(0);
    /// while let Some(dialog) = stream.try_next().await? {
    ///     println!("{}", dialog.title());
    /// }
    /// # Ok(()) }
    /// ```
    pub fn stream_dialogs(
        &self,
        opts: impl Into<crate::dialog::GetDialogsOptions>,
    ) -> crate::dialog::DialogsStream {
        let opts = opts.into();
        let iter = self
            .iter_dialogs()
            .exclude_pinned(opts.exclude_pinned)
            .folder_id(opts.folder_id);
        crate::dialog::DialogsStream::new(self.clone(), iter)
    }

    /// Fetch the user's chat folders (Settings > Chat Folders), a.k.a.
    /// `DialogFilter`s. Doesn't include the "All Chats" pseudo-folder
    /// (id `0`) - Telegram doesn't return it here, but every dialog matches
    /// it anyway (see [`Dialog::matches_filter`](crate::dialog::Dialog::matches_filter)).
    pub async fn get_dialog_filters(
        &self,
    ) -> Result<Vec<tl::enums::DialogFilter>, InvocationError> {
        let req = tl::functions::messages::GetDialogFilters {};
        match self.invoke(&req).await? {
            tl::enums::messages::DialogFilters::DialogFilters(f) => Ok(f.filters),
        }
    }

    /// Look up a `DialogFilter` by id. `0` is the "All Chats" pseudo-filter,
    /// synthesized locally since Telegram doesn't return it from `getDialogFilters`.
    async fn resolve_dialog_filter(
        &self,
        filter_id: i32,
    ) -> Result<tl::enums::DialogFilter, InvocationError> {
        if filter_id == 0 {
            return Ok(tl::enums::DialogFilter::Default);
        }
        self.get_dialog_filters()
            .await?
            .into_iter()
            .find(|f| crate::dialog::dialog_filter_id(f) == filter_id)
            .ok_or_else(|| {
                InvocationError::Deserialize(format!("no dialog filter with id {filter_id}"))
            })
    }

    /// Iterate the dialogs in a given chat folder (`filter_id`, `0` for
    /// "All Chats"), page by page.
    ///
    /// `getDialogs` has no filter-id parameter of its own, and a folder's
    /// chats can come from either the main list or Archive - so this pages
    /// through both (skipping Archive entirely for filters that can't
    /// include it, e.g. "All Chats" itself) and keeps only what
    /// [`Dialog::matches_filter`](crate::dialog::Dialog::matches_filter) accepts.
    pub async fn iter_dialogs_in_filter(
        &self,
        filter_id: i32,
    ) -> Result<crate::dialog::DialogFilterIter, InvocationError> {
        let filter = crate::dialog::FlattenedDialogFilter::from(
            &self.resolve_dialog_filter(filter_id).await?,
        );
        let scan_archive = crate::dialog::scan_archive_for(&filter);
        Ok(crate::dialog::DialogFilterIter {
            filter_id,
            filter,
            main: self.iter_dialogs().folder_id(Some(0)),
            archived: self.iter_dialogs().folder_id(Some(1)),
            scan_archive,
            in_archive: false,
        })
    }

    /// Resume [`Client::iter_dialogs_in_filter`] from a
    /// [`DialogFilterCursor`](crate::dialog::DialogFilterCursor) saved earlier
    /// with [`DialogFilterIter::cursor`](crate::dialog::DialogFilterIter::cursor).
    pub async fn iter_dialogs_in_filter_from(
        &self,
        cursor: crate::dialog::DialogFilterCursor,
    ) -> Result<crate::dialog::DialogFilterIter, InvocationError> {
        let filter = crate::dialog::FlattenedDialogFilter::from(
            &self.resolve_dialog_filter(cursor.filter_id).await?,
        );
        let scan_archive = crate::dialog::scan_archive_for(&filter);
        Ok(crate::dialog::DialogFilterIter {
            filter_id: cursor.filter_id,
            filter,
            main: self.iter_dialogs_from(cursor.main)?,
            archived: self.iter_dialogs_from(cursor.archived)?,
            scan_archive,
            in_archive: cursor.in_archive,
        })
    }

    /// Stream only the dialogs in a given chat folder - the non-resumable,
    /// `futures::Stream` counterpart to [`Client::iter_dialogs_in_filter`].
    pub async fn stream_dialogs_in_filter(
        &self,
        filter_id: i32,
    ) -> Result<crate::dialog::DialogsStream, InvocationError> {
        let iter = self.iter_dialogs_in_filter(filter_id).await?;
        let client = self.clone();
        let raw = try_unfold((client, iter), |(client, mut iter)| async move {
            match iter.next(&client).await? {
                Some(d) => Ok(Some((d, (client, iter)))),
                None => Ok(None),
            }
        });
        Ok(crate::dialog::DialogsStream::boxed(raw))
    }

    /// Fetch messages from a peer, page by page.
    ///
    /// Returns a [`MessageIter`] that can be advanced with [`MessageIter::next`].
    ///
    /// # Example
    /// ```rust,no_run
    /// # async fn f(client: ferogram::Client, peer: ferogram_tl_types::enums::Peer) -> Result<(), Box<dyn std::error::Error>> {
    /// let mut iter = client.iter_messages(peer);
    /// while let Some(msg) = iter.next(&client).await? {
    /// println!("{:?}", msg.text());
    /// }
    /// # Ok(()) }
    /// ```
    pub fn iter_messages(&self, peer: impl Into<PeerRef>) -> MessageIter {
        MessageIter {
            unresolved: Some(peer.into()),
            peer: None,
            offset_id: 0,
            done: false,
            buffer: VecDeque::new(),
            total: None,
        }
    }

    /// Fetch all saved drafts across all chats.
    ///
    /// The server responds with an `Updates` containing `updateDraftMessage`
    /// entries; this method triggers that push and returns immediately.
    pub async fn sync_drafts(&self) -> Result<(), InvocationError> {
        let req = tl::functions::messages::GetAllDrafts {};
        self.rpc_write(&req).await
    }

    /// Delete all saved drafts across all chats.
    pub async fn clear_all_drafts(&self) -> Result<(), InvocationError> {
        let req = tl::functions::messages::ClearAllDrafts {};
        self.rpc_write(&req).await
    }

    /// Pin or unpin a dialog. `pin: true` pins, `pin: false` unpins.
    pub async fn pin_dialog(
        &self,
        peer: impl Into<PeerRef>,
        pin: bool,
    ) -> Result<(), InvocationError> {
        let peer = peer.into().resolve(self).await?;
        let input_peer = self.inner.peer_cache.read().await.peer_to_input(&peer)?;
        let req = tl::functions::messages::ToggleDialogPin {
            pinned: pin,
            peer: tl::enums::InputDialogPeer::InputDialogPeer(tl::types::InputDialogPeer {
                peer: input_peer,
            }),
        };
        self.rpc_write(&req).await
    }

    /// Archive or unarchive a dialog. `archive: true` moves to folder 1 (archive), `false` moves back to folder 0.
    pub async fn archive(
        &self,
        peer: impl Into<PeerRef>,
        archive: bool,
    ) -> Result<(), InvocationError> {
        let peer = peer.into().resolve(self).await?;
        let input_peer = self.inner.peer_cache.read().await.peer_to_input(&peer)?;
        let req = tl::functions::folders::EditPeerFolders {
            folder_peers: vec![tl::enums::InputFolderPeer::InputFolderPeer(
                tl::types::InputFolderPeer {
                    peer: input_peer,
                    folder_id: if archive { 1 } else { 0 },
                },
            )],
        };
        self.rpc_write(&req).await
    }

    // Internal helper: fetch dialogs with a custom GetDialogs request.
    // Like `get_messages_with_count` but for dialogs - also returns the
    // total count from `messages.DialogsSlice`.
    pub(crate) async fn get_dialogs_raw_with_count(
        &self,
        req: tl::functions::messages::GetDialogs,
    ) -> Result<(Vec<Dialog>, Option<i32>), InvocationError> {
        let body: Vec<u8> = self.rpc_call_raw(&req).await?;
        let mut cur = Cursor::from_slice(&body);
        let raw = tl::enums::messages::Dialogs::deserialize(&mut cur)?;
        let (dialogs_raw, messages, users, chats, count) = match raw {
            tl::enums::messages::Dialogs::Dialogs(d) => {
                (d.dialogs, d.messages, d.users, d.chats, None)
            }
            tl::enums::messages::Dialogs::Slice(d) => {
                (d.dialogs, d.messages, d.users, d.chats, Some(d.count))
            }
            tl::enums::messages::Dialogs::NotModified(d) => return Ok((vec![], Some(d.count))),
        };

        self.cache_users_and_chats(&users, &chats).await;

        let msg_map: std::collections::HashMap<i32, tl::enums::Message> = messages
            .into_iter()
            .map(|m| {
                let id = match &m {
                    tl::enums::Message::Message(x) => x.id,
                    tl::enums::Message::Service(x) => x.id,
                    tl::enums::Message::Empty(x) => x.id,
                };
                (id, m)
            })
            .collect();

        let user_map: std::collections::HashMap<i64, tl::enums::User> = users
            .into_iter()
            .filter_map(|u| {
                if let tl::enums::User::User(ref uu) = u {
                    Some((uu.id, u))
                } else {
                    None
                }
            })
            .collect();

        let chat_map: std::collections::HashMap<i64, tl::enums::Chat> = chats
            .into_iter()
            .map(|c| {
                let id = match &c {
                    tl::enums::Chat::Chat(x) => x.id,
                    tl::enums::Chat::Forbidden(x) => x.id,
                    tl::enums::Chat::Channel(x) => x.id,
                    tl::enums::Chat::ChannelForbidden(x) => x.id,
                    tl::enums::Chat::Empty(x) => x.id,
                    tl::enums::Chat::Community(x) => x.id,
                    tl::enums::Chat::CommunityForbidden(x) => x.id,
                };
                (id, c)
            })
            .collect();

        let dialogs: Vec<Dialog> = dialogs_raw
            .into_iter()
            .map(|d| {
                let top_id = match &d {
                    tl::enums::Dialog::Dialog(x) => x.top_message,
                    _ => 0,
                };
                let peer = match &d {
                    tl::enums::Dialog::Dialog(x) => Some(&x.peer),
                    _ => None,
                };
                let message = msg_map.get(&top_id).cloned();
                let entity = peer.and_then(|p| match p {
                    tl::enums::Peer::User(u) => user_map.get(&u.user_id).cloned(),
                    _ => None,
                });
                // Communities aren't addressed through a `Peer`, so they need
                // their own lookup by `community_id` instead of falling
                // through the `peer`-based chat/channel match below.
                let chat = match &d {
                    tl::enums::Dialog::Community(c) => chat_map.get(&c.community_id).cloned(),
                    _ => peer.and_then(|p| match p {
                        tl::enums::Peer::Chat(c) => chat_map.get(&c.chat_id).cloned(),
                        tl::enums::Peer::Channel(c) => chat_map.get(&c.channel_id).cloned(),
                        _ => None,
                    }),
                };
                Dialog {
                    raw: d,
                    message,
                    entity,
                    chat,
                }
            })
            .collect();

        Ok((dialogs, count))
    }

    /// List the pinned dialogs in a folder. `folder_id` is `0` for the main
    /// list, `1` for the Archive folder.
    pub async fn get_pinned_dialogs(
        &self,
        folder_id: i32,
    ) -> Result<Vec<tl::enums::Dialog>, InvocationError> {
        let req = tl::functions::messages::GetPinnedDialogs { folder_id };
        let body = self.rpc_call_raw(&req).await?;
        let mut cur = Cursor::from_slice(&body);
        let tl::enums::messages::PeerDialogs::PeerDialogs(result) =
            tl::enums::messages::PeerDialogs::deserialize(&mut cur)?;
        self.cache_users_slice(&result.users).await;
        self.cache_chats_slice(&result.chats).await;
        Ok(result.dialogs)
    }

    /// Mark a dialog as unread, the way "Mark as unread" in the app does -
    /// independent of whether it actually has unread messages.
    pub async fn mark_dialog_unread(
        &self,
        peer: impl Into<PeerRef>,
    ) -> Result<(), InvocationError> {
        self.set_dialog_unread_flag(peer, true).await
    }

    pub(crate) async fn set_dialog_unread_flag(
        &self,
        peer: impl Into<PeerRef>,
        unread: bool,
    ) -> Result<(), InvocationError> {
        let peer = peer.into().resolve(self).await?;
        let input_peer = self.inner.peer_cache.read().await.peer_to_input(&peer)?;
        let req = tl::functions::messages::MarkDialogUnread {
            unread,
            parent_peer: None,
            peer: tl::enums::InputDialogPeer::InputDialogPeer(tl::types::InputDialogPeer {
                peer: input_peer,
            }),
        };
        self.rpc_write(&req).await
    }
}
