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

//! Turning raw Telegram updates into [`E2eEvent`]s.

use ferogram_tl_types as tl;
use tracing::{debug, warn};

use crate::chat::E2eClient;
use crate::error::E2eError;
use crate::tl::DecryptedMessage;
use crate::transport::E2eTransport;

/// Events emitted by the E2E layer after processing a ferogram update.
#[derive(Debug, Clone)]
pub enum E2eEvent {
    /// An incoming secret chat request arrived (User B side).
    ChatRequested { chat_id: i32, from_user_id: i64 },
    /// Our outgoing request was accepted; chat is now established (User A side).
    ChatReady { chat_id: i32 },
    /// The remote side closed the secret chat.
    ChatDiscarded { chat_id: i32 },
    /// An incoming decrypted message (text or media).
    ///
    /// `file` is `Some` when the message carries an encrypted attachment;
    /// the caller can pass it to `E2eClient::download_decrypted_file` together
    /// with the key/iv from `DecryptedMedia` to obtain the plaintext bytes.
    Message {
        chat_id: i32,
        message: Box<DecryptedMessage>,
        file: Option<tl::enums::EncryptedFile>,
    },
    /// Fired `ttl` seconds after a self-destructing message was sent or received.
    /// The caller should delete it from local storage and UI.
    MessageExpired { chat_id: i32, random_id: i64 },
}

impl<T: E2eTransport> E2eClient<T> {
    /// Feed a raw Telegram update into the E2E layer.
    ///
    /// Returns `Some(E2eEvent)` for events the caller needs to act on, and
    /// `None` for updates that are not secret-chat related or that produce no
    /// caller-visible event (service messages are handled internally).
    ///
    /// With `ferogram`, use `ferogram::e2e::E2eClientExt::handle_client_update`
    /// to pass an `Update` straight from the update stream.
    pub async fn handle_update(
        &self,
        update: &tl::enums::Update,
    ) -> Result<Option<E2eEvent>, E2eError> {
        match update {
            tl::enums::Update::Encryption(u) => self.handle_encryption_update(&u.chat).await,
            tl::enums::Update::NewEncryptedMessage(u) => {
                self.handle_new_encrypted_message(&u.message, u.qts).await
            }
            _ => Ok(None),
        }
    }

    async fn handle_encryption_update(
        &self,
        chat: &tl::enums::EncryptedChat,
    ) -> Result<Option<E2eEvent>, E2eError> {
        match chat {
            tl::enums::EncryptedChat::Requested(c) => {
                let from_user_id = c.admin_id;
                let chat_id = c.id;
                self.accept_secret_chat(c).await?;
                Ok(Some(E2eEvent::ChatRequested {
                    chat_id,
                    from_user_id,
                }))
            }
            tl::enums::EncryptedChat::EncryptedChat(c) => {
                let chat_id = c.id;
                self.finish_secret_chat(c).await?;
                Ok(Some(E2eEvent::ChatReady { chat_id }))
            }
            tl::enums::EncryptedChat::Discarded(c) => {
                let chat_id = c.id;
                {
                    let mut store = self.store.lock().await;
                    store.remove(chat_id);
                }
                debug!(chat_id, "secret chat discarded by remote");
                Ok(Some(E2eEvent::ChatDiscarded { chat_id }))
            }
            tl::enums::EncryptedChat::Waiting(_) | tl::enums::EncryptedChat::Empty(_) => Ok(None),
        }
    }

    async fn handle_new_encrypted_message(
        &self,
        message: &tl::enums::EncryptedMessage,
        qts: i32,
    ) -> Result<Option<E2eEvent>, E2eError> {
        let (chat_id, data, enc_file) = match message {
            tl::enums::EncryptedMessage::EncryptedMessage(m) => {
                (m.chat_id, m.bytes.as_slice(), Some(m.file.clone()))
            }
            tl::enums::EncryptedMessage::Service(m) => (m.chat_id, m.bytes.as_slice(), None),
        };

        let result = self.receive(chat_id, data).await?;

        // Acknowledge the server's message queue so it stops re-delivering.
        // qts=0 means we don't have a valid QTS yet; skip to avoid MAX_QTS_INVALID.
        if qts > 0
            && let Err(e) = self.received_queue(qts).await
        {
            warn!(chat_id, qts, "received_queue failed: {e}");
        }

        match result {
            Some(msg) => Ok(Some(E2eEvent::Message {
                chat_id,
                message: Box::new(msg),
                file: enc_file,
            })),
            None => Ok(None),
        }
    }
}
