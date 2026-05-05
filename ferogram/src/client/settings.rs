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
    /// Mark all unread reactions in a chat as read.
    pub async fn read_reactions(&self, peer: impl Into<PeerRef>) -> Result<(), InvocationError> {
        let peer = peer.into().resolve(self).await?;
        let input_peer = self.inner.peer_cache.read().await.peer_to_input(&peer)?;
        let req = tl::functions::messages::ReadReactions {
            peer: input_peer,
            top_msg_id: None,
            saved_peer_id: None,
        };
        self.rpc_write(&req).await
    }

    /// Clear the recent reactions list shown in the reaction picker.
    pub async fn clear_recent_reactions(&self) -> Result<(), InvocationError> {
        let req = tl::functions::messages::ClearRecentReactions {};
        self.rpc_write(&req).await
    }

    /// Get the approximate number of online members in a group or channel.
    pub async fn get_online_count(&self, peer: impl Into<PeerRef>) -> Result<i32, InvocationError> {
        let peer = peer.into().resolve(self).await?;
        let input_peer = self.inner.peer_cache.read().await.peer_to_input(&peer)?;
        let req = tl::functions::messages::GetOnlines { peer: input_peer };
        let body: Vec<u8> = self.rpc_call_raw(&req).await?;
        let mut cur = Cursor::from_slice(&body);
        let tl::enums::ChatOnlines::ChatOnlines(result) =
            tl::enums::ChatOnlines::deserialize(&mut cur)?;
        Ok(result.onlines)
    }
}
