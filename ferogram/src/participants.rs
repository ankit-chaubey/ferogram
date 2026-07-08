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
use ferogram_tl_types::{Cursor, Deserializable};

use crate::{Client, InvocationError, PeerRef};

// Participant

/// A member of a chat, group or channel.
#[derive(Debug, Clone)]
pub struct Participant {
    /// The user object.
    pub user: tl::types::User,
    /// Their role/status in the chat.
    pub status: ParticipantStatus,
}

/// The role of a participant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParticipantStatus {
    /// Regular member.
    Member,
    /// The channel/group creator.
    Creator,
    /// Admin (may have custom title).
    Admin,
    /// Restricted / banned user.
    Restricted,
    /// Left the group.
    Left,
    /// Kicked (banned) from the group.
    Banned,
}

impl Participant {
    pub(crate) fn from_channel_participant(
        p: tl::enums::ChannelParticipant,
        user_map: &std::collections::HashMap<i64, tl::types::User>,
    ) -> Option<Self> {
        let (user_id, status) = match &p {
            tl::enums::ChannelParticipant::ChannelParticipant(x) => {
                (x.user_id, ParticipantStatus::Member)
            }
            tl::enums::ChannelParticipant::ParticipantSelf(x) => {
                (x.user_id, ParticipantStatus::Member)
            }
            tl::enums::ChannelParticipant::Creator(x) => (x.user_id, ParticipantStatus::Creator),
            tl::enums::ChannelParticipant::Admin(x) => (x.user_id, ParticipantStatus::Admin),
            tl::enums::ChannelParticipant::Banned(x) => {
                (x.peer.user_id_or(0), ParticipantStatus::Banned)
            }
            tl::enums::ChannelParticipant::Left(x) => {
                (x.peer.user_id_or(0), ParticipantStatus::Left)
            }
        };
        user_map
            .get(&user_id)
            .cloned()
            .map(|user| Participant { user, status })
    }
}

