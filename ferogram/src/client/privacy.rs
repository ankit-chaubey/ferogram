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
