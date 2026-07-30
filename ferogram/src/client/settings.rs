/*
 * Copyright (c) 2026 Ankit Chaubey <ankitchaubey.dev@gmail.com>
 * https://github.com/ankit-chaubey
 *
 * Project: ferogram
 * Website: https://ferogram.dev
 *
 * Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
 * https://www.apache.org/licenses/LICENSE-2.0> or the MIT license
 * <LICENSE-MIT or https://opensource.org/licenses/MIT>, at your option.
 * This file may not be copied, modified, or distributed except according
 * to those terms.
 */

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