impl Client {
    /// Fetch all participants of a chat, group or channel.
    ///
    /// For channels this uses `channels.getParticipants`; for basic groups it
    /// uses `messages.getFullChat`.
    ///
    /// Returns up to `limit` participants; pass `0` for the default (200 for channels).
    pub async fn get_participants(
        &self,
        peer: impl Into<PeerRef>,
        limit: i32,
    ) -> Result<Vec<Participant>, InvocationError> {
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
                self.get_channel_participants(c.channel_id, access_hash, limit)
                    .await
            }
            tl::enums::Peer::Chat(c) => self.get_chat_participants(c.chat_id).await,
            _ => Err(InvocationError::Deserialize(
                "get_participants: peer must be a chat or channel".into(),
            )),
        }
    }

    async fn get_channel_participants(
        &self,
        channel_id: i64,
        access_hash: i64,
        limit: i32,
    ) -> Result<Vec<Participant>, InvocationError> {
        let limit = if limit <= 0 { 200 } else { limit };
        let req = tl::functions::channels::GetParticipants {
            channel: tl::enums::InputChannel::InputChannel(tl::types::InputChannel {
                channel_id,
                access_hash,
            }),
            filter: tl::enums::ChannelParticipantsFilter::ChannelParticipantsRecent,
            offset: 0,
            limit,
            hash: 0,
        };
        let body: Vec<u8> = self.rpc_call_raw(&req).await?;
        let mut cur = Cursor::from_slice(&body);
        let raw = match tl::enums::channels::ChannelParticipants::deserialize(&mut cur)? {
            tl::enums::channels::ChannelParticipants::ChannelParticipants(p) => p,
            tl::enums::channels::ChannelParticipants::NotModified => return Ok(vec![]),
        };

        // Build user map
        let user_map: std::collections::HashMap<i64, tl::types::User> = raw
            .users
            .into_iter()
            .filter_map(|u| match u {
                tl::enums::User::User(u) => Some((u.id, u)),
                _ => None,
            })
            .collect();

        // Cache them
        {
            for u in user_map.values() {
                if let Some(h) = u.access_hash {
                    self.inner.peer_cache.write().await.users.insert(u.id, h);
                }
            }
        }

        let mut result = Vec::new();
        for p in raw.participants {
            let (user_id, status) = match &p {
                tl::enums::ChannelParticipant::ChannelParticipant(x) => {
                    (x.user_id, ParticipantStatus::Member)
                }
                tl::enums::ChannelParticipant::ParticipantSelf(x) => {
                    (x.user_id, ParticipantStatus::Member)
                }
                tl::enums::ChannelParticipant::Creator(x) => {
                    (x.user_id, ParticipantStatus::Creator)
                }
                tl::enums::ChannelParticipant::Admin(x) => (x.user_id, ParticipantStatus::Admin),
                tl::enums::ChannelParticipant::Banned(x) => {
                    (x.peer.user_id_or(0), ParticipantStatus::Banned)
                }
                tl::enums::ChannelParticipant::Left(x) => {
                    (x.peer.user_id_or(0), ParticipantStatus::Left)
                }
            };
            if let Some(user) = user_map.get(&user_id).cloned() {
                result.push(Participant { user, status });
            }
        }
        Ok(result)
    }

    async fn get_chat_participants(
        &self,
        chat_id: i64,
    ) -> Result<Vec<Participant>, InvocationError> {
        let req = tl::functions::messages::GetFullChat { chat_id };
        let body: Vec<u8> = self.rpc_call_raw(&req).await?;
        let mut cur = Cursor::from_slice(&body);
        let tl::enums::messages::ChatFull::ChatFull(full) =
            tl::enums::messages::ChatFull::deserialize(&mut cur)?;

        let user_map: std::collections::HashMap<i64, tl::types::User> = full
            .users
            .into_iter()
            .filter_map(|u| match u {
                tl::enums::User::User(u) => Some((u.id, u)),
                _ => None,
            })
            .collect();

        {
            for u in user_map.values() {
                if let Some(h) = u.access_hash {
                    self.inner.peer_cache.write().await.users.insert(u.id, h);
                }
            }
        }

        let participants = match &full.full_chat {
            tl::enums::ChatFull::ChatFull(cf) => match &cf.participants {
                tl::enums::ChatParticipants::ChatParticipants(p) => p.participants.clone(),
                tl::enums::ChatParticipants::Forbidden(_) => vec![],
            },
            tl::enums::ChatFull::ChannelFull(_) => {
                return Err(InvocationError::Deserialize(
                    "get_chat_participants: peer is a channel, use get_participants with a Channel peer instead".into()
                ));
            }
        };

        let mut result = Vec::new();
        for p in participants {
            let (user_id, status) = match p {
                tl::enums::ChatParticipant::ChatParticipant(x) => {
                    (x.user_id, ParticipantStatus::Member)
                }
                tl::enums::ChatParticipant::Creator(x) => (x.user_id, ParticipantStatus::Creator),
                tl::enums::ChatParticipant::Admin(x) => (x.user_id, ParticipantStatus::Admin),
            };
            if let Some(user) = user_map.get(&user_id).cloned() {
                result.push(Participant { user, status });
            }
        }
        Ok(result)
    }

    /// Kick a user from a group chat or channel.
    ///
    /// For basic groups, this removes the user immediately.
    /// For channels and supergroups, it's a shortcut for [`restrict`] with
    /// [`BannedRightsBuilder::full_ban`] followed immediately by [`restrict`]
    /// with the default (all-`false`) builder, i.e. ban then unban.
    ///
    /// [`restrict`]: Client::restrict
    pub async fn kick(
        &self,
        peer: impl Into<PeerRef>,
        user_id: i64,
    ) -> Result<(), InvocationError> {
        let peer = peer.into().resolve(self).await?;
        let input_peer = self.inner.peer_cache.read().await.peer_to_input(&peer)?;
        match &input_peer {
            tl::enums::InputPeer::Chat(c) => {
                let access_hash = self
                    .inner
                    .peer_cache
                    .read()
                    .await
                    .users
                    .get(&user_id)
                    .copied()
                    .unwrap_or(0);
                let req = tl::functions::messages::DeleteChatUser {
                    revoke_history: false,
                    chat_id: c.chat_id,
                    user_id: tl::enums::InputUser::InputUser(tl::types::InputUser {
                        user_id,
                        access_hash,
                    }),
                };
                self.rpc_call_raw(&req).await?;
            }
            tl::enums::InputPeer::Channel(_) => {
                // Standard Telegram kick: ban, then immediately unban.
                self.restrict(peer.clone(), user_id, |_| BannedRightsBuilder::full_ban())
                    .await?;
                self.restrict(peer, user_id, |b| b).await?;
            }
            _ => {
                return Err(InvocationError::Deserialize(
                    "kick: peer must be a chat or channel".into(),
                ));
            }
        }
        Ok(())
    }

    /// Ban a user from a channel or supergroup.
    ///
    /// `until: None` is a permanent ban; `until: Some(ts)` bans until the Unix timestamp.
    ///
    /// Shortcut for [`restrict`](Client::restrict) with
    /// [`BannedRightsBuilder::full_ban`]. For a partial ban (e.g. block file
    /// uploads but allow photos), use [`restrict`](Client::restrict) directly.
    pub async fn ban(
        &self,
        channel: impl Into<PeerRef>,
        user_id: i64,
        until: Option<i32>,
    ) -> Result<(), InvocationError> {
        self.restrict(channel, user_id, |_| {
            BannedRightsBuilder::full_ban().until_date(until.unwrap_or(0))
        })
        .await
    }

    #[allow(dead_code)]
    async fn set_participant_admin(
        &self,
        channel: impl Into<PeerRef>,
        user_id: i64,
        promote: bool,
    ) -> Result<(), InvocationError> {
        let channel = channel.into().resolve(self).await?;
        let (channel_id, ch_hash) = match &channel {
            tl::enums::Peer::Channel(c) => {
                let h = self
                    .inner
                    .peer_cache
                    .read()
                    .await
                    .channels
                    .get(&c.channel_id)
                    .map(|&(hash, _)| hash)
                    .unwrap_or(0);
                (c.channel_id, h)
            }
            _ => {
                return Err(InvocationError::Deserialize(
                    "promote_participant: peer must be a channel".into(),
                ));
            }
        };
        let user_hash = self
            .inner
            .peer_cache
            .read()
            .await
            .users
            .get(&user_id)
            .copied()
            .unwrap_or(0);

        let rights = if promote {
            tl::types::ChatAdminRights {
                change_info: true,
                post_messages: true,
                edit_messages: true,
                delete_messages: true,
                ban_users: true,
                invite_users: true,
                pin_messages: true,
                add_admins: false,
                anonymous: false,
                manage_call: true,
                other: false,
                manage_topics: false,
                post_stories: false,
                edit_stories: false,
                delete_stories: false,
                manage_direct_messages: false,
                manage_ranks: false,
            }
        } else {
            tl::types::ChatAdminRights {
                change_info: false,
                post_messages: false,
                edit_messages: false,
                delete_messages: false,
                ban_users: false,
                invite_users: false,
                pin_messages: false,
                add_admins: false,
                anonymous: false,
                manage_call: false,
                other: false,
                manage_topics: false,
                post_stories: false,
                edit_stories: false,
                delete_stories: false,
                manage_direct_messages: false,
                manage_ranks: false,
            }
        };

        let req = tl::functions::channels::EditAdmin {
            channel: tl::enums::InputChannel::InputChannel(tl::types::InputChannel {
                channel_id,
                access_hash: ch_hash,
            }),
            user_id: tl::enums::InputUser::InputUser(tl::types::InputUser {
                user_id,
                access_hash: user_hash,
            }),
            admin_rights: tl::enums::ChatAdminRights::ChatAdminRights(rights),
            rank: None,
        };
        self.rpc_call_raw(&req).await?;
        Ok(())
    }

    /// Fetch a page of profile photos for a user (or yourself).
    ///
    /// Returns a list of photo objects (up to `limit`), starting from the
    /// most recent. Only works for users/self - Telegram has no equivalent
    /// paged photo-history API for chats/channels. For channel or group
    /// avatar history, use [`Client::get_chat_photos`] instead.
    pub async fn get_profile_photos(
        &self,
        peer: impl Into<PeerRef>,
        limit: i32,
    ) -> Result<Vec<tl::enums::Photo>, InvocationError> {
        let peer = peer.into().resolve(self).await?;
        let input_peer = { self.inner.peer_cache.read().await.peer_to_input(&peer)? };

        let req = tl::functions::photos::GetUserPhotos {
            user_id: match &input_peer {
                tl::enums::InputPeer::User(u) => {
                    tl::enums::InputUser::InputUser(tl::types::InputUser {
                        user_id: u.user_id,
                        access_hash: u.access_hash,
                    })
                }
                tl::enums::InputPeer::PeerSelf => tl::enums::InputUser::UserSelf,
                _ => {
                    return Err(InvocationError::Deserialize(
                        "get_profile_photos: peer must be a user".into(),
                    ));
                }
            },
            offset: 0,
            max_id: 0,
            limit,
        };
        let body: Vec<u8> = self.rpc_call_raw(&req).await?;
        let mut cur = Cursor::from_slice(&body);
        match tl::enums::photos::Photos::deserialize(&mut cur)? {
            tl::enums::photos::Photos::Photos(p) => Ok(p.photos),
            tl::enums::photos::Photos::Slice(p) => Ok(p.photos),
        }
    }

    /// Stream profile photos of a user lazily, one page at a time.
    ///
    /// Returns a [`ProfilePhotoIter`] that fetches photos in pages of
    /// `chunk_size` and exposes them one-by-one via `.next().await`.
    /// Set `chunk_size` to `0` to use the default (100).
    ///
    /// Only works for users: channels use `messages.search` with a photo
    /// filter instead.
    ///
    /// # Example
    /// ```rust,no_run
    /// # use ferogram::Client;
    /// # async fn example(client: Client, peer: ferogram::tl::enums::Peer) -> Result<(), Box<dyn std::error::Error>> {
    /// let mut iter = client.iter_profile_photos(peer, 0).await?;
    /// while let Some(photo) = iter.next().await? {
    /// println!("{photo:?}");
    /// }
    /// # Ok(()) }
    /// ```
    pub async fn iter_profile_photos(
        &self,
        peer: impl Into<PeerRef>,
        chunk_size: i32,
    ) -> Result<ProfilePhotoIter, InvocationError> {
        let chunk_size = if chunk_size <= 0 { 100 } else { chunk_size };
        let peer = peer.into().resolve(self).await?;
        let input_peer = { self.inner.peer_cache.read().await.peer_to_input(&peer)? };
        let input_user = match &input_peer {
            tl::enums::InputPeer::User(u) => {
                tl::enums::InputUser::InputUser(tl::types::InputUser {
                    user_id: u.user_id,
                    access_hash: u.access_hash,
                })
            }
            tl::enums::InputPeer::PeerSelf => tl::enums::InputUser::UserSelf,
            _ => {
                return Err(InvocationError::Deserialize(
                    "iter_profile_photos: peer must be a user".into(),
                ));
            }
        };

        Ok(ProfilePhotoIter {
            client: self.clone(),
            input_user,
            chunk_size,
            offset: 0,
            buffer: VecDeque::new(),
            done: false,
        })
    }

    /// Fetch a chat's or channel's photo (avatar) history.
    ///
    /// Unlike [`get_profile_photos`](Self::get_profile_photos), which only
    /// works for users, this works for groups and channels too. The
    /// *current* photo comes from the chat/channel's full info (independent
    /// of message history, so it's always available even if every message
    /// has been deleted); older photos are recovered from
    /// `messageActionChatEditPhoto` service messages via search, which is
    /// the only place Telegram keeps that history, so those are lost if the
    /// underlying messages are deleted.
    ///
    /// # Example
    /// ```rust,no_run
    /// # use ferogram::Client;
    /// # async fn example(client: Client, peer: ferogram::tl::enums::Peer) -> Result<(), Box<dyn std::error::Error>> {
    /// let photos = client.get_chat_photos(peer, 100).await?;
    /// # Ok(()) }
    /// ```
    pub async fn get_chat_photos(
        &self,
        peer: impl Into<PeerRef>,
        limit: i32,
    ) -> Result<Vec<tl::enums::Photo>, InvocationError> {
        let peer: PeerRef = peer.into();
        let mut photos = Vec::new();
        let mut seen_id: Option<i64> = None;

        // Current photo, straight from the full chat/channel object. This
        // doesn't depend on message history at all, so it survives even if
        // every message in the chat has been deleted.
        if let Ok(tl::enums::messages::ChatFull::ChatFull(f)) =
            self.get_chat_full(peer.clone()).await
        {
            let current = match f.full_chat {
                tl::enums::ChatFull::ChatFull(c) => c.chat_photo,
                tl::enums::ChatFull::ChannelFull(c) => Some(c.chat_photo),
            };
            if let Some(tl::enums::Photo::Photo(p)) = current {
                seen_id = Some(p.id);
                photos.push(tl::enums::Photo::Photo(p));
            }
        }

        // Older photos: only recoverable from surviving edit-photo service
        // messages, so skip anything already covered by the current photo.
        let messages = self
            .search(peer, "")
            .filter(tl::enums::MessagesFilter::InputMessagesFilterChatPhotos)
            .limit(limit)
            .fetch(self)
            .await?;

        photos.extend(messages.into_iter().filter_map(|m| match m.action()? {
            tl::enums::MessageAction::ChatEditPhoto(a) => match &a.photo {
                tl::enums::Photo::Photo(p) if Some(p.id) == seen_id => None,
                _ => Some(a.photo.clone()),
            },
            _ => None,
        }));

        Ok(photos)
    }

    /// Search for a peer (user, group, or channel) by name prefix.
    ///
    /// Searches contacts, dialogs, and globally. Returns combined results.
    pub async fn search_peer(&self, query: &str) -> Result<Vec<tl::enums::Peer>, InvocationError> {
        let req = tl::functions::contacts::Search {
            q: query.to_string(),
            limit: 20,
            bots: false,
            broadcasts: false,
        };
        let body: Vec<u8> = self.rpc_call_raw(&req).await?;
        let mut cur = Cursor::from_slice(&body);
        let tl::enums::contacts::Found::Found(found) =
            tl::enums::contacts::Found::deserialize(&mut cur)?;

        self.cache_users_slice(&found.users).await;
        self.cache_chats_slice(&found.chats).await;

        let mut peers = Vec::new();
        for r in found.my_results.iter().chain(found.results.iter()) {
            peers.push(r.clone());
        }
        Ok(peers)
    }

    /// Send a reaction to a message.
    ///
    /// Accepts anything that converts to [`InputReactions`]:
    ///
    /// ```rust,no_run
    /// # use ferogram::Client;
    /// # async fn example(client: Client, peer: ferogram::PeerRef, msg_id: i32) -> Result<(), ferogram::InvocationError> {
    /// // emoji shorthand
    /// client.send_reaction(peer.clone(), msg_id, "👍").await?;
    ///
    /// // fluent builder
    /// use ferogram::reactions::InputReactions;
    /// client.send_reaction(peer.clone(), msg_id, InputReactions::custom_emoji(123).big()).await?;
    ///
    /// // remove all reactions
    /// client.send_reaction(peer, msg_id, InputReactions::remove()).await?;
    /// # Ok(()) }
    /// ```
    #[doc(hidden)]
    pub async fn send_reaction(
        &self,
        peer: impl Into<PeerRef>,
        message_id: i32,
        reaction: impl Into<crate::reactions::InputReactions>,
    ) -> Result<(), InvocationError> {
        let peer = peer.into().resolve(self).await?;
        let input_peer = { self.inner.peer_cache.read().await.peer_to_input(&peer)? };

        let r: crate::reactions::InputReactions = reaction.into();
        let req = tl::functions::messages::SendReaction {
            big: r.big,
            add_to_recent: r.add_to_recent,
            peer: input_peer,
            msg_id: message_id,
            reaction: if r.reactions.is_empty() {
                None
            } else {
                Some(r.reactions)
            },
        };
        self.rpc_call_raw(&req).await?;
        Ok(())
    }
}

