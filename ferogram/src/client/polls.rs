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
    /// Send a poll, built with [`crate::poll::PollBuilder`].
    pub async fn send_poll(
        &self,
        peer: impl Into<PeerRef>,
        poll: crate::poll::PollBuilder,
    ) -> Result<(), InvocationError> {
        let peer = peer.into().resolve(self).await?;
        let input_peer = self.inner.peer_cache.read().await.peer_to_input(&peer)?;
        let media = poll.into_input_media();
        let req = tl::functions::messages::SendMedia {
            silent: false,
            background: false,
            clear_draft: false,
            noforwards: false,
            update_stickersets_order: false,
            invert_media: false,
            allow_paid_floodskip: false,
            peer: input_peer,
            reply_to: None,
            media,
            message: String::new(),
            random_id: random_i64(),
            reply_markup: None,
            entities: None,
            schedule_date: None,
            schedule_repeat_period: None,
            send_as: None,
            quick_reply_shortcut: None,
            effect: None,
            allow_paid_stars: None,
            suggested_post: None,
        };
        self.rpc_call_raw(&req).await?;
        Ok(())
    }

    /// Vote on a poll. `options` are the option byte identifiers from the
    /// poll's own answer list, not their text or index - pass more than one
    /// only if the poll allows multiple choice.
    pub async fn send_vote(
        &self,
        peer: impl Into<PeerRef>,
        msg_id: i32,
        options: Vec<Vec<u8>>,
    ) -> Result<(), InvocationError> {
        let peer = peer.into().resolve(self).await?;
        let input_peer = self.inner.peer_cache.read().await.peer_to_input(&peer)?;
        let req = tl::functions::messages::SendVote {
            peer: input_peer,
            msg_id,
            options,
        };
        self.rpc_write(&req).await
    }

    /// Get statistics for a poll message.
    pub async fn poll_results(
        &self,
        peer: impl Into<PeerRef>,
        msg_id: i32,
    ) -> Result<tl::types::stats::PollStats, InvocationError> {
        let peer = peer.into().resolve(self).await?;
        let input_peer = self.inner.peer_cache.read().await.peer_to_input(&peer)?;
        let req = tl::functions::stats::GetPollStats {
            dark: false,
            peer: input_peer,
            msg_id,
        };
        let body = self.rpc_call_raw(&req).await?;
        let mut cur = Cursor::from_slice(&body);
        let tl::enums::stats::PollStats::PollStats(result) =
            tl::enums::stats::PollStats::deserialize(&mut cur)?;
        Ok(result)
    }

    /// List who voted for what on a poll. Pass `option` to filter to one
    /// specific answer, or `None` for everyone; `offset`/`limit` page
    /// through results.
    pub async fn get_poll_votes(
        &self,
        peer: impl Into<PeerRef>,
        msg_id: i32,
        option: Option<Vec<u8>>,
        limit: i32,
        offset: Option<String>,
    ) -> Result<tl::types::messages::VotesList, InvocationError> {
        let peer = peer.into().resolve(self).await?;
        let input_peer = self.inner.peer_cache.read().await.peer_to_input(&peer)?;
        let req = tl::functions::messages::GetPollVotes {
            peer: input_peer,
            id: msg_id,
            option,
            offset,
            limit,
        };
        let body = self.rpc_call_raw(&req).await?;
        let mut cur = Cursor::from_slice(&body);
        let tl::enums::messages::VotesList::VotesList(result) =
            tl::enums::messages::VotesList::deserialize(&mut cur)?;
        self.cache_users_slice(&result.users).await;
        self.cache_chats_slice(&result.chats).await;
        Ok(result)
    }
}
