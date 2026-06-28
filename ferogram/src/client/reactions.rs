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
    /// Tell Telegram you've seen the reactions on these messages, so it
    /// stops marking them as new/unread.
    pub async fn get_reactions(
        &self,
        peer: impl Into<PeerRef>,
        msg_ids: Vec<i32>,
    ) -> Result<(), InvocationError> {
        let peer = peer.into().resolve(self).await?;
        let input_peer = self.inner.peer_cache.read().await.peer_to_input(&peer)?;
        let req = tl::functions::messages::GetMessagesReactions {
            peer: input_peer,
            id: msg_ids,
        };
        self.rpc_write(&req).await
    }

    /// Report (and request removal of) a specific user's reaction on a message.
    pub async fn delete_reaction(
        &self,
        peer: impl Into<PeerRef>,
        msg_id: i32,
        participant: impl Into<PeerRef>,
    ) -> Result<bool, InvocationError> {
        let peer = peer.into().resolve(self).await?;
        let input_peer = self.inner.peer_cache.read().await.peer_to_input(&peer)?;
        let part = participant.into().resolve(self).await?;
        let reaction_peer = self.inner.peer_cache.read().await.peer_to_input(&part)?;
        let req = tl::functions::messages::ReportReaction {
            peer: input_peer,
            id: msg_id,
            reaction_peer,
        };
        let body: Vec<u8> = self.rpc_call_raw(&req).await?;
        Ok(body.len() >= 4 && u32::from_le_bytes(body[..4].try_into().unwrap()) == 0x997275b5)
    }

    /// List who reacted to a message, and with what. Pass `reaction` to
    /// filter to one specific reaction, or `None` for all of them; `offset`
    /// pages through results.
    pub async fn iter_reaction_users(
        &self,
        peer: impl Into<PeerRef>,
        msg_id: i32,
        reaction: Option<tl::enums::Reaction>,
        limit: i32,
        offset: Option<String>,
    ) -> Result<tl::types::messages::MessageReactionsList, InvocationError> {
        let peer = peer.into().resolve(self).await?;
        let input_peer = self.inner.peer_cache.read().await.peer_to_input(&peer)?;
        let req = tl::functions::messages::GetMessageReactionsList {
            peer: input_peer,
            id: msg_id,
            reaction,
            offset,
            limit,
        };
        let body = self.rpc_call_raw(&req).await?;
        let mut cur = Cursor::from_slice(&body);
        let tl::enums::messages::MessageReactionsList::MessageReactionsList(result) =
            tl::enums::messages::MessageReactionsList::deserialize(&mut cur)?;
        self.cache_users_slice(&result.users).await;
        self.cache_chats_slice(&result.chats).await;
        Ok(result)
    }

    /// Send `count` Star reactions on a message - the paid reaction type,
    /// not a regular emoji one.
    pub async fn send_paid_reaction(
        &self,
        peer: impl Into<PeerRef>,
        msg_id: i32,
        count: i32,
    ) -> Result<(), InvocationError> {
        let peer = peer.into().resolve(self).await?;
        let input_peer = self.inner.peer_cache.read().await.peer_to_input(&peer)?;
        let req = tl::functions::messages::SendPaidReaction {
            peer: input_peer,
            msg_id,
            count,
            random_id: random_i64(),
            private: None,
        };
        self.rpc_write(&req).await
    }
}
