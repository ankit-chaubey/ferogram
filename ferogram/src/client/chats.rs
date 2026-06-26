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

impl Client {
    /// Delete a chat or channel. Dispatches to `channels.deleteChannel` for channels/supergroups
    /// and `messages.deleteChat` for legacy basic groups.
    ///
    /// Only the creator can delete. This action is irreversible.
    pub async fn delete_chat(&self, peer: impl Into<PeerRef>) -> Result<(), InvocationError> {
        let peer = peer.into().resolve(self).await?;
        let input_peer = self.inner.peer_cache.read().await.peer_to_input(&peer)?;
        match &input_peer {
            tl::enums::InputPeer::Channel(c) => {
                let req = tl::functions::channels::DeleteChannel {
                    channel: tl::enums::InputChannel::InputChannel(tl::types::InputChannel {
                        channel_id: c.channel_id,
                        access_hash: c.access_hash,
                    }),
                };
                self.rpc_write(&req).await
            }
            tl::enums::InputPeer::Chat(c) => {
                let req = tl::functions::messages::DeleteChat { chat_id: c.chat_id };
                self.rpc_write(&req).await
            }
            _ => Err(InvocationError::Deserialize(
                "delete_chat: peer must be a chat or channel".into(),
            )),
        }
    }

    /// Leave a channel or supergroup.
    ///
    /// For basic groups, kick yourself or use
    /// [`delete_dialog`] to just hide it.
    pub async fn leave_chat(&self, peer: impl Into<PeerRef>) -> Result<(), InvocationError> {
        let peer = peer.into().resolve(self).await?;
        let input_peer = self.inner.peer_cache.read().await.peer_to_input(&peer)?;
        let channel = match &input_peer {
            tl::enums::InputPeer::Channel(c) => {
                tl::enums::InputChannel::InputChannel(tl::types::InputChannel {
                    channel_id: c.channel_id,
                    access_hash: c.access_hash,
                })
            }
            _ => {
                return Err(InvocationError::Deserialize(
                    "leave_chat: peer must be a channel or supergroup".into(),
                ));
            }
        };
        let req = tl::functions::channels::LeaveChannel { channel };
        self.rpc_write(&req).await
    }

    /// Upgrade a legacy group to a supergroup (megagroup).
    ///
    /// Returns the new channel/supergroup peer. The original chat ID becomes
    /// invalid after migration.
    pub async fn migrate_chat(&self, chat_id: i64) -> Result<tl::enums::Chat, InvocationError> {
        let req = tl::functions::messages::MigrateChat { chat_id };
        let body: Vec<u8> = self.rpc_call_raw(&req).await?;
        let mut cur = Cursor::from_slice(&body);
        let updates = tl::enums::Updates::deserialize(&mut cur)?;
        let chats = match updates {
            tl::enums::Updates::Updates(u) => u.chats,
            tl::enums::Updates::Combined(u) => u.chats,
            _ => vec![],
        };
        // The migrated supergroup is the channel in the chats list.
        chats
            .into_iter()
            .find(|c| matches!(c, tl::enums::Chat::Channel(_)))
            .ok_or_else(|| {
                InvocationError::Deserialize("migrate_chat: no channel in response".into())
            })
    }

    #[allow(dead_code)]
    async fn set_default_banned_rights_raw(
        &self,
        peer: impl Into<PeerRef>,
        build: impl FnOnce(
            crate::participants::BannedRightsBuilder,
        ) -> crate::participants::BannedRightsBuilder,
    ) -> Result<(), InvocationError> {
        let peer = peer.into().resolve(self).await?;
        let input_peer = self.inner.peer_cache.read().await.peer_to_input(&peer)?;
        let rights = build(crate::participants::BannedRightsBuilder::new()).into_tl();
        let req = tl::functions::messages::EditChatDefaultBannedRights {
            peer: input_peer,
            banned_rights: rights,
        };
        self.rpc_write(&req).await
    }

