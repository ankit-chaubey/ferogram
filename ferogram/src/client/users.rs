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
    /// Fetch the full contact list of the current user.
    ///
    /// Returns an empty list when the server reports no changes since the last fetch.
    pub async fn get_contacts(&self) -> Result<Vec<tl::enums::User>, InvocationError> {
        let req = tl::functions::contacts::GetContacts { hash: 0 };
        let body: Vec<u8> = self.rpc_call_raw(&req).await?;
        let mut cur = Cursor::from_slice(&body);
        match tl::enums::contacts::Contacts::deserialize(&mut cur)? {
            tl::enums::contacts::Contacts::Contacts(c) => {
                self.cache_users_slice(&c.users).await;
                Ok(c.users)
            }
            tl::enums::contacts::Contacts::NotModified => Ok(vec![]),
        }
    }

    /// Remove one or more users from the contact list.
    pub async fn delete_contacts(&self, user_ids: &[i64]) -> Result<(), InvocationError> {
        if user_ids.is_empty() {
            return Ok(());
        }
        let cache: tokio::sync::RwLockReadGuard<'_, PeerCache> = self.inner.peer_cache.read().await;
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
        let req = tl::functions::contacts::DeleteContacts { id: users };
        self.rpc_write(&req).await
    }

    /// Block a user or peer so they can no longer send you messages.
    pub async fn block_user(&self, peer: impl Into<PeerRef>) -> Result<(), InvocationError> {
        let peer = peer.into().resolve(self).await?;
        let input_peer = self.inner.peer_cache.read().await.peer_to_input(&peer)?;
        let req = tl::functions::contacts::Block {
            my_stories_from: false,
            id: input_peer,
        };
        self.rpc_write(&req).await
    }

    /// Unblock a previously blocked user or peer.
    pub async fn unblock_user(&self, peer: impl Into<PeerRef>) -> Result<(), InvocationError> {
        let peer = peer.into().resolve(self).await?;
        let input_peer = self.inner.peer_cache.read().await.peer_to_input(&peer)?;
        let req = tl::functions::contacts::Unblock {
            my_stories_from: false,
            id: input_peer,
        };
        self.rpc_write(&req).await
    }

    /// Appear online to other users.
    pub async fn set_online(&self) -> Result<(), InvocationError> {
        let req = tl::functions::account::UpdateStatus { offline: false };
        self.rpc_write(&req).await
    }

    /// Appear offline immediately.
    pub async fn set_offline(&self) -> Result<(), InvocationError> {
        let req = tl::functions::account::UpdateStatus { offline: true };
        self.rpc_write(&req).await
    }

    /// Terminate a specific session by its `hash` (obtained from [`get_authorizations`]).
    pub async fn terminate_session(&self, hash: i64) -> Result<(), InvocationError> {
        let req = tl::functions::account::ResetAuthorization { hash };
        self.rpc_write(&req).await
    }
}
