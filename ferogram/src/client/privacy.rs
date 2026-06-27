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
    /// Get your privacy rules for one setting - who can see your phone
    /// number, add you to groups, call you, and so on.
    pub async fn get_privacy(
        &self,
        key: tl::enums::InputPrivacyKey,
    ) -> Result<Vec<tl::enums::PrivacyRule>, InvocationError> {
        let req = tl::functions::account::GetPrivacy { key };
        let body = self.rpc_call_raw(&req).await?;
        let mut cur = Cursor::from_slice(&body);
        let tl::enums::account::PrivacyRules::PrivacyRules(result) =
            tl::enums::account::PrivacyRules::deserialize(&mut cur)?;
        self.cache_users_slice(&result.users).await;
        self.cache_chats_slice(&result.chats).await;
        Ok(result.rules)
    }

    /// Replace your privacy rules for one setting. This overwrites the
    /// whole rule set for `key`, not just one rule in it.
    pub async fn set_privacy(
        &self,
        key: tl::enums::InputPrivacyKey,
        rules: Vec<tl::enums::InputPrivacyRule>,
    ) -> Result<Vec<tl::enums::PrivacyRule>, InvocationError> {
        let req = tl::functions::account::SetPrivacy { key, rules };
        let body = self.rpc_call_raw(&req).await?;
        let mut cur = Cursor::from_slice(&body);
        let tl::enums::account::PrivacyRules::PrivacyRules(result) =
            tl::enums::account::PrivacyRules::deserialize(&mut cur)?;
        self.cache_users_slice(&result.users).await;
        self.cache_chats_slice(&result.chats).await;
        Ok(result.rules)
    }

    /// Get notification settings for a chat - muted, sound, and so on.
    pub async fn get_notify_settings(
        &self,
        peer: impl Into<PeerRef>,
    ) -> Result<tl::enums::PeerNotifySettings, InvocationError> {
        let peer = peer.into().resolve(self).await?;
        let input_peer = self.inner.peer_cache.read().await.peer_to_input(&peer)?;
        let req = tl::functions::account::GetNotifySettings {
            peer: tl::enums::InputNotifyPeer::InputNotifyPeer(tl::types::InputNotifyPeer {
                peer: input_peer,
            }),
        };
        let body = self.rpc_call_raw(&req).await?;
        let mut cur = Cursor::from_slice(&body);
        Ok(tl::enums::PeerNotifySettings::deserialize(&mut cur)?)
    }

    /// Change notification settings for a chat.
    pub async fn update_notify_settings(
        &self,
        peer: impl Into<PeerRef>,
        settings: tl::enums::InputPeerNotifySettings,
    ) -> Result<(), InvocationError> {
        let peer = peer.into().resolve(self).await?;
        let input_peer = self.inner.peer_cache.read().await.peer_to_input(&peer)?;
        let req = tl::functions::account::UpdateNotifySettings {
            peer: tl::enums::InputNotifyPeer::InputNotifyPeer(tl::types::InputNotifyPeer {
                peer: input_peer,
            }),
            settings,
        };
        self.rpc_write(&req).await
    }
}