    #[allow(dead_code)]
    pub async fn get_chat_full(
        &self,
        peer: impl Into<PeerRef>,
    ) -> Result<tl::enums::messages::ChatFull, InvocationError> {
        let peer = peer.into().resolve(self).await?;
        let input_peer = self.inner.peer_cache.read().await.peer_to_input(&peer)?;
        let body = match &input_peer {
            tl::enums::InputPeer::Channel(c) => {
                let req = tl::functions::channels::GetFullChannel {
                    channel: tl::enums::InputChannel::InputChannel(tl::types::InputChannel {
                        channel_id: c.channel_id,
                        access_hash: c.access_hash,
                    }),
                };
                self.rpc_call_raw(&req).await?
            }
            tl::enums::InputPeer::Chat(c) => {
                let req = tl::functions::messages::GetFullChat { chat_id: c.chat_id };
                self.rpc_call_raw(&req).await?
            }
            _ => {
                return Err(InvocationError::Deserialize(
                    "get_chat_full: peer must be a chat or channel".into(),
                ));
            }
        };
        // Cache users/chats from the response so subsequent calls work.
        let mut cur = Cursor::from_slice(&body);
        let full = tl::enums::messages::ChatFull::deserialize(&mut cur)?;
        let tl::enums::messages::ChatFull::ChatFull(ref f) = full;
        self.cache_users_slice(&f.users).await;
        self.cache_chats_slice(&f.chats).await;
        Ok(full)
    }

    #[allow(dead_code)]
    pub(crate) async fn add_chat_members(
        &self,
        peer: impl Into<PeerRef>,
        user_ids: &[i64],
    ) -> Result<(), InvocationError> {
        if user_ids.is_empty() {
            return Ok(());
        }
        let peer = peer.into().resolve(self).await?;
        let input_peer = self.inner.peer_cache.read().await.peer_to_input(&peer)?;

        match &input_peer {
            tl::enums::InputPeer::Channel(c) => {
                let cache: tokio::sync::RwLockReadGuard<'_, PeerCache> =
                    self.inner.peer_cache.read().await;
                let users: Vec<tl::enums::InputUser> = user_ids
                    .iter()
                    .map(|&id| {
                        let hash = cache.users.get(&id).copied().unwrap_or(0);
                        tl::enums::InputUser::InputUser(tl::types::InputUser {
                            user_id: id,
                            access_hash: hash,
                        })
                    })
                    .collect();
                let req = tl::functions::channels::InviteToChannel {
                    channel: tl::enums::InputChannel::InputChannel(tl::types::InputChannel {
                        channel_id: c.channel_id,
                        access_hash: c.access_hash,
                    }),
                    users,
                };
                self.rpc_write(&req).await
            }
            tl::enums::InputPeer::Chat(c) => {
                // Legacy groups: add one at a time
                for &id in user_ids {
                    let hash = self
                        .inner
                        .peer_cache
                        .read()
                        .await
                        .users
                        .get(&id)
                        .copied()
                        .unwrap_or(0);
                    let req = tl::functions::messages::AddChatUser {
                        chat_id: c.chat_id,
                        user_id: tl::enums::InputUser::InputUser(tl::types::InputUser {
                            user_id: id,
                            access_hash: hash,
                        }),
                        fwd_limit: 0,
                    };
                    self.rpc_write(&req).await?;
                }
                Ok(())
            }
            _ => Err(InvocationError::Deserialize(
                "invite_users: peer must be a chat or channel".into(),
            )),
        }
    }

    pub async fn delete_chat_history(
        &self,
        peer: impl Into<PeerRef>,
        max_id: i32,
        revoke: bool,
    ) -> Result<(), InvocationError> {
        let peer = peer.into().resolve(self).await?;
        let input_peer = self.inner.peer_cache.read().await.peer_to_input(&peer)?;
        match &input_peer {
            tl::enums::InputPeer::Channel(c) => {
                let req = tl::functions::channels::DeleteHistory {
                    for_everyone: revoke,
                    channel: tl::enums::InputChannel::InputChannel(tl::types::InputChannel {
                        channel_id: c.channel_id,
                        access_hash: c.access_hash,
                    }),
                    max_id,
                };
                self.rpc_write(&req).await
            }
            _ => {
                // For regular chats the server may return an offset != 0, indicating
                // that more messages remain and we must call again.
                loop {
                    let req = tl::functions::messages::DeleteHistory {
                        just_clear: false,
                        revoke,
                        peer: input_peer.clone(),
                        max_id,
                        min_date: None,
                        max_date: None,
                    };
                    let body = self.rpc_call_raw(&req).await?;
                    let mut cur = Cursor::from_slice(&body);
                    let tl::enums::messages::AffectedHistory::AffectedHistory(result) =
                        tl::enums::messages::AffectedHistory::deserialize(&mut cur)?;
                    if result.offset == 0 {
                        break;
                    }
                }
                Ok(())
            }
        }
    }

