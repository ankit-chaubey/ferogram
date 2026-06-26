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
#[allow(unused_imports)]
use ferogram_tl_types::{Cursor, Deserializable};

impl Client {
    pub async fn export_invite_link(
        &self,
        peer: impl Into<PeerRef>,
        expire_date: Option<i32>,
        usage_limit: Option<i32>,
        request_needed: bool,
        title: Option<String>,
    ) -> Result<tl::enums::ExportedChatInvite, InvocationError> {
        let peer = peer.into().resolve(self).await?;
        let input_peer = self.inner.peer_cache.read().await.peer_to_input(&peer)?;
        let req = tl::functions::messages::ExportChatInvite {
            legacy_revoke_permanent: false,
            request_needed,
            peer: input_peer,
            expire_date,
            usage_limit,
            title,
            subscription_pricing: None,
        };
        let body = self.rpc_call_raw(&req).await?;
        let mut cur = Cursor::from_slice(&body);
        Ok(tl::enums::ExportedChatInvite::deserialize(&mut cur)?)
    }

    pub async fn revoke_invite_link(
        &self,
        peer: impl Into<PeerRef>,
        link: impl Into<String>,
    ) -> Result<tl::enums::ExportedChatInvite, InvocationError> {
        let peer = peer.into().resolve(self).await?;
        let input_peer = self.inner.peer_cache.read().await.peer_to_input(&peer)?;
        let req = tl::functions::messages::EditExportedChatInvite {
            revoked: true,
            peer: input_peer,
            link: link.into(),
            expire_date: None,
            usage_limit: None,
            request_needed: None,
            title: None,
        };
        let body = self.rpc_call_raw(&req).await?;
        let mut cur = Cursor::from_slice(&body);
        let invite = tl::enums::messages::ExportedChatInvite::deserialize(&mut cur)?;
        let result = match invite {
            tl::enums::messages::ExportedChatInvite::ExportedChatInvite(i) => i,
            _ => {
                return Err(InvocationError::Deserialize(
                    "unexpected ExportedChatInvite variant".into(),
                ));
            }
        };
        Ok(result.invite)
    }

    pub async fn edit_invite_link(
        &self,
        peer: impl Into<PeerRef>,
        link: impl Into<String>,
        expire_date: Option<i32>,
        usage_limit: Option<i32>,
        request_needed: Option<bool>,
        title: Option<String>,
    ) -> Result<tl::enums::ExportedChatInvite, InvocationError> {
        let peer = peer.into().resolve(self).await?;
        let input_peer = self.inner.peer_cache.read().await.peer_to_input(&peer)?;
        let req = tl::functions::messages::EditExportedChatInvite {
            revoked: false,
            peer: input_peer,
            link: link.into(),
            expire_date,
            usage_limit,
            request_needed,
            title,
        };
        let body = self.rpc_call_raw(&req).await?;
        let mut cur = Cursor::from_slice(&body);
        let invite = tl::enums::messages::ExportedChatInvite::deserialize(&mut cur)?;
        let result = match invite {
            tl::enums::messages::ExportedChatInvite::ExportedChatInvite(i) => i,
            _ => {
                return Err(InvocationError::Deserialize(
                    "unexpected ExportedChatInvite variant".into(),
                ));
            }
        };
        Ok(result.invite)
    }

    pub async fn get_invite_links(
        &self,
        peer: impl Into<PeerRef>,
        admin_id: i64,
        revoked: bool,
        limit: i32,
        offset_date: Option<i32>,
        offset_link: Option<String>,
    ) -> Result<Vec<tl::enums::ExportedChatInvite>, InvocationError> {
        let peer = peer.into().resolve(self).await?;
        let input_peer = self.inner.peer_cache.read().await.peer_to_input(&peer)?;
        let admin_hash = self
            .inner
            .peer_cache
            .read()
            .await
            .users
            .get(&admin_id)
            .copied()
            .unwrap_or(0);
        let req = tl::functions::messages::GetExportedChatInvites {
            revoked,
            peer: input_peer,
            admin_id: tl::enums::InputUser::InputUser(tl::types::InputUser {
                user_id: admin_id,
                access_hash: admin_hash,
            }),
            offset_date,
            offset_link,
            limit,
        };
        let body = self.rpc_call_raw(&req).await?;
        let mut cur = Cursor::from_slice(&body);
        let invites = tl::enums::messages::ExportedChatInvites::deserialize(&mut cur)?;
        let tl::enums::messages::ExportedChatInvites::ExportedChatInvites(result) = invites;
        self.cache_users_slice(&result.users).await;
        Ok(result.invites)
    }

