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

    /// Block or unblock a user or peer. `block: true` blocks, `block: false`
    /// unblocks. `my_stories_from: true` only affects whether they can see
    /// your stories, rather than a full block.
    pub async fn block(
        &self,
        peer: impl Into<PeerRef>,
        block: bool,
        my_stories_from: bool,
    ) -> Result<(), InvocationError> {
        let peer = peer.into().resolve(self).await?;
        let input_peer = self.inner.peer_cache.read().await.peer_to_input(&peer)?;
        if block {
            let req = tl::functions::contacts::Block {
                my_stories_from,
                id: input_peer,
            };
            self.rpc_write(&req).await
        } else {
            let req = tl::functions::contacts::Unblock {
                my_stories_from,
                id: input_peer,
            };
            self.rpc_write(&req).await
        }
    }

    /// Set presence status. `online: true` appears online, `online: false` appears offline.
    pub async fn set_presence(&self, online: bool) -> Result<(), InvocationError> {
        let req = tl::functions::account::UpdateStatus { offline: !online };
        self.rpc_write(&req).await
    }

    /// Terminate a specific session by its `hash` (obtained from [`Self::get_authorizations`]).
    pub async fn terminate_session(&self, hash: i64) -> Result<(), InvocationError> {
        let req = tl::functions::account::ResetAuthorization { hash };
        self.rpc_write(&req).await
    }

    /// Add a user to your contacts. With `add_phone_privacy_exception: true`,
    /// they'll be able to see your phone number even if your privacy
    /// settings would normally hide it from them.
    pub async fn add_contact(
        &self,
        user_id: i64,
        first_name: impl Into<String>,
        last_name: impl Into<String>,
        phone: impl Into<String>,
        add_phone_privacy_exception: bool,
    ) -> Result<(), InvocationError> {
        let hash = self
            .inner
            .peer_cache
            .read()
            .await
            .users
            .get(&user_id)
            .copied()
            .unwrap_or(0);
        let req = tl::functions::contacts::AddContact {
            add_phone_privacy_exception,
            id: tl::enums::InputUser::InputUser(tl::types::InputUser {
                user_id,
                access_hash: hash,
            }),
            first_name: first_name.into(),
            last_name: last_name.into(),
            phone: phone.into(),
            note: None,
        };
        self.rpc_write(&req).await
    }

    /// Add several contacts at once, each given as `(phone, first_name,
    /// last_name)`.
    pub async fn import_contacts(
        &self,
        contacts: &[(&str, &str, &str)],
    ) -> Result<tl::types::contacts::ImportedContacts, InvocationError> {
        use ferogram_tl_types::{Cursor, Deserializable};
        let contacts_tl: Vec<tl::enums::InputContact> = contacts
            .iter()
            .enumerate()
            .map(|(i, (phone, first, last))| {
                tl::enums::InputContact::InputPhoneContact(tl::types::InputPhoneContact {
                    client_id: i as i64,
                    phone: phone.to_string(),
                    first_name: first.to_string(),
                    last_name: last.to_string(),
                    note: None,
                })
            })
            .collect();
        let req = tl::functions::contacts::ImportContacts {
            contacts: contacts_tl,
        };
        let body = self.rpc_call_raw(&req).await?;
        let mut cur = Cursor::from_slice(&body);
        let tl::enums::contacts::ImportedContacts::ImportedContacts(result) =
            tl::enums::contacts::ImportedContacts::deserialize(&mut cur)?;
        self.cache_users_slice(&result.users).await;
        Ok(result)
    }

    /// List the users you've blocked. `offset`/`limit` page through results.
    /// `my_stories_from: true` lists only the story-visibility-only blocks
    /// rather than full blocks.
    pub async fn get_blocked_users(
        &self,
        offset: i32,
        limit: i32,
        my_stories_from: bool,
    ) -> Result<Vec<tl::enums::Peer>, InvocationError> {
        let req = tl::functions::contacts::GetBlocked {
            my_stories_from,
            offset,
            limit,
        };
        let body = self.rpc_call_raw(&req).await?;
        let mut cur = Cursor::from_slice(&body);
        let (blocked, chats, users) = match tl::enums::contacts::Blocked::deserialize(&mut cur)? {
            tl::enums::contacts::Blocked::Blocked(b) => (b.blocked, b.chats, b.users),
            tl::enums::contacts::Blocked::Slice(b) => (b.blocked, b.chats, b.users),
        };
        self.cache_users_slice(&users).await;
        self.cache_chats_slice(&chats).await;
        Ok(blocked
            .into_iter()
            .map(|b| match b {
                tl::enums::PeerBlocked::PeerBlocked(pb) => pb.peer_id,
            })
            .collect())
    }

    /// Search your contacts and the global user directory by name or
    /// username for `query`.
    pub async fn search_contacts(
        &self,
        query: impl Into<String>,
        limit: i32,
    ) -> Result<Vec<tl::enums::Peer>, InvocationError> {
        let req = tl::functions::contacts::Search {
            q: query.into(),
            limit,
            bots: false,
            broadcasts: false,
        };
        let body = self.rpc_call_raw(&req).await?;
        let mut cur = Cursor::from_slice(&body);
        let tl::enums::contacts::Found::Found(found) =
            tl::enums::contacts::Found::deserialize(&mut cur)?;
        self.cache_users_slice(&found.users).await;
        self.cache_chats_slice(&found.chats).await;
        // Combine my_results + results, deduplicated by position
        let mut peers = found.my_results;
        for p in found.results {
            if !peers.contains(&p) {
                peers.push(p);
            }
        }
        Ok(peers)
    }

    /// Delete profile photos by `(id, access_hash, file_reference)`. Returns
    /// the IDs that were actually deleted.
    pub async fn delete_profile_photos(
        &self,
        photo_ids: Vec<(i64, i64, Vec<u8>)>,
    ) -> Result<Vec<i64>, InvocationError> {
        let id: Vec<tl::enums::InputPhoto> = photo_ids
            .into_iter()
            .map(|(id, access_hash, file_reference)| {
                tl::enums::InputPhoto::InputPhoto(tl::types::InputPhoto {
                    id,
                    access_hash,
                    file_reference,
                })
            })
            .collect();
        let req = tl::functions::photos::DeletePhotos { id };
        let body = self.rpc_call_raw(&req).await?;
        let mut cur = Cursor::from_slice(&body);
        // Returns Vector<long> - the IDs that were actually deleted.
        let v = Vec::<i64>::deserialize(&mut cur)?;
        Ok(v)
    }

    /// Update user or chat profile fields via a builder.
    ///
    /// Call `.send().await` to apply. Unset fields are left unchanged.
    ///
    /// # Example
    /// ```rust,no_run
    /// # use ferogram::Client;
    /// # async fn ex(client: Client) {
    /// client.set_profile("me").name("Alice", "").bio("Hello!").send().await.unwrap();
    /// # }
    /// ```
    pub fn set_profile(&self, peer: impl Into<PeerRef>) -> crate::SetProfileBuilder {
        crate::SetProfileBuilder::new(self.clone(), peer.into())
    }

    /// List your active sessions - every device currently logged into this
    /// account.
    pub async fn get_authorizations(
        &self,
    ) -> Result<Vec<tl::types::Authorization>, InvocationError> {
        let req = tl::functions::account::GetAuthorizations {};
        let body = self.rpc_call_raw(&req).await?;
        let mut cur = Cursor::from_slice(&body);
        let tl::enums::account::Authorizations::Authorizations(result) =
            tl::enums::account::Authorizations::deserialize(&mut cur)?;
        Ok(result
            .authorizations
            .into_iter()
            .map(|x| {
                let tl::enums::Authorization::Authorization(a) = x;
                a
            })
            .collect())
    }

    /// Get full info for a user - bio, common chats count, blocked status,
    /// and other fields the basic user object doesn't carry.
    ///
    /// The returned [`crate::types::UserFull`] also bundles the `User`
    /// object from the same response (`.user()` / `.status()`), so you
    /// don't need a follow-up `users.getUsers` call just to read status.
    pub async fn get_user_full(
        &self,
        user_id: i64,
    ) -> Result<crate::types::UserFull, InvocationError> {
        let hash = self
            .inner
            .peer_cache
            .read()
            .await
            .users
            .get(&user_id)
            .copied()
            .unwrap_or(0);
        let req = tl::functions::users::GetFullUser {
            id: tl::enums::InputUser::InputUser(tl::types::InputUser {
                user_id,
                access_hash: hash,
            }),
        };
        let body = self.rpc_call_raw(&req).await?;
        let mut cur = Cursor::from_slice(&body);
        let full = tl::enums::users::UserFull::deserialize(&mut cur)?;
        let tl::enums::users::UserFull::UserFull(ref result) = full;
        self.cache_users_slice(&result.users).await;
        self.cache_chats_slice(&result.chats).await;
        Ok(crate::types::UserFull::from_raw(full))
    }

    /// Retrieve channel or supergroup statistics.
    ///
    /// Auto-dispatches to `stats.getBroadcastStats` for channels and
    /// `stats.getMegagroupStats` for supergroups.
    pub async fn stats(
        &self,
        peer: impl Into<PeerRef>,
    ) -> Result<crate::ChannelStats, InvocationError> {
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
                    "stats: peer must be a channel or supergroup".into(),
                ));
            }
        };
        // Try broadcast stats first; fall back to megagroup stats on error.
        let broadcast_req = tl::functions::stats::GetBroadcastStats {
            dark: false,
            channel: channel.clone(),
        };
        if let Ok(body) = self.rpc_call_raw(&broadcast_req).await {
            let mut cur = Cursor::from_slice(&body);
            if let Ok(s) = tl::enums::stats::BroadcastStats::deserialize(&mut cur) {
                return Ok(crate::ChannelStats::Broadcast(s));
            }
        }
        let meg_req = tl::functions::stats::GetMegagroupStats {
            dark: false,
            channel,
        };
        let body = self.rpc_call_raw(&meg_req).await?;
        let mut cur = Cursor::from_slice(&body);
        Ok(crate::ChannelStats::Megagroup(
            tl::enums::stats::MegagroupStats::deserialize(&mut cur)?,
        ))
    }
}
