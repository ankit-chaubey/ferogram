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
use super::random_i64;
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
    /// List a forum group's topics, optionally filtered by `query`. Page
    /// through results with `offset_date`/`offset_id`/`offset_topic` from
    /// the last topic you saw - all `0` to start from the most recent.
    pub async fn get_forum_topics(
        &self,
        peer: impl Into<PeerRef>,
        query: Option<String>,
        limit: i32,
        offset_date: i32,
        offset_id: i32,
        offset_topic: i32,
    ) -> Result<Vec<tl::enums::ForumTopic>, InvocationError> {
        let peer = peer.into().resolve(self).await?;
        let input_peer = self.inner.peer_cache.read().await.peer_to_input(&peer)?;
        let req = tl::functions::messages::GetForumTopics {
            peer: input_peer,
            q: query,
            offset_date,
            offset_id,
            offset_topic,
            limit,
        };
        let body = self.rpc_call_raw(&req).await?;
        let mut cur = Cursor::from_slice(&body);
        let tl::enums::messages::ForumTopics::ForumTopics(result) =
            tl::enums::messages::ForumTopics::deserialize(&mut cur)?;
        self.cache_users_slice(&result.users).await;
        self.cache_chats_slice(&result.chats).await;
        Ok(result.topics)
    }

    /// Look up specific topics in a forum group by their topic IDs.
    pub async fn get_forum_topics_by_id(
        &self,
        peer: impl Into<PeerRef>,
        topic_ids: Vec<i32>,
    ) -> Result<Vec<tl::enums::ForumTopic>, InvocationError> {
        let peer = peer.into().resolve(self).await?;
        let input_peer = self.inner.peer_cache.read().await.peer_to_input(&peer)?;
        let req = tl::functions::messages::GetForumTopicsById {
            peer: input_peer,
            topics: topic_ids,
        };
        let body = self.rpc_call_raw(&req).await?;
        let mut cur = Cursor::from_slice(&body);
        let tl::enums::messages::ForumTopics::ForumTopics(result) =
            tl::enums::messages::ForumTopics::deserialize(&mut cur)?;
        self.cache_users_slice(&result.users).await;
        self.cache_chats_slice(&result.chats).await;
        Ok(result.topics)
    }

    /// Create a new topic in a forum group. `icon_color`/`icon_emoji_id` are
    /// both optional - leave them `None` for Telegram's default icon.
    pub async fn create_forum_topic(
        &self,
        peer: impl Into<PeerRef>,
        title: impl Into<String>,
        icon_color: Option<i32>,
        icon_emoji_id: Option<i64>,
    ) -> Result<(), InvocationError> {
        let peer = peer.into().resolve(self).await?;
        let input_peer = self.inner.peer_cache.read().await.peer_to_input(&peer)?;
        let req = tl::functions::messages::CreateForumTopic {
            title_missing: false,
            peer: input_peer,
            title: title.into(),
            icon_color,
            icon_emoji_id,
            random_id: random_i64(),
            send_as: None,
        };
        self.rpc_write(&req).await
    }

    /// Edit a topic's title, icon, or open/closed and hidden state. Pass
    /// `None` for any field you don't want to change.
    pub async fn edit_forum_topic(
        &self,
        peer: impl Into<PeerRef>,
        topic_id: i32,
        title: Option<String>,
        icon_emoji_id: Option<i64>,
        closed: Option<bool>,
        hidden: Option<bool>,
    ) -> Result<(), InvocationError> {
        let peer = peer.into().resolve(self).await?;
        let input_peer = self.inner.peer_cache.read().await.peer_to_input(&peer)?;
        let req = tl::functions::messages::EditForumTopic {
            peer: input_peer,
            topic_id,
            title,
            icon_emoji_id,
            closed,
            hidden,
        };
        self.rpc_write(&req).await
    }

    /// Delete every message in a topic. Telegram only deletes a batch at a
    /// time, so this keeps calling through until nothing's left.
    pub async fn delete_forum_topic_history(
        &self,
        peer: impl Into<PeerRef>,
        top_msg_id: i32,
    ) -> Result<(), InvocationError> {
        let peer = peer.into().resolve(self).await?;
        let input_peer = self.inner.peer_cache.read().await.peer_to_input(&peer)?;
        loop {
            let req = tl::functions::messages::DeleteTopicHistory {
                peer: input_peer.clone(),
                top_msg_id,
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

    /// Turn Topics mode on or off for a supergroup.
    pub async fn toggle_forum(
        &self,
        peer: impl Into<PeerRef>,
        enabled: bool,
    ) -> Result<(), InvocationError> {
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
                    "toggle_forum: peer must be a supergroup channel".into(),
                ));
            }
        };
        let req = tl::functions::channels::ToggleForum {
            channel,
            enabled,
            tabs: false,
        };
        self.rpc_write(&req).await
    }
}