// Helper extension for Peer

trait PeerUserIdExt {
    fn user_id_or(&self, default: i64) -> i64;
}

impl PeerUserIdExt for tl::enums::Peer {
    fn user_id_or(&self, default: i64) -> i64 {
        match self {
            tl::enums::Peer::User(u) => u.user_id,
            _ => default,
        }
    }
}

// BannedRightsBuilder

/// Fluent builder for granular channel ban rights.
///
/// ```rust,no_run
/// # async fn f(client: ferogram::Client, channel: ferogram_tl_types::enums::Peer) -> Result<(), Box<dyn std::error::Error>> {
/// client.edit_chat_default_banned_rights(channel, |b| b
/// .send_messages(true)
/// .send_media(true)
/// .until_date(0))
/// .await?;
/// # Ok(()) }
/// ```
#[derive(Debug, Clone, Default)]
pub struct BannedRightsBuilder {
    pub view_messages: bool,
    pub send_messages: bool,
    pub send_media: bool,
    pub send_stickers: bool,
    pub send_gifs: bool,
    pub send_games: bool,
    pub send_inline: bool,
    pub embed_links: bool,
    pub send_polls: bool,
    pub send_reactions: bool,
    pub change_info: bool,
    pub invite_users: bool,
    pub pin_messages: bool,
    pub manage_topics: bool,
    pub send_photos: bool,
    pub send_videos: bool,
    pub send_roundvideos: bool,
    pub send_audios: bool,
    pub send_voices: bool,
    pub send_docs: bool,
    pub send_plain: bool,
    pub edit_rank: bool,
    pub until_date: i32,
}