    pub async fn create_group(
        &self,
        title: impl Into<String>,
        user_ids: Vec<i64>,
    ) -> Result<tl::enums::Chat, InvocationError> {
        let cache = self.inner.peer_cache.read().await;
        let users: Vec<tl::enums::InputUser> = user_ids
            .into_iter()
            .map(|id| {
                let hash = cache.users.get(&id).copied().unwrap_or(0);
                tl::enums::InputUser::InputUser(tl::types::InputUser {
                    user_id: id,
                    access_hash: hash,
                })
            })
            .collect();

        let req = tl::functions::messages::CreateChat {
            users,
            title: title.into(),
            ttl_period: None,
        };
        let body = self.rpc_call_raw(&req).await?;
        let mut cur = Cursor::from_slice(&body);
        let updates = tl::enums::Updates::deserialize(&mut cur)?;
        // Extract the chat from updates
        let chats = match updates {
            tl::enums::Updates::Updates(u) => u.chats,
            tl::enums::Updates::Combined(u) => u.chats,
            _ => vec![],
        };
        chats
            .into_iter()
            .next()
            .ok_or_else(|| InvocationError::Deserialize("create_group: no chat in response".into()))
    }

    pub async fn create_channel(
        &self,
        title: impl Into<String>,
        about: impl Into<String>,
        broadcast: bool,
    ) -> Result<tl::enums::Chat, InvocationError> {
        let req = tl::functions::channels::CreateChannel {
            broadcast,
            megagroup: !broadcast,
            for_import: false,
            forum: false,
            title: title.into(),
            about: about.into(),
            geo_point: None,
            address: None,
            ttl_period: None,
        };
        let body = self.rpc_call_raw(&req).await?;
        let mut cur = Cursor::from_slice(&body);
        let updates = tl::enums::Updates::deserialize(&mut cur)?;
        let chats = match updates {
            tl::enums::Updates::Updates(u) => u.chats,
            tl::enums::Updates::Combined(u) => u.chats,
            _ => vec![],
        };
        chats.into_iter().next().ok_or_else(|| {
            InvocationError::Deserialize("create_channel: no chat in response".into())
        })
    }

    pub async fn edit_chat_default_banned_rights(
        &self,
        peer: impl Into<PeerRef>,
        build: impl FnOnce(
            crate::participants::BannedRightsBuilder,
        ) -> crate::participants::BannedRightsBuilder,
    ) -> Result<(), InvocationError> {
        let peer = peer.into().resolve(self).await?;
        let input_peer = self.inner.peer_cache.read().await.peer_to_input(&peer)?;
        let rights = build(crate::participants::BannedRightsBuilder::new()).into_tl();
        let req = tl::functions::messages::EditChatDefaultBannedRights {
            peer: input_peer,
            banned_rights: rights,
        };
        self.rpc_write(&req).await
    }

