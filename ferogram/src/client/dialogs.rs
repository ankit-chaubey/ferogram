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

use crate::*;
#[allow(unused_imports)]
use crate::{
    InputMessage, InvocationError, PeerRef,
    dialog::{Dialog, DialogIter, MessageIter},
    inline_iter, media, participants, search, update,
};
use ferogram_tl_types::{Cursor, Deserializable};
use std::collections::{HashMap, VecDeque};

impl Client {
    /// Fetch up to `limit` dialogs, most recent first. Populates entity/message.
    pub async fn get_dialogs(&self, limit: i32) -> Result<Vec<Dialog>, InvocationError> {
        let req = tl::functions::messages::GetDialogs {
            exclude_pinned: false,
            folder_id: None,
            offset_date: 0,
            offset_id: 0,
            offset_peer: tl::enums::InputPeer::Empty,
            limit,
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
                let chat = peer.and_then(|p| match p {
                    tl::enums::Peer::Chat(c) => chat_map.get(&c.chat_id).cloned(),
                    tl::enums::Peer::Channel(c) => chat_map.get(&c.channel_id).cloned(),
                    _ => None,
                });

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

    // Internal helper: fetch dialogs with a custom GetDialogs request.
    // Like `get_messages` but also returns the total count from `messages.Slice`.

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
                self.rpc_call_raw(&req).await?;
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
    pub fn parse_invite_hash(link: &str) -> Option<&str> {
        if let Some(pos) = link.find("/+") {
            return Some(&link[pos + 2..]);
        }
        if let Some(pos) = link.find("/joinchat/") {
            return Some(&link[pos + 10..]);
        }
        None
    }

    ///
    /// Returns a [`DialogIter`] that can be advanced with [`DialogIter::next`].
    /// Lets you page through all dialogs without loading them all at once.
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
            done: false,
            buffer: VecDeque::new(),
            total: None,
        }
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
                let chat = peer.and_then(|p| match p {
                    tl::enums::Peer::Chat(c) => chat_map.get(&c.chat_id).cloned(),
                    tl::enums::Peer::Channel(c) => chat_map.get(&c.channel_id).cloned(),
                    _ => None,
                });
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