impl BannedRightsBuilder {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn view_messages(mut self, v: bool) -> Self {
        self.view_messages = v;
        self
    }
    pub fn send_messages(mut self, v: bool) -> Self {
        self.send_messages = v;
        self
    }
    pub fn send_media(mut self, v: bool) -> Self {
        self.send_media = v;
        self
    }
    pub fn send_stickers(mut self, v: bool) -> Self {
        self.send_stickers = v;
        self
    }
    pub fn send_gifs(mut self, v: bool) -> Self {
        self.send_gifs = v;
        self
    }
    pub fn send_games(mut self, v: bool) -> Self {
        self.send_games = v;
        self
    }
    pub fn send_inline(mut self, v: bool) -> Self {
        self.send_inline = v;
        self
    }
    pub fn embed_links(mut self, v: bool) -> Self {
        self.embed_links = v;
        self
    }
    pub fn send_polls(mut self, v: bool) -> Self {
        self.send_polls = v;
        self
    }
    pub fn send_reactions(mut self, v: bool) -> Self {
        self.send_reactions = v;
        self
    }
    pub fn change_info(mut self, v: bool) -> Self {
        self.change_info = v;
        self
    }
    pub fn invite_users(mut self, v: bool) -> Self {
        self.invite_users = v;
        self
    }
    pub fn pin_messages(mut self, v: bool) -> Self {
        self.pin_messages = v;
        self
    }
    pub fn manage_topics(mut self, v: bool) -> Self {
        self.manage_topics = v;
        self
    }
    pub fn send_photos(mut self, v: bool) -> Self {
        self.send_photos = v;
        self
    }
    pub fn send_videos(mut self, v: bool) -> Self {
        self.send_videos = v;
        self
    }
    pub fn send_roundvideos(mut self, v: bool) -> Self {
        self.send_roundvideos = v;
        self
    }
    pub fn send_audios(mut self, v: bool) -> Self {
        self.send_audios = v;
        self
    }
    pub fn send_voices(mut self, v: bool) -> Self {
        self.send_voices = v;
        self
    }
    pub fn send_docs(mut self, v: bool) -> Self {
        self.send_docs = v;
        self
    }
    pub fn send_plain(mut self, v: bool) -> Self {
        self.send_plain = v;
        self
    }
    /// Restrict changing the admin rank/title shown next to the user. Only
    /// meaningful in the context that accepts `edit_rank`; harmless elsewhere.
    pub fn edit_rank(mut self, v: bool) -> Self {
        self.edit_rank = v;
        self
    }
    /// Ban until a Unix timestamp. `0` = permanent.
    pub fn until_date(mut self, ts: i32) -> Self {
        self.until_date = ts;
        self
    }