    pub async fn invite_users(
        &self,
        peer: impl Into<PeerRef>,
        user_ids: &[i64],
    ) -> Result<(), InvocationError> {
        if user_ids.is_empty() {
            return Ok(());
        }
        let peer = peer.into().resolve(self).await?;
        let input_peer = self.inner.peer_cache.read().await.peer_to_input(&peer)?;

        match &input_peer {
            tl::enums::InputPeer::Channel(c) => {
                let cache = self.inner.peer_cache.read().await;
                let users: Vec<tl::enums::InputUser> = user_ids
                    .iter()
                    .map(|&id| {
                        let hash = cache.users.get(&id).copied().unwrap_or(0);
                        tl::enums::InputUser::InputUser(tl::types::InputUser {
                            user_id: id,
                            access_hash: hash,
                        })
                    })
                    .collect();
                let req = tl::functions::channels::InviteToChannel {
                    channel: tl::enums::InputChannel::InputChannel(tl::types::InputChannel {
                        channel_id: c.channel_id,
                        access_hash: c.access_hash,
                    }),
                    users,
                };
                self.rpc_write(&req).await
            }
            tl::enums::InputPeer::Chat(c) => {
                // Legacy groups: add one at a time
                for &id in user_ids {
                    let hash = self
                        .inner
                        .peer_cache
                        .read()
                        .await
                        .users
                        .get(&id)
                        .copied()
                        .unwrap_or(0);
                    let req = tl::functions::messages::AddChatUser {
                        chat_id: c.chat_id,
                        user_id: tl::enums::InputUser::InputUser(tl::types::InputUser {
                            user_id: id,
                            access_hash: hash,
                        }),
                        fwd_limit: 0,
                    };
                    self.rpc_write(&req).await?;
                }
                Ok(())
            }
            _ => Err(InvocationError::Deserialize(
                "invite_users: peer must be a chat or channel".into(),
            )),
        }
    }

    pub async fn set_history_ttl(
        &self,
        peer: impl Into<PeerRef>,
        period: i32,
    ) -> Result<(), InvocationError> {
        let peer = peer.into().resolve(self).await?;
        let input_peer = self.inner.peer_cache.read().await.peer_to_input(&peer)?;
        let req = tl::functions::messages::SetHistoryTtl {
            peer: input_peer,
            period,
        };
        self.rpc_write(&req).await
    }

    pub async fn get_common_chats(
        &self,
        user_id: i64,
        max_id: i64,
        limit: i32,
    ) -> Result<Vec<tl::enums::Chat>, InvocationError> {
        let hash = self
            .inner
            .peer_cache
            .read()
            .await
            .users
            .get(&user_id)
            .copied()
            .unwrap_or(0);
        let req = tl::functions::messages::GetCommonChats {
            user_id: tl::enums::InputUser::InputUser(tl::types::InputUser {
                user_id,
                access_hash: hash,
            }),
            max_id,
            limit,
        };
        let body = self.rpc_call_raw(&req).await?;
        let mut cur = Cursor::from_slice(&body);
        let chats = tl::enums::messages::Chats::deserialize(&mut cur)?;
        Ok(match chats {
            tl::enums::messages::Chats::Chats(c) => c.chats,
            tl::enums::messages::Chats::Slice(c) => c.chats,
        })
    }

    pub async fn get_chat_administrators(
        &self,
        peer: impl Into<PeerRef>,
    ) -> Result<Vec<crate::participants::Participant>, InvocationError> {
        use ferogram_tl_types::{Cursor, Deserializable};
        let peer = peer.into().resolve(self).await?;
        match &peer {
            tl::enums::Peer::Channel(c) => {
                let access_hash = self
                    .inner
                    .peer_cache
                    .read()
                    .await
                    .channels
                    .get(&c.channel_id)
                    .map(|&(hash, _)| hash)
                    .unwrap_or(0);
                let req = tl::functions::channels::GetParticipants {
                    channel: tl::enums::InputChannel::InputChannel(tl::types::InputChannel {
                        channel_id: c.channel_id,
                        access_hash,
                    }),
                    filter: tl::enums::ChannelParticipantsFilter::ChannelParticipantsAdmins,
                    offset: 0,
                    limit: 200,
                    hash: 0,
                };
                let body = self.rpc_call_raw(&req).await?;
                let mut cur = Cursor::from_slice(&body);
                let raw = match tl::enums::channels::ChannelParticipants::deserialize(&mut cur)? {
                    tl::enums::channels::ChannelParticipants::ChannelParticipants(p) => p,
                    tl::enums::channels::ChannelParticipants::NotModified => return Ok(vec![]),
                };
                let user_map: std::collections::HashMap<i64, tl::types::User> = raw
                    .users
                    .into_iter()
                    .filter_map(|u| match u {
                        tl::enums::User::User(u) => Some((u.id, u)),
                        _ => None,
                    })
                    .collect();
                Ok(raw
                    .participants
                    .into_iter()
                    .filter_map(|p| {
                        crate::participants::Participant::from_channel_participant(p, &user_map)
                    })
                    .collect())
            }
            tl::enums::Peer::Chat(_) => {
                // For basic groups return all members; callers check is_admin flag.
                self.get_participants(peer, 0).await
            }
            _ => Err(InvocationError::Deserialize(
                "get_chat_administrators: peer must be a chat or channel".into(),
            )),
        }
    }

