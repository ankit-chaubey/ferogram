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
use super::{chat_to_peer, updates_entities};
use crate::*;
#[allow(unused_imports)]
use crate::{
    InputMessage, InvocationError, PeerRef,
    dialog::{Dialog, DialogIter, MessageIter},
    inline_iter, media, participants, search, update,
};
#[allow(unused_imports)]
use ferogram_tl_types::{Cursor, Deserializable};

impl Client {
    /// Resolve any peer reference to a [`tl::enums::Peer`].
    ///
    /// Accepts everything [`PeerRef`] accepts:
    ///
    /// - `&str` / `String`: `"@username"`, `"me"`, `"self"`, numeric string,
    ///   `t.me/` URL, invite link, E.164 phone
    /// - `i64` / `i32`: Bot-API encoded numeric ID
    /// - [`tl::enums::Peer`]: returned as-is (zero cost)
    /// - [`tl::enums::InputPeer`]: hash cached, then stripped to `Peer`
    ///
    /// Resolution is cache-first; an RPC is only made on a genuine cache miss.
    pub async fn resolve<P: Into<PeerRef>>(
        &self,
        peer: P,
    ) -> Result<tl::enums::Peer, InvocationError> {
        peer.into().resolve(self).await
    }

    /// `contacts.resolveUsername` RPC; called only on cache miss.
    pub(crate) async fn resolve_username_rpc(
        &self,
        username: &str,
    ) -> Result<tl::enums::Peer, InvocationError> {
        let req = tl::functions::contacts::ResolveUsername {
            username: username.to_string(),
            referer: None,
        };
        let body: Vec<u8> = self.rpc_call_raw(&req).await?;
        let mut cur = Cursor::from_slice(&body);
        let tl::enums::contacts::ResolvedPeer::ResolvedPeer(resolved) =
            tl::enums::contacts::ResolvedPeer::deserialize(&mut cur)?;
        self.cache_users_slice(&resolved.users).await;
        self.cache_chats_slice(&resolved.chats).await;
        Ok(resolved.peer)
    }

    /// RPC fallback for `PeerRef::Id` when the peer is not in the cache.
    ///
    /// - `Peer::User`    → `users.getUsers` with `access_hash = 0` (works for
    ///   contacts and recently-interacted users; may return `UserEmpty` for
    ///   strangers; falls back to `messages.getPeerDialogs` in that case).
    /// - `Peer::Channel` → `channels.getChannels` with `access_hash = 0`.
    ///   This works only for public channels; returns `ChannelEmpty` otherwise.
    /// - `Peer::Chat`    → basic groups never need a hash; return immediately.
    ///
    /// On success the resolved entity is inserted into the peer cache so
    /// subsequent lookups are free.
    pub(crate) async fn fetch_by_id_rpc(
        &self,
        peer: tl::enums::Peer,
    ) -> Result<tl::enums::Peer, InvocationError> {
        match &peer {
            tl::enums::Peer::Chat(_) => {
                // Basic groups need no access_hash; always resolvable from ID.
                Ok(peer)
            }

            tl::enums::Peer::User(u) => {
                let req = tl::functions::users::GetUsers {
                    id: vec![tl::enums::InputUser::InputUser(tl::types::InputUser {
                        user_id: u.user_id,
                        access_hash: 0,
                    })],
                };
                let body: Vec<u8> = self.rpc_call_raw(&req).await?;
                let mut cur = Cursor::from_slice(&body);
                let users = Vec::<tl::enums::User>::deserialize(&mut cur)?;
                // Filter out UserEmpty responses
                let valid: Vec<_> = users
                    .into_iter()
                    .filter(|u| matches!(u, tl::enums::User::User(_)))
                    .collect();
                if !valid.is_empty() {
                    self.cache_users_slice(&valid).await;
                    return Ok(peer);
                }

                // Fallback: messages.getPeerDialogs (finds peers you've interacted with)
                let cache_read: tokio::sync::RwLockReadGuard<'_, crate::PeerCache> =
                    self.inner.peer_cache.read().await;
                if cache_read.users.contains_key(&match &peer {
                    tl::enums::Peer::User(u) => u.user_id,
                    _ => unreachable!(),
                }) {
                    drop(cache_read);
                    return Ok(peer);
                }
                drop(cache_read);

                let uid = match &peer {
                    tl::enums::Peer::User(u) => u.user_id,
                    _ => unreachable!(),
                };
                let req2 = tl::functions::messages::GetPeerDialogs {
                    peers: vec![tl::enums::InputDialogPeer::InputDialogPeer(
                        tl::types::InputDialogPeer {
                            peer: tl::enums::InputPeer::User(tl::types::InputPeerUser {
                                user_id: uid,
                                access_hash: 0,
                            }),
                        },
                    )],
                };
                let body2 = self.rpc_call_raw(&req2).await;
                match body2 {
                    Ok(b) => {
                        let mut cur2 = Cursor::from_slice(&b);
                        if let Ok(tl::enums::messages::PeerDialogs::PeerDialogs(pd)) =
                            tl::enums::messages::PeerDialogs::deserialize(&mut cur2)
                        {
                            self.cache_users_and_chats(&pd.users, &pd.chats).await;
                        }
                        Ok(peer)
                    }
                    Err(e) => Err(e),
                }
            }

            tl::enums::Peer::Channel(c) => {
                let req = tl::functions::channels::GetChannels {
                    id: vec![tl::enums::InputChannel::InputChannel(
                        tl::types::InputChannel {
                            channel_id: c.channel_id,
                            access_hash: 0,
                        },
                    )],
                };
                let body: Vec<u8> = self.rpc_call_raw(&req).await?;
                let mut cur = Cursor::from_slice(&body);
                let chats = tl::enums::messages::Chats::deserialize(&mut cur)?;
                let chats_vec = match chats {
                    tl::enums::messages::Chats::Chats(c) => c.chats,
                    tl::enums::messages::Chats::Slice(c) => c.chats,
                };
                let non_empty: Vec<_> = chats_vec
                    .into_iter()
                    .filter(|ch| !matches!(ch, tl::enums::Chat::Empty(_)))
                    .collect();
                if !non_empty.is_empty() {
                    self.cache_chats_slice(&non_empty).await;
                    return Ok(peer);
                }

                // Fallback: getPeerDialogs
                let cid = c.channel_id;
                let req2 = tl::functions::messages::GetPeerDialogs {
                    peers: vec![tl::enums::InputDialogPeer::InputDialogPeer(
                        tl::types::InputDialogPeer {
                            peer: tl::enums::InputPeer::Channel(tl::types::InputPeerChannel {
                                channel_id: cid,
                                access_hash: 0,
                            }),
                        },
                    )],
                };
                let body2 = self.rpc_call_raw(&req2).await;
                match body2 {
                    Ok(b) => {
                        let mut cur2 = Cursor::from_slice(&b);
                        if let Ok(tl::enums::messages::PeerDialogs::PeerDialogs(pd)) =
                            tl::enums::messages::PeerDialogs::deserialize(&mut cur2)
                        {
                            self.cache_users_and_chats(&pd.users, &pd.chats).await;
                        }
                        Ok(peer)
                    }
                    Err(e) => Err(e),
                }
            }
        }
    }