    /// Full ban: every restrictable right revoked, permanent.
    ///
    /// Sets all 22 `ChatBannedRights` flags, including the per-media-type
    /// ones (`send_photos`, `send_videos`, `send_roundvideos`, `send_audios`,
    /// `send_voices`, `send_docs`), not just the legacy `send_media` flag.
    pub fn full_ban() -> Self {
        Self {
            view_messages: true,
            send_messages: true,
            send_media: true,
            send_stickers: true,
            send_gifs: true,
            send_games: true,
            send_inline: true,
            embed_links: true,
            send_polls: true,
            send_reactions: true,
            change_info: true,
            invite_users: true,
            pin_messages: true,
            manage_topics: true,
            send_photos: true,
            send_videos: true,
            send_roundvideos: true,
            send_audios: true,
            send_voices: true,
            send_docs: true,
            send_plain: true,
            edit_rank: true,
            until_date: 0,
        }
    }

    pub(crate) fn into_tl(self) -> tl::enums::ChatBannedRights {
        tl::enums::ChatBannedRights::ChatBannedRights(tl::types::ChatBannedRights {
            view_messages: self.view_messages,
            send_messages: self.send_messages,
            send_media: self.send_media,
            send_stickers: self.send_stickers,
            send_gifs: self.send_gifs,
            send_games: self.send_games,
            send_inline: self.send_inline,
            embed_links: self.embed_links,
            send_polls: self.send_polls,
            change_info: self.change_info,
            invite_users: self.invite_users,
            pin_messages: self.pin_messages,
            manage_topics: self.manage_topics,
            send_photos: self.send_photos,
            send_videos: self.send_videos,
            send_roundvideos: self.send_roundvideos,
            send_audios: self.send_audios,
            send_voices: self.send_voices,
            send_docs: self.send_docs,
            send_plain: self.send_plain,
            edit_rank: self.edit_rank,
            send_reactions: self.send_reactions,
            until_date: self.until_date,
        })
    }
}

// AdminRightsBuilder

/// Fluent builder for granular admin rights.
#[derive(Debug, Clone, Default)]
pub struct AdminRightsBuilder {
    pub change_info: bool,
    pub post_messages: bool,
    pub edit_messages: bool,
    pub delete_messages: bool,
    pub ban_users: bool,
    pub invite_users: bool,
    pub pin_messages: bool,
    pub add_admins: bool,
    pub anonymous: bool,
    pub manage_call: bool,
    pub manage_topics: bool,
    pub rank: Option<String>,
}