    pub async fn delete_invite_link(
        &self,
        peer: impl Into<PeerRef>,
        link: impl Into<String>,
    ) -> Result<(), InvocationError> {
        let peer = peer.into().resolve(self).await?;
        let input_peer = self.inner.peer_cache.read().await.peer_to_input(&peer)?;
        let req = tl::functions::messages::DeleteExportedChatInvite {
            peer: input_peer,
            link: link.into(),
        };
        self.rpc_write(&req).await
    }

    pub async fn delete_revoked_invite_links(
        &self,
        peer: impl Into<PeerRef>,
        admin_id: i64,
    ) -> Result<(), InvocationError> {
        let peer = peer.into().resolve(self).await?;
        let input_peer = self.inner.peer_cache.read().await.peer_to_input(&peer)?;
        let admin_hash = self
            .inner
            .peer_cache
            .read()
            .await
            .users
            .get(&admin_id)
            .copied()
            .unwrap_or(0);
        let req = tl::functions::messages::DeleteRevokedExportedChatInvites {
            peer: input_peer,
            admin_id: tl::enums::InputUser::InputUser(tl::types::InputUser {
                user_id: admin_id,
                access_hash: admin_hash,
            }),
        };
        self.rpc_write(&req).await
    }

    pub async fn join_request(
        &self,
        peer: impl Into<PeerRef>,
        user_id: i64,
        approve: bool,
    ) -> Result<(), InvocationError> {
        let peer = peer.into().resolve(self).await?;
        let input_peer = self.inner.peer_cache.read().await.peer_to_input(&peer)?;
        let user_hash = self
            .inner
            .peer_cache
            .read()
            .await
            .users
            .get(&user_id)
            .copied()
            .unwrap_or(0);
        let req = tl::functions::messages::HideChatJoinRequest {
            approved: approve,
            peer: input_peer,
            user_id: tl::enums::InputUser::InputUser(tl::types::InputUser {
                user_id,
                access_hash: user_hash,
            }),
        };
        self.rpc_write(&req).await
    }

    pub async fn all_join_requests(
        &self,
        peer: impl Into<PeerRef>,
        approve: bool,
        link: Option<String>,
    ) -> Result<(), InvocationError> {
        let peer = peer.into().resolve(self).await?;
        let input_peer = self.inner.peer_cache.read().await.peer_to_input(&peer)?;
        let req = tl::functions::messages::HideAllChatJoinRequests {
            approved: approve,
            peer: input_peer,
            link,
        };
        self.rpc_write(&req).await
    }

    pub async fn get_invite_link_members(
        &self,
        peer: impl Into<PeerRef>,
        link: Option<String>,
        requested: bool,
        limit: i32,
        offset_date: i32,
        offset_user_id: i64,
    ) -> Result<Vec<tl::types::ChatInviteImporter>, InvocationError> {
        let peer = peer.into().resolve(self).await?;
        let input_peer = self.inner.peer_cache.read().await.peer_to_input(&peer)?;
        let offset_hash = self
            .inner
            .peer_cache
            .read()
            .await
            .users
            .get(&offset_user_id)
            .copied()
            .unwrap_or(0);
        let req = tl::functions::messages::GetChatInviteImporters {
            requested,
            subscription_expired: false,
            peer: input_peer,
            link,
            q: None,
            offset_date,
            offset_user: tl::enums::InputUser::InputUser(tl::types::InputUser {
                user_id: offset_user_id,
                access_hash: offset_hash,
            }),
            limit,
        };
        let body = self.rpc_call_raw(&req).await?;
        let mut cur = Cursor::from_slice(&body);
        let tl::enums::messages::ChatInviteImporters::ChatInviteImporters(result) =
            tl::enums::messages::ChatInviteImporters::deserialize(&mut cur)?;
        self.cache_users_slice(&result.users).await;
        Ok(result
            .importers
            .into_iter()
            .map(|x| {
                let tl::enums::ChatInviteImporter::ChatInviteImporter(i) = x;
                i
            })
            .collect())
    }

    pub async fn get_admins_with_invites(
        &self,
        peer: impl Into<PeerRef>,
    ) -> Result<tl::types::messages::ChatAdminsWithInvites, InvocationError> {
        let peer = peer.into().resolve(self).await?;
        let input_peer = self.inner.peer_cache.read().await.peer_to_input(&peer)?;
        let req = tl::functions::messages::GetAdminsWithInvites { peer: input_peer };
        let body = self.rpc_call_raw(&req).await?;
        let mut cur = Cursor::from_slice(&body);
        let tl::enums::messages::ChatAdminsWithInvites::ChatAdminsWithInvites(result) =
            tl::enums::messages::ChatAdminsWithInvites::deserialize(&mut cur)?;
        self.cache_users_slice(&result.users).await;
        Ok(result)
    }
}