    /// `contacts.importContacts` RPC for phone-based resolution.
    ///
    /// Imports the phone as a temporary contact, caches the returned user, and
    /// returns the resolved Peer.
    pub(crate) async fn resolve_phone_rpc(
        &self,
        phone: &str,
    ) -> Result<tl::enums::Peer, InvocationError> {
        let req = tl::functions::contacts::ImportContacts {
            contacts: vec![tl::enums::InputContact::InputPhoneContact(
                tl::types::InputPhoneContact {
                    client_id: 0,
                    phone: phone.to_string(),
                    first_name: String::new(),
                    last_name: String::new(),
                    note: None,
                },
            )],
        };
        let body: Vec<u8> = self.rpc_call_raw(&req).await?;
        let mut cur = Cursor::from_slice(&body);
        let tl::enums::contacts::ImportedContacts::ImportedContacts(result) =
            tl::enums::contacts::ImportedContacts::deserialize(&mut cur)?;
        self.cache_users_slice(&result.users).await;

        // Check if the phone is now in the cache reverse index
        {
            let cache: tokio::sync::RwLockReadGuard<'_, PeerCache> =
                self.inner.peer_cache.read().await;
            if let Some(&uid) = cache.phone_to_user.get(phone) {
                return Ok(tl::enums::Peer::User(tl::types::PeerUser { user_id: uid }));
            }
        }

        // Fall back: first imported contact's user_id
        result
            .imported
            .first()
            .map(|imp| match imp {
                tl::enums::ImportedContact::ImportedContact(c) => {
                    Ok(tl::enums::Peer::User(tl::types::PeerUser {
                        user_id: c.user_id,
                    }))
                }
            })
            .unwrap_or_else(|| {
                Err(InvocationError::Deserialize(format!(
                    "phone {phone} not found on Telegram"
                )))
            })
    }