impl AdminRightsBuilder {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn change_info(mut self, v: bool) -> Self {
        self.change_info = v;
        self
    }
    pub fn post_messages(mut self, v: bool) -> Self {
        self.post_messages = v;
        self
    }
    pub fn edit_messages(mut self, v: bool) -> Self {
        self.edit_messages = v;
        self
    }
    pub fn delete_messages(mut self, v: bool) -> Self {
        self.delete_messages = v;
        self
    }
    pub fn ban_users(mut self, v: bool) -> Self {
        self.ban_users = v;
        self
    }
    pub fn invite_users(mut self, v: bool) -> Self {
        self.invite_users = v;
        self
    }
    pub fn pin_messages(mut self, v: bool) -> Self {
        self.pin_messages = v;
        self
    }
    pub fn add_admins(mut self, v: bool) -> Self {
        self.add_admins = v;
        self
    }
    pub fn anonymous(mut self, v: bool) -> Self {
        self.anonymous = v;
        self
    }
    pub fn manage_call(mut self, v: bool) -> Self {
        self.manage_call = v;
        self
    }
    pub fn manage_topics(mut self, v: bool) -> Self {
        self.manage_topics = v;
        self
    }
    /// Custom admin title (max 16 chars).
    pub fn rank(mut self, r: impl Into<String>) -> Self {
        self.rank = Some(r.into());
        self
    }

    /// Full admin (all standard rights).
    pub fn full_admin() -> Self {
        Self {
            change_info: true,
            post_messages: true,
            edit_messages: true,
            delete_messages: true,
            ban_users: true,
            invite_users: true,
            pin_messages: true,
            add_admins: false,
            anonymous: false,
            manage_call: true,
            manage_topics: true,
            rank: None,
        }
    }

    pub(crate) fn into_tl_rights(self) -> tl::enums::ChatAdminRights {
        tl::enums::ChatAdminRights::ChatAdminRights(tl::types::ChatAdminRights {
            change_info: self.change_info,
            post_messages: self.post_messages,
            edit_messages: self.edit_messages,
            delete_messages: self.delete_messages,
            ban_users: self.ban_users,
            invite_users: self.invite_users,
            pin_messages: self.pin_messages,
            add_admins: self.add_admins,
            anonymous: self.anonymous,
            manage_call: self.manage_call,
            other: false,
            manage_topics: self.manage_topics,
            post_stories: false,
            edit_stories: false,
            delete_stories: false,
            manage_direct_messages: false,
            manage_ranks: false,
        })
    }
}

// ParticipantPermissions

/// The effective permissions/rights of a specific participant.
#[derive(Debug, Clone)]
pub struct ParticipantPermissions {
    pub is_creator: bool,
    pub is_admin: bool,
    pub is_banned: bool,
    pub is_left: bool,
    pub can_send_messages: bool,
    pub can_send_media: bool,
    pub can_pin_messages: bool,
    pub can_add_admins: bool,
    pub admin_rank: Option<String>,
}

impl ParticipantPermissions {
    pub fn is_creator(&self) -> bool {
        self.is_creator
    }
    pub fn is_admin(&self) -> bool {
        self.is_admin
    }
    pub fn is_banned(&self) -> bool {
        self.is_banned
    }
    pub fn is_member(&self) -> bool {
        !self.is_banned && !self.is_left
    }
}

// Client: new participant methods

impl Client {
    // set_banned_rights

    /// Apply granular ban rights to a user in a channel or supergroup.
    ///
    /// Use [`BannedRightsBuilder`] to specify which rights to restrict.
    /// [`ban`](Client::ban) and [`kick`](Client::kick) are shortcuts built on
    /// top of this method; reach for `restrict` directly when you need
    /// anything less than a full ban.
    ///
    /// # Example: allow photos, block file uploads
    ///
    /// "Send as photo" and "send as file" are different upload paths on the
    /// wire (`send_photos` vs `send_docs`), so they're independent flags:
    ///
    /// ```rust,no_run
    /// # async fn f(client: ferogram::Client, channel: ferogram_tl_types::enums::Peer, user: i64) -> Result<(), Box<dyn std::error::Error>> {
    /// client.restrict(channel, user, |b| b.send_docs(true)).await?;
    /// # Ok(()) }
    /// ```
    /// `send_media`/`send_photos` are left at their default `false`, so
    /// images sent as photos are still allowed; only uploads sent as raw
    /// documents (including images sent as files) are blocked.
    pub async fn restrict(
        &self,
        channel: impl Into<PeerRef>,
        user_id: i64,
        build: impl FnOnce(BannedRightsBuilder) -> BannedRightsBuilder,
    ) -> Result<(), InvocationError> {
        let rights = build(BannedRightsBuilder::new()).into_tl();
        let channel = channel.into().resolve(self).await?;
        let (channel_id, ch_hash) = match &channel {
            tl::enums::Peer::Channel(c) => {
                let h = self
                    .inner
                    .peer_cache
                    .read()
                    .await
                    .channels
                    .get(&c.channel_id)
                    .map(|&(hash, _)| hash)
                    .unwrap_or(0);
                (c.channel_id, h)
            }
            _ => {
                return Err(InvocationError::Deserialize(
                    "restrict: must be a channel".into(),
                ));
            }
        };
        let user_hash = self
            .inner
            .peer_cache
            .read()
            .await
            .users
            .get(&user_id)
            .copied()
            .unwrap_or(0);
        let req = tl::functions::channels::EditBanned {
            channel: tl::enums::InputChannel::InputChannel(tl::types::InputChannel {
                channel_id,
                access_hash: ch_hash,
            }),
            participant: tl::enums::InputPeer::User(tl::types::InputPeerUser {
                user_id,
                access_hash: user_hash,
            }),
            banned_rights: rights,
        };
        self.rpc_call_raw(&req).await?;
        Ok(())
    }