    pub async fn transfer_chat_ownership(
        &self,
        peer: impl Into<PeerRef>,
        new_owner_id: i64,
        password: tl::enums::InputCheckPasswordSrp,
    ) -> Result<(), InvocationError> {
        use ferogram_tl_types as tl;
        let peer = peer.into().resolve(self).await?;
        let input_peer = self.inner.peer_cache.read().await.peer_to_input(&peer)?;

        // Resolve the new owner to InputUser
        let owner_peer = tl::enums::Peer::User(tl::types::PeerUser {
            user_id: new_owner_id,
        });
        let owner_input = self
            .inner
            .peer_cache
            .read()
            .await
            .peer_to_input(&owner_peer)?;
        let user_id = match owner_input {
            tl::enums::InputPeer::User(u) => {
                tl::enums::InputUser::InputUser(tl::types::InputUser {
                    user_id: u.user_id,
                    access_hash: u.access_hash,
                })
            }
            _ => {
                return Err(InvocationError::Deserialize(
                    "transfer_chat_ownership: new owner must be a user".into(),
                ));
            }
        };

        let req = tl::functions::messages::EditChatCreator {
            peer: input_peer,
            user_id,
            password,
        };
        self.rpc_call_raw(&req).await?;
        Ok(())
    }

    pub async fn get_linked_channel(
        &self,
        peer: impl Into<PeerRef>,
    ) -> Result<Option<i64>, InvocationError> {
        use ferogram_tl_types as tl;
        let peer = peer.into().resolve(self).await?;
        let input_peer = self.inner.peer_cache.read().await.peer_to_input(&peer)?;
        let channel = match &input_peer {
            tl::enums::InputPeer::Channel(c) => {
                tl::enums::InputChannel::InputChannel(tl::types::InputChannel {
                    channel_id: c.channel_id,
                    access_hash: c.access_hash,
                })
            }
            _ => {
                return Err(InvocationError::Deserialize(
                    "get_linked_channel: peer must be a channel or supergroup".into(),
                ));
            }
        };
        let req = tl::functions::channels::GetFullChannel { channel };
        let body = self.rpc_call_raw(&req).await?;
        let mut cur = Cursor::from_slice(&body);
        let full = tl::enums::messages::ChatFull::deserialize(&mut cur)?;
        let linked = match full {
            tl::enums::messages::ChatFull::ChatFull(f) => match f.full_chat {
                tl::enums::ChatFull::ChannelFull(cf) => cf.linked_chat_id,
                _ => None,
            },
        };
        Ok(linked)
    }

    pub async fn get_admin_log(
        &self,
        peer: impl Into<PeerRef>,
        query: impl Into<String>,
        limit: i32,
        max_id: i64,
        min_id: i64,
    ) -> Result<Vec<tl::types::ChannelAdminLogEvent>, InvocationError> {
        let peer = peer.into().resolve(self).await?;
        let input_peer = self.inner.peer_cache.read().await.peer_to_input(&peer)?;
        let channel = match &input_peer {
            tl::enums::InputPeer::Channel(c) => {
                tl::enums::InputChannel::InputChannel(tl::types::InputChannel {
                    channel_id: c.channel_id,
                    access_hash: c.access_hash,
                })
            }
            _ => {
                return Err(InvocationError::Deserialize(
                    "get_admin_log: peer must be a channel or supergroup".into(),
                ));
            }
        };
        let req = tl::functions::channels::GetAdminLog {
            channel,
            q: query.into(),
            events_filter: None,
            admins: None,
            max_id,
            min_id,
            limit,
        };
        let body = self.rpc_call_raw(&req).await?;
        let mut cur = Cursor::from_slice(&body);
        let tl::enums::channels::AdminLogResults::AdminLogResults(result) =
            tl::enums::channels::AdminLogResults::deserialize(&mut cur)?;
        self.cache_users_slice(&result.users).await;
        self.cache_chats_slice(&result.chats).await;
        Ok(result
            .events
            .into_iter()
            .map(|e| match e {
                tl::enums::ChannelAdminLogEvent::ChannelAdminLogEvent(ev) => ev,
            })
            .collect())
    }

