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
    /// Permanently delete a channel or supergroup.
    ///
    /// Only the creator can delete a channel. This action is irreversible.
    pub async fn delete_channel(&self, peer: impl Into<PeerRef>) -> Result<(), InvocationError> {
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
                    "delete_channel: peer must be a channel or supergroup".into(),
                ));
            }
        };
        let req = tl::functions::channels::DeleteChannel { channel };
        self.rpc_write(&req).await
    }

    /// Delete a legacy group chat (basic group).
    ///
    /// Only the creator can delete the chat. For channels use [`delete_channel`].
    pub async fn delete_chat(&self, chat_id: i64) -> Result<(), InvocationError> {
        let req = tl::functions::messages::DeleteChat { chat_id };
        self.rpc_write(&req).await
    }

    /// Leave a channel or supergroup.
    ///
    /// For basic groups, kick yourself with [`kick_participant`] or use
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
}