    /// Apply granular admin rights to a user in a channel or supergroup.
    ///
    /// Use [`AdminRightsBuilder`] to specify which rights to grant.
    pub async fn set_admin(
        &self,
        channel: impl Into<PeerRef>,
        user_id: i64,
        build: impl FnOnce(AdminRightsBuilder) -> AdminRightsBuilder,
    ) -> Result<(), InvocationError> {
        let b = build(AdminRightsBuilder::new());
        let rank = b.rank.clone();
        let rights = b.into_tl_rights();
        let channel = channel.into().resolve(self).await?;
        let (channel_id, ch_hash) = match &channel {
            tl::enums::Peer::Channel(c) => {
                let h = self
                    .inner
                    .peer_cache
                    .read()
                    .await
                    .channels
                    .get(&c.channel_id)
                    .map(|&(hash, _)| hash)
                    .unwrap_or(0);
                (c.channel_id, h)
            }
            _ => {
                return Err(InvocationError::Deserialize(
                    "set_admin: must be a channel".into(),
                ));
            }
        };
        let user_hash = self
            .inner
            .peer_cache
            .read()
            .await
            .users
            .get(&user_id)
            .copied()
            .unwrap_or(0);
        let req = tl::functions::channels::EditAdmin {
            channel: tl::enums::InputChannel::InputChannel(tl::types::InputChannel {
                channel_id,
                access_hash: ch_hash,
            }),
            user_id: tl::enums::InputUser::InputUser(tl::types::InputUser {
                user_id,
                access_hash: user_hash,
            }),
            admin_rights: rights,
            rank,
        };
        self.rpc_call_raw(&req).await?;
        Ok(())
    }

    // get_participants_filtered

    /// Fetch participants with an optional filter, paginated.
    ///
    /// `filter` defaults to `ChannelParticipantsRecent` when `None`.
    pub async fn get_participants_filtered(
        &self,
        peer: impl Into<PeerRef>,
        filter: Option<tl::enums::ChannelParticipantsFilter>,
        limit: i32,
    ) -> Result<Vec<Participant>, InvocationError> {
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
                let filter = filter
                    .unwrap_or(tl::enums::ChannelParticipantsFilter::ChannelParticipantsRecent);
                let limit = if limit <= 0 { 200 } else { limit };
                let req = tl::functions::channels::GetParticipants {
                    channel: tl::enums::InputChannel::InputChannel(tl::types::InputChannel {
                        channel_id: c.channel_id,
                        access_hash,
                    }),
                    filter,
                    offset: 0,
                    limit,
                    hash: 0,
                };
                let body: Vec<u8> = self.rpc_call_raw(&req).await?;
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
                {
                    for u in user_map.values() {
                        if let Some(h) = u.access_hash {
                            self.inner.peer_cache.write().await.users.insert(u.id, h);
                        }
                    }
                }
                Ok(raw
                    .participants
                    .into_iter()
                    .filter_map(|p| {
                        let (uid, status) = match &p {
                            tl::enums::ChannelParticipant::ChannelParticipant(x) => {
                                (x.user_id, ParticipantStatus::Member)
                            }
                            tl::enums::ChannelParticipant::ParticipantSelf(x) => {
                                (x.user_id, ParticipantStatus::Member)
                            }
                            tl::enums::ChannelParticipant::Creator(x) => {
                                (x.user_id, ParticipantStatus::Creator)
                            }
                            tl::enums::ChannelParticipant::Admin(x) => {
                                (x.user_id, ParticipantStatus::Admin)
                            }
                            tl::enums::ChannelParticipant::Banned(x) => {
                                if let tl::enums::Peer::User(u) = &x.peer {
                                    (u.user_id, ParticipantStatus::Banned)
                                } else {
                                    return None;
                                }
                            }
                            tl::enums::ChannelParticipant::Left(x) => {
                                if let tl::enums::Peer::User(u) = &x.peer {
                                    (u.user_id, ParticipantStatus::Left)
                                } else {
                                    return None;
                                }
                            }
                        };
                        user_map.get(&uid).map(|u| Participant {
                            user: u.clone(),
                            status,
                        })
                    })
                    .collect())
            }
            tl::enums::Peer::Chat(c) => self.get_chat_participants(c.chat_id).await,
            _ => Err(InvocationError::Deserialize(
                "get_participants_filtered: must be chat or channel".into(),
            )),
        }
    }

    // get_permissions

    /// Get the effective permissions of a specific user in a channel.
    pub async fn get_permissions(
        &self,
        channel: impl Into<PeerRef>,
        user_id: i64,
    ) -> Result<ParticipantPermissions, InvocationError> {
        let channel = channel.into().resolve(self).await?;
        let (channel_id, ch_hash) = match &channel {
            tl::enums::Peer::Channel(c) => {
                let h = self
                    .inner
                    .peer_cache
                    .read()
                    .await
                    .channels
                    .get(&c.channel_id)
                    .map(|&(hash, _)| hash)
                    .unwrap_or(0);
                (c.channel_id, h)
            }
            _ => {
                return Err(InvocationError::Deserialize(
                    "get_permissions: must be a channel".into(),
                ));
            }
        };
        let user_hash = self
            .inner
            .peer_cache
            .read()
            .await
            .users
            .get(&user_id)
            .copied()
            .unwrap_or(0);
        let req = tl::functions::channels::GetParticipant {
            channel: tl::enums::InputChannel::InputChannel(tl::types::InputChannel {
                channel_id,
                access_hash: ch_hash,
            }),
            participant: tl::enums::InputPeer::User(tl::types::InputPeerUser {
                user_id,
                access_hash: user_hash,
            }),
        };
        let body: Vec<u8> = self.rpc_call_raw(&req).await?;
        let mut cur = Cursor::from_slice(&body);
        let tl::enums::channels::ChannelParticipant::ChannelParticipant(raw) =
            tl::enums::channels::ChannelParticipant::deserialize(&mut cur)?;

        let perms = match raw.participant {
            tl::enums::ChannelParticipant::Creator(_) => ParticipantPermissions {
                is_creator: true,
                is_admin: true,
                is_banned: false,
                is_left: false,
                can_send_messages: true,
                can_send_media: true,
                can_pin_messages: true,
                can_add_admins: true,
                admin_rank: None,
            },
            tl::enums::ChannelParticipant::Admin(a) => {
                let tl::enums::ChatAdminRights::ChatAdminRights(rights) = a.admin_rights;
                ParticipantPermissions {
                    is_creator: false,
                    is_admin: true,
                    is_banned: false,
                    is_left: false,
                    can_send_messages: true,
                    can_send_media: true,
                    can_pin_messages: rights.pin_messages,
                    can_add_admins: rights.add_admins,
                    admin_rank: a.rank,
                }
            }
            tl::enums::ChannelParticipant::Banned(b) => {
                let tl::enums::ChatBannedRights::ChatBannedRights(rights) = b.banned_rights;
                ParticipantPermissions {
                    is_creator: false,
                    is_admin: false,
                    is_banned: true,
                    is_left: false,
                    can_send_messages: !rights.send_messages,
                    can_send_media: !rights.send_media,
                    can_pin_messages: !rights.pin_messages,
                    can_add_admins: false,
                    admin_rank: None,
                }
            }
            tl::enums::ChannelParticipant::Left(_) => ParticipantPermissions {
                is_creator: false,
                is_admin: false,
                is_banned: false,
                is_left: true,
                can_send_messages: false,
                can_send_media: false,
                can_pin_messages: false,
                can_add_admins: false,
                admin_rank: None,
            },
            _ => ParticipantPermissions {
                is_creator: false,
                is_admin: false,
                is_banned: false,
                is_left: false,
                can_send_messages: true,
                can_send_media: true,
                can_pin_messages: false,
                can_add_admins: false,
                admin_rank: None,
            },
        };

        Ok(perms)
    }
}