    /// `messages.checkChatInvite`; resolves an invite hash to a Peer.
    ///
    /// Succeeds only when you are already a member (`chatInviteAlready` or
    /// `chatInvitePeek`).  Use [`Client::join_by_invite`] to join first.
    pub(crate) async fn resolve_invite_hash_rpc(
        &self,
        hash: &str,
    ) -> Result<tl::enums::Peer, InvocationError> {
        let req = tl::functions::messages::CheckChatInvite {
            hash: hash.to_string(),
        };
        let body: Vec<u8> = self.rpc_call_raw(&req).await?;
        let mut cur = Cursor::from_slice(&body);
        let invite = tl::enums::ChatInvite::deserialize(&mut cur)?;

        match invite {
            tl::enums::ChatInvite::Already(a) => {
                let peer = chat_to_peer(&a.chat);
                self.cache_chats_slice(&[a.chat]).await;
                peer.ok_or_else(|| {
                    InvocationError::Deserialize(
                        "chatInviteAlready: unrecognised chat variant".into(),
                    )
                })
            }
            tl::enums::ChatInvite::Peek(p) => {
                let peer = chat_to_peer(&p.chat);
                self.cache_chats_slice(&[p.chat]).await;
                peer.ok_or_else(|| {
                    InvocationError::Deserialize("chatInvitePeek: unrecognised chat variant".into())
                })
            }
            tl::enums::ChatInvite::ChatInvite(_) => Err(InvocationError::Deserialize(
                "not a member of this chat yet; call client.join_by_invite() first".into(),
            )),
        }
    }

    /// Join a chat by invite link, returning its `InputPeer`.
    ///
    /// Returns `Ok(None)` if Telegram answers with the `WebView` invite
    /// result (e.g. paid channels) - that flow isn't implemented yet.
    pub async fn join_link(
        &self,
        link: &str,
    ) -> Result<Option<tl::enums::InputPeer>, InvocationError> {
        let hash = PeerRef::parse_invite_hash(link)
            .ok_or_else(|| InvocationError::Deserialize(format!("invalid invite link: {link}")))?;
        let req = tl::functions::messages::ImportChatInvite {
            hash: hash.to_string(),
        };
        let body: Vec<u8> = self.rpc_call_raw(&req).await?;
        let mut cur = Cursor::from_slice(&body);
        let result = tl::enums::messages::ChatInviteJoinResult::deserialize(&mut cur)?;

        let updates = match result {
            tl::enums::messages::ChatInviteJoinResult::Ok(ok) => ok.updates,
            tl::enums::messages::ChatInviteJoinResult::WebView(wv) => {
                // No chat was actually joined yet; just cache whatever
                // users Telegram handed us (e.g. the bot) and bail out.
                self.cache_users_slice(&wv.users).await;
                return Ok(None);
            }
        };

        // Extract users and chats embedded in the Updates object
        let (users, chats) = updates_entities(&updates);
        self.cache_users_and_chats(&users, &chats).await;

        // Return the InputPeer of the first chat from the updates
        let cache: tokio::sync::RwLockReadGuard<'_, PeerCache> = self.inner.peer_cache.read().await;
        for chat in &chats {
            match chat {
                tl::enums::Chat::Channel(c) if !c.min => {
                    if let Some(&(hash, _)) = cache.channels.get(&c.id) {
                        return Ok(Some(tl::enums::InputPeer::Channel(
                            tl::types::InputPeerChannel {
                                channel_id: c.id,
                                access_hash: hash,
                            },
                        )));
                    }
                }
                // A community joined by invite link is addressed like a channel
                // on the wire, just tracked in its own cache bucket.
                tl::enums::Chat::Community(c) if !c.min => {
                    if let Some(&hash) = cache.communities.get(&c.id) {
                        return Ok(Some(tl::enums::InputPeer::Channel(
                            tl::types::InputPeerChannel {
                                channel_id: c.id,
                                access_hash: hash,
                            },
                        )));
                    }
                }
                tl::enums::Chat::Chat(c) => {
                    return Ok(Some(tl::enums::InputPeer::Chat(tl::types::InputPeerChat {
                        chat_id: c.id,
                    })));
                }
                _ => {}
            }
        }

        Err(InvocationError::Deserialize(
            "importChatInvite: no chat returned".into(),
        ))
    }

    /// Peek at an invite link without joining.
    ///
    /// Returns the title and participant count of the chat the link points to.
    pub async fn check_invite(&self, link: &str) -> Result<tl::enums::ChatInvite, InvocationError> {
        let hash = PeerRef::parse_invite_hash(link)
            .ok_or_else(|| InvocationError::Deserialize(format!("invalid invite link: {link}")))?;
        let req = tl::functions::messages::CheckChatInvite {
            hash: hash.to_string(),
        };
        let body: Vec<u8> = self.rpc_call_raw(&req).await?;
        let mut cur = Cursor::from_slice(&body);
        Ok(tl::enums::ChatInvite::deserialize(&mut cur)?)
    }
}