    pub async fn toggle_no_forwards(
        &self,
        peer: impl Into<PeerRef>,
        enabled: bool,
    ) -> Result<(), InvocationError> {
        let peer = peer.into().resolve(self).await?;
        let input_peer = self.inner.peer_cache.read().await.peer_to_input(&peer)?;
        let req = tl::functions::messages::ToggleNoForwards {
            peer: input_peer,
            enabled,
            request_msg_id: None,
        };
        self.rpc_write(&req).await
    }

    pub async fn set_chat_theme(
        &self,
        peer: impl Into<PeerRef>,
        emoticon: impl Into<String>,
    ) -> Result<(), InvocationError> {
        let peer = peer.into().resolve(self).await?;
        let input_peer = self.inner.peer_cache.read().await.peer_to_input(&peer)?;
        let req = tl::functions::messages::SetChatTheme {
            peer: input_peer,
            theme: tl::enums::InputChatTheme::InputChatTheme(tl::types::InputChatTheme {
                emoticon: emoticon.into(),
            }),
        };
        self.rpc_write(&req).await
    }

    pub async fn set_chat_reactions(
        &self,
        peer: impl Into<PeerRef>,
        reactions: tl::enums::ChatReactions,
    ) -> Result<(), InvocationError> {
        let peer = peer.into().resolve(self).await?;
        let input_peer = self.inner.peer_cache.read().await.peer_to_input(&peer)?;
        let req = tl::functions::messages::SetChatAvailableReactions {
            peer: input_peer,
            available_reactions: reactions,
            reactions_limit: None,
            paid_enabled: None,
        };
        self.rpc_write(&req).await
    }

    pub async fn get_send_as_peers(
        &self,
        peer: impl Into<PeerRef>,
    ) -> Result<Vec<tl::enums::Peer>, InvocationError> {
        let peer = peer.into().resolve(self).await?;
        let input_peer = self.inner.peer_cache.read().await.peer_to_input(&peer)?;
        let req = tl::functions::channels::GetSendAs {
            for_paid_reactions: false,
            for_live_stories: false,
            peer: input_peer,
        };
        let body = self.rpc_call_raw(&req).await?;
        let mut cur = Cursor::from_slice(&body);
        let tl::enums::channels::SendAsPeers::SendAsPeers(result) =
            tl::enums::channels::SendAsPeers::deserialize(&mut cur)?;
        self.cache_users_slice(&result.users).await;
        self.cache_chats_slice(&result.chats).await;
        Ok(result
            .peers
            .into_iter()
            .map(|p| match p {
                tl::enums::SendAsPeer::SendAsPeer(sp) => sp.peer,
            })
            .collect())
    }

    pub async fn set_default_send_as(
        &self,
        peer: impl Into<PeerRef>,
        send_as_peer: impl Into<PeerRef>,
    ) -> Result<(), InvocationError> {
        let peer = peer.into().resolve(self).await?;
        let input_peer = self.inner.peer_cache.read().await.peer_to_input(&peer)?;
        let send_as = send_as_peer.into().resolve(self).await?;
        let send_as_input = self.inner.peer_cache.read().await.peer_to_input(&send_as)?;
        let req = tl::functions::messages::SaveDefaultSendAs {
            peer: input_peer,
            send_as: send_as_input,
        };
        self.rpc_write(&req).await
    }
}