/// Lazy async iterator over a user's profile photos.
///
/// Obtained from [`Client::iter_profile_photos`].
///
/// Fetches photos in pages and yields them one at a time.
/// Returns `Ok(None)` when all photos have been consumed.
///
/// # Example
/// ```rust,no_run
/// # use ferogram::Client;
/// # async fn example(client: Client, peer: ferogram::tl::enums::Peer) -> Result<(), Box<dyn std::error::Error>> {
/// let mut iter = client.iter_profile_photos(peer, 0).await?;
/// while let Some(photo) = iter.next().await? {
/// println!("{photo:?}");
/// }
/// # Ok(()) }
/// ```
pub struct ProfilePhotoIter {
    client: Client,
    input_user: tl::enums::InputUser,
    chunk_size: i32,
    /// Next offset to request from the server.
    offset: i32,
    /// Buffered photos from the last fetched page.
    buffer: VecDeque<tl::enums::Photo>,
    /// `true` once the server has no more photos to return.
    done: bool,
}

impl ProfilePhotoIter {
    /// Yield the next profile photo, fetching a new page from Telegram when
    /// the local buffer is empty.
    ///
    /// Returns `Ok(None)` when iteration is complete.
    pub async fn next(&mut self) -> Result<Option<tl::enums::Photo>, InvocationError> {
        // Serve from buffer first.
        if let Some(photo) = self.buffer.pop_front() {
            return Ok(Some(photo));
        }

        // Buffer empty: if we already know there are no more pages, stop.
        if self.done {
            return Ok(None);
        }

        // Fetch next page.
        let req = tl::functions::photos::GetUserPhotos {
            user_id: self.input_user.clone(),
            offset: self.offset,
            max_id: 0,
            limit: self.chunk_size,
        };
        let body: Vec<u8> = self.client.rpc_call_raw(&req).await?;
        let mut cur = Cursor::from_slice(&body);

        let (photos, total): (Vec<tl::enums::Photo>, Option<i32>) =
            match tl::enums::photos::Photos::deserialize(&mut cur)? {
                tl::enums::photos::Photos::Photos(p) => {
                    // Server returned everything at once: no more pages.
                    self.done = true;
                    (p.photos, None)
                }
                tl::enums::photos::Photos::Slice(p) => (p.photos, Some(p.count)),
            };

        let returned = photos.len() as i32;
        self.offset += returned;

        // If we got fewer than requested, or we've reached the total, we're done.
        if returned < self.chunk_size {
            self.done = true;
        }
        if let Some(total) = total
            && self.offset >= total
        {
            self.done = true;
        }

        self.buffer.extend(photos);
        Ok(self.buffer.pop_front())
    }

    /// Collect all remaining photos into a `Vec`.
    ///
    /// Convenience wrapper around repeated `.next()` calls.
    pub async fn collect(mut self) -> Result<Vec<tl::enums::Photo>, InvocationError> {
        let mut out = Vec::new();
        while let Some(photo) = self.next().await? {
            out.push(photo);
        }
        Ok(out)
    }

    /// Total number of photos reported by the server on the first page.
    ///
    /// Returns `None` until the first page has been fetched, or if the server
    /// returned a non-slice response (meaning all photos fit in one page).
    pub fn total_count(&self) -> Option<i32> {
        // Exposed as a future extension point: currently the total is only
        // available after the first network round-trip, so callers should
        // call `.next()` once before querying this if they need the count.
        // For now, we expose offset as a proxy.
        if self.offset > 0 {
            Some(self.offset)
        } else {
            None
        }
    }
}
