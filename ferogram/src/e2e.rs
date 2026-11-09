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

//! Telegram secret chats (end-to-end encryption).
//!
//! Enabled by the `e2e` cargo feature. This module re-exports the
//! [`ferogram-e2e`](https://crates.io/crates/ferogram-e2e) crate and plugs
//! [`Client`] into it.
//!
//! ```rust,no_run
//! use ferogram::{Client, e2e::{E2eClient, E2eClientExt, E2eEvent}};
//!
//! # async fn demo(client: Client) -> Result<(), Box<dyn std::error::Error>> {
//! let e2e = E2eClient::new(client.clone());
//! let chat_id = e2e.request_secret_chat("@username").await?;
//!
//! let mut stream = client.stream_updates();
//! while let Some(update) = stream.next().await {
//!     if let Some(E2eEvent::ChatReady { chat_id }) = e2e.handle_client_update(&update).await? {
//!         e2e.send_text(chat_id, "hello", 0).await?;
//!     }
//! }
//! # let _ = chat_id;
//! # Ok(())
//! # }
//! ```

use ferogram_tl_types::{
    RemoteCall,
    enums::{InputPeer, Peer},
};

use crate::{Client, PeerRef, update::Update};

pub use ferogram_e2e::*;

/// Secret chat client driven by a [`Client`].
pub type E2eClient = ferogram_e2e::E2eClient<Client>;

impl E2eTransport for Client {
    async fn invoke<R>(&self, req: &R) -> Result<R::Return, E2eError>
    where
        R: RemoteCall + Send + Sync,
    {
        Client::invoke(self, req)
            .await
            .map_err(|e| E2eError::Transport(e.to_string()))
    }

    async fn invoke_on_dc<R>(&self, dc_id: i32, req: &R) -> Result<R::Return, E2eError>
    where
        R: RemoteCall + Send + Sync,
    {
        Client::invoke_on_dc(self, dc_id, req)
            .await
            .map_err(|e| E2eError::Transport(e.to_string()))
    }
}

/// ferogram-aware helpers for [`E2eClient`].
pub trait E2eClientExt {
    /// Start a secret chat with a user, looking up the access hash for you.
    ///
    /// `peer` is anything [`PeerRef`] accepts: `"@username"`, a phone number, a
    /// Bot-API user id, or a `tl::enums::Peer`. The access hash comes from
    /// ferogram's peer cache, with an RPC only on a cache miss.
    ///
    /// Returns the secret chat id. The chat becomes usable once the peer accepts
    /// and an [`E2eEvent::ChatReady`] arrives.
    ///
    /// Fails if `peer` is not a user (secret chats are one-to-one) or is yourself.
    fn request_secret_chat<P: Into<PeerRef> + Send>(
        &self,
        peer: P,
    ) -> impl std::future::Future<Output = Result<i32, E2eError>> + Send;

    /// Process an [`Update`] from the update stream.
    ///
    /// Secret chat updates arrive as [`Update::Raw`]; everything else returns
    /// `Ok(None)`. See [`ferogram_e2e::E2eClient::handle_update`].
    fn handle_client_update(
        &self,
        update: &Update,
    ) -> impl std::future::Future<Output = Result<Option<E2eEvent>, E2eError>> + Send;
}

impl E2eClientExt for E2eClient {
    async fn request_secret_chat<P: Into<PeerRef> + Send>(&self, peer: P) -> Result<i32, E2eError> {
        let client = self.client();
        let transport = |e: crate::InvocationError| E2eError::Transport(e.to_string());

        let peer = client.resolve(peer).await.map_err(transport)?;
        if !matches!(peer, Peer::User(_)) {
            return Err(E2eError::InvalidState(
                "secret chats are only possible with users".into(),
            ));
        }
        match client
            .resolve_to_input_peer(&peer)
            .await
            .map_err(transport)?
        {
            InputPeer::User(u) => self.request_secret_chat_raw(u.user_id, u.access_hash).await,
            _ => Err(E2eError::InvalidState(
                "cannot open a secret chat with this user (yourself, or no access hash known)"
                    .into(),
            )),
        }
    }

    async fn handle_client_update(&self, update: &Update) -> Result<Option<E2eEvent>, E2eError> {
        match update {
            Update::Raw(raw) => self.handle_update(&raw.inner).await,
            _ => Ok(None),
        }
    }
}
