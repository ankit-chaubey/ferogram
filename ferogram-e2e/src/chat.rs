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

//! High-level secret chat client ([`E2eClient`]): request, accept, send, receive, re-key.

use std::path::PathBuf;
use std::sync::Arc;

use ferogram_tl_types as tl;
use tokio::sync::{Mutex, broadcast};
use tracing::{debug, warn};

use crate::crypto::{
    decrypt_message, encrypt_message, file_decrypt, file_encrypt, file_key_fingerprint,
    key_fingerprint,
};
use crate::dh::{DEFAULT_LAYER, DhCacheEntry, DhContext, compute_shared_from_raw};
use crate::error::E2eError;
use crate::state::{ChatStore, RekeyingState, SecretChatState, TempChatState};
use crate::tl::{
    DecryptedAction, DecryptedMedia, DecryptedMessage, DecryptedMessageLayer, make_random_bytes,
    serialize_layer8_service, serialize_message_layer,
};
use crate::transport::E2eTransport;
use crate::update::E2eEvent;
use crate::upload::upload_encrypted_file;

/// High-level secret chat client.
///
/// Generic over the [`E2eTransport`] that sends RPCs; with `ferogram` enabled,
/// use `ferogram::e2e::E2eClient`, which fixes `T` to `ferogram::Client`.
///
/// Cheap to clone: the chat store, DH cache and event channel are shared behind `Arc`.
#[derive(Clone)]
pub struct E2eClient<T: E2eTransport> {
    pub(crate) client: T,
    pub(crate) store: Arc<Mutex<ChatStore>>,
    dh_cache: Arc<Mutex<Option<DhCacheEntry>>>,
    // Broadcast channel for async events (TTL expiry, etc.)
    event_tx: broadcast::Sender<E2eEvent>,
}

impl<T: E2eTransport> E2eClient<T> {
    /// Create a client that keeps secret chat state in memory only.
    ///
    /// Chats are lost when the process exits; use [`Self::with_persistence`] to keep them.
    pub fn new(client: T) -> Self {
        let (event_tx, _) = broadcast::channel(256);
        Self {
            client,
            store: Arc::new(Mutex::new(ChatStore::new())),
            dh_cache: Arc::new(Mutex::new(None)),
            event_tx,
        }
    }

    /// Create a client that keeps secret chat state next to a session file.
    ///
    /// The store is `<session_name>.e2e`: the session path with its extension
    /// replaced (`mybot.session` becomes `mybot.e2e`). See
    /// [`with_persistence`](Self::with_persistence) for the details.
    pub fn for_session(client: T, session: impl AsRef<std::path::Path>) -> std::io::Result<Self> {
        Self::with_persistence(client, crate::state::store_path_for_session(session))
    }

    /// Create a client that persists secret chat state to the binary file at `path`.
    ///
    /// Previously saved chats are loaded on startup, and every change is written back
    /// atomically. Loading fails with `InvalidData` if the file is corrupt or from an
    /// unsupported version. The file holds each chat's `auth_key` unencrypted (it is
    /// created with mode `0600` on Unix), so protect it accordingly.
    pub fn with_persistence(client: T, path: impl Into<PathBuf>) -> std::io::Result<Self> {
        let store = ChatStore::load(path.into())?;
        let (event_tx, _) = broadcast::channel(256);
        Ok(Self {
            client,
            store: Arc::new(Mutex::new(store)),
            dh_cache: Arc::new(Mutex::new(None)),
            event_tx,
        })
    }

    /// Subscribe to asynchronous events that are not tied to an incoming update
    /// (currently only [`E2eEvent::MessageExpired`] for self-destruct timers).
    pub fn subscribe(&self) -> broadcast::Receiver<E2eEvent> {
        self.event_tx.subscribe()
    }

    /// Spawn a background task that fires [`E2eEvent::MessageExpired`] after `ttl` seconds.
    /// Does nothing if `ttl <= 0`.
    fn schedule_expiry(&self, chat_id: i32, random_id: i64, ttl: i32) {
        if ttl <= 0 {
            return;
        }
        let tx = self.event_tx.clone();
        let secs = ttl as u64;
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_secs(secs)).await;
            let _ = tx.send(E2eEvent::MessageExpired { chat_id, random_id });
        });
    }

    /// Fetch (and cache) the DH config from the server.
    async fn get_dh_config(&self) -> Result<(u32, Vec<u8>), E2eError> {
        let mut cache = self.dh_cache.lock().await;
        let version = cache.as_ref().map(|c| c.version).unwrap_or(0);

        let result = self
            .client
            .invoke(&tl::functions::messages::GetDhConfig {
                version,
                random_length: 0,
            })
            .await?;

        match result {
            tl::enums::messages::DhConfig::DhConfig(c) => {
                let g = c.g as u32;
                let p = c.p.clone();
                *cache = Some(DhCacheEntry {
                    g,
                    p: p.clone(),
                    version: c.version,
                });
                Ok((g, p))
            }
            tl::enums::messages::DhConfig::NotModified(_) => {
                let cached = cache.as_ref().ok_or_else(|| {
                    E2eError::InvalidState("DhConfigNotModified but no cache".into())
                })?;
                Ok((cached.g, cached.p.clone()))
            }
        }
    }

    /// Encrypt and send via `messages.sendEncryptedService`.
    async fn do_send_service(
        &self,
        chat_id: i32,
        access_hash: i64,
        data: Vec<u8>,
    ) -> Result<(), E2eError> {
        let random_id = rand::random::<i64>();
        self.client
            .invoke(&tl::functions::messages::SendEncryptedService {
                peer: tl::enums::InputEncryptedChat::InputEncryptedChat(
                    tl::types::InputEncryptedChat {
                        chat_id,
                        access_hash,
                    },
                ),
                random_id,
                data,
            })
            .await?;
        Ok(())
    }

    /// Encrypt and send via `messages.sendEncrypted`.
    async fn do_send(&self, chat_id: i32, access_hash: i64, data: Vec<u8>) -> Result<(), E2eError> {
        let random_id = rand::random::<i64>();
        self.client
            .invoke(&tl::functions::messages::SendEncrypted {
                silent: false,
                peer: tl::enums::InputEncryptedChat::InputEncryptedChat(
                    tl::types::InputEncryptedChat {
                        chat_id,
                        access_hash,
                    },
                ),
                random_id,
                data,
            })
            .await?;
        Ok(())
    }

    /// Re-send a buffered encrypted blob verbatim (for Resend handling).
    async fn do_resend(
        &self,
        chat_id: i32,
        access_hash: i64,
        data: Vec<u8>,
    ) -> Result<(), E2eError> {
        let random_id = rand::random::<i64>();
        self.client
            .invoke(&tl::functions::messages::SendEncrypted {
                silent: false,
                peer: tl::enums::InputEncryptedChat::InputEncryptedChat(
                    tl::types::InputEncryptedChat {
                        chat_id,
                        access_hash,
                    },
                ),
                random_id,
                data,
            })
            .await?;
        Ok(())
    }

    /// Send the layer notification message immediately after chat establishment.
    async fn notify_layer_inner(
        &self,
        chat_id: i32,
        access_hash: i64,
        auth_key: [u8; 256],
        admin: bool,
    ) -> Result<(), E2eError> {
        let random_id = rand::random::<i64>();
        let plaintext = serialize_layer8_service(
            random_id,
            &DecryptedAction::NotifyLayer {
                layer: DEFAULT_LAYER,
            },
        )?;
        let blob = encrypt_message(&auth_key, &plaintext, admin)?;
        self.do_send_service(chat_id, access_hash, blob).await
    }

    /// The underlying transport (client) this E2E client sends through.
    pub fn client(&self) -> &T {
        &self.client
    }

    /// Initiate a secret chat with `user_id` / `access_hash`.
    ///
    /// This is the low-level form for clients that already hold the access hash.
    /// With `ferogram`, prefer `E2eClientExt::request_secret_chat`, which looks it up.
    pub async fn request_secret_chat_raw(
        &self,
        user_id: i64,
        access_hash: i64,
    ) -> Result<i32, E2eError> {
        let (g, p_bytes) = self.get_dh_config().await?;
        let ctx = DhContext::generate(g, &p_bytes)?;

        let random_id = rand::random::<i32>();
        let result = self
            .client
            .invoke(&tl::functions::messages::RequestEncryption {
                user_id: tl::enums::InputUser::InputUser(tl::types::InputUser {
                    user_id,
                    access_hash,
                }),
                random_id,
                g_a: ctx.public_value.clone(),
            })
            .await?;

        let chat_id = match result {
            tl::enums::EncryptedChat::Waiting(w) => w.id,
            other => {
                return Err(E2eError::InvalidState(format!(
                    "unexpected chat state after requestEncryption: {:?}",
                    other
                )));
            }
        };

        let mut store = self.store.lock().await;
        store.insert_pending(TempChatState {
            id: chat_id,
            private_key_a: ctx.private_key,
        });

        debug!(chat_id, "secret chat requested");
        Ok(chat_id)
    }

    /// Accept an incoming secret chat request (User B side).
    pub async fn accept_secret_chat(
        &self,
        chat: &tl::types::EncryptedChatRequested,
    ) -> Result<(), E2eError> {
        let (g, p_bytes) = self.get_dh_config().await?;
        let ctx = DhContext::generate(g, &p_bytes)?;
        let auth_key = ctx.compute_shared_secret(&chat.g_a)?;
        let fp = key_fingerprint(&auth_key);

        self.client
            .invoke(&tl::functions::messages::AcceptEncryption {
                peer: tl::enums::InputEncryptedChat::InputEncryptedChat(
                    tl::types::InputEncryptedChat {
                        chat_id: chat.id,
                        access_hash: chat.access_hash,
                    },
                ),
                g_b: ctx.public_value.clone(),
                key_fingerprint: fp,
            })
            .await?;

        let state = SecretChatState::new(chat.id, chat.access_hash, auth_key, false, chat.admin_id);

        {
            let mut store = self.store.lock().await;
            store.insert(state)?;
        }

        self.notify_layer_inner(chat.id, chat.access_hash, auth_key, false)
            .await?;

        debug!(chat_id = chat.id, "secret chat accepted");
        Ok(())
    }

    /// Complete chat creation on the initiator side (User A) after peer accepts.
    pub async fn finish_secret_chat(
        &self,
        chat: &tl::types::EncryptedChat,
    ) -> Result<(), E2eError> {
        let (_g, p_bytes) = self.get_dh_config().await?;

        let temp = {
            let mut store = self.store.lock().await;
            store
                .take_pending(chat.id)
                .ok_or(E2eError::ChatNotFound(chat.id))?
        };

        let auth_key = compute_shared_from_raw(&temp.private_key_a, &chat.g_a_or_b, &p_bytes)?;

        let computed_fp = key_fingerprint(&auth_key);
        if computed_fp != chat.key_fingerprint {
            self.client
                .invoke(&tl::functions::messages::DiscardEncryption {
                    delete_history: false,
                    chat_id: chat.id,
                })
                .await?;
            return Err(E2eError::KeyFingerprintMismatch);
        }

        let state = SecretChatState::new(
            chat.id,
            chat.access_hash,
            auth_key,
            true,
            chat.participant_id,
        );

        {
            let mut store = self.store.lock().await;
            store.insert(state)?;
        }

        self.notify_layer_inner(chat.id, chat.access_hash, auth_key, true)
            .await?;

        debug!(chat_id = chat.id, "secret chat creation finished");
        Ok(())
    }

    /// Send a text message to an established secret chat.
    pub async fn send_text(&self, chat_id: i32, text: &str, ttl: i32) -> Result<(), E2eError> {
        if text.len() > 512 * 1024 {
            return Err(E2eError::MessageTooLarge);
        }

        let random_id = rand::random::<i64>();
        let (blob, access_hash) = {
            let mut store = self.store.lock().await;
            let state = store.get_mut(chat_id)?;

            let wire_seq_no = state.wire_out_seq_no();

            let msg = DecryptedMessage::Text {
                random_id,
                ttl,
                message: text.to_owned(),
                media: DecryptedMedia::Empty,
            };

            let layer_msg = DecryptedMessageLayer {
                random_bytes: make_random_bytes(),
                layer: DEFAULT_LAYER,
                in_seq_no: state.wire_in_seq_no(),
                out_seq_no: wire_seq_no,
                message: msg,
            };

            let plaintext = serialize_message_layer(&layer_msg)?;
            if plaintext.len() > 512 * 1024 {
                return Err(E2eError::MessageTooLarge);
            }

            let blob = encrypt_message(&state.auth_key, &plaintext, state.admin)?;
            state.push_sent(wire_seq_no, blob.clone());
            state.out_seq_no += 1;
            state.ttr -= 1;
            let access_hash = state.access_hash;
            store.save();
            (blob, access_hash)
        };

        self.maybe_rekey(chat_id).await?;
        self.do_send(chat_id, access_hash, blob).await?;
        self.schedule_expiry(chat_id, random_id, ttl);
        Ok(())
    }

    /// Encrypt and send a file as a secret chat document message.
    ///
    /// `file_bytes` is the raw plaintext content.
    /// `mime_type` e.g. "image/jpeg", "application/pdf".
    /// `file_name` is the suggested filename shown to the recipient.
    /// `caption` is optional (empty string = no caption).
    /// `ttl` is self-destruct seconds (0 = no TTL).
    ///
    /// Returns the `random_id` of the sent message.
    pub async fn send_file(
        &self,
        chat_id: i32,
        file_bytes: &[u8],
        mime_type: &str,
        file_name: &str,
        caption: &str,
        ttl: i32,
    ) -> Result<i64, E2eError> {
        let (file_key, file_iv, ciphertext) = file_encrypt(file_bytes);
        let fp = file_key_fingerprint(&file_key, &file_iv);

        let input_file = upload_encrypted_file(&self.client, &ciphertext, fp).await?;

        let random_id = rand::random::<i64>();

        let media = DecryptedMedia::Document {
            thumb: Vec::new(),
            thumb_w: 0,
            thumb_h: 0,
            mime_type: mime_type.to_owned(),
            size: file_bytes.len() as i64,
            key: file_key.to_vec(),
            iv: file_iv.to_vec(),
            attributes: vec![crate::tl::DocumentAttribute::FileName {
                file_name: file_name.to_owned(),
            }],
            caption: caption.to_owned(),
        };

        let msg = DecryptedMessage::Text {
            random_id,
            ttl,
            message: String::new(),
            media,
        };

        let (blob, access_hash) = {
            let mut store = self.store.lock().await;
            let state = store.get_mut(chat_id)?;

            let wire_seq_no = state.wire_out_seq_no();

            let layer_msg = DecryptedMessageLayer {
                random_bytes: make_random_bytes(),
                layer: DEFAULT_LAYER,
                in_seq_no: state.wire_in_seq_no(),
                out_seq_no: wire_seq_no,
                message: msg,
            };

            let plaintext = serialize_message_layer(&layer_msg)?;
            if plaintext.len() > 512 * 1024 {
                return Err(E2eError::MessageTooLarge);
            }

            let blob = encrypt_message(&state.auth_key, &plaintext, state.admin)?;
            state.push_sent(wire_seq_no, blob.clone());
            state.out_seq_no += 1;
            state.ttr -= 1;
            let access_hash = state.access_hash;
            store.save();
            (blob, access_hash)
        };

        self.maybe_rekey(chat_id).await?;

        self.client
            .invoke(&tl::functions::messages::SendEncryptedFile {
                silent: false,
                peer: tl::enums::InputEncryptedChat::InputEncryptedChat(
                    tl::types::InputEncryptedChat {
                        chat_id,
                        access_hash,
                    },
                ),
                random_id,
                data: blob,
                file: input_file,
            })
            .await?;

        self.schedule_expiry(chat_id, random_id, ttl);
        Ok(random_id)
    }

    /// Send a photo to a secret chat.
    ///
    /// Uses `DecryptedMedia::Photo` (constructor 0x32798a8c), which is what
    /// official clients send for inline photos. `w` and `h` are the image
    /// dimensions in pixels; pass 0 if unknown. `ttl` is self-destruct seconds
    /// (0 = no TTL). Returns the `random_id` of the sent message.
    pub async fn send_photo(
        &self,
        chat_id: i32,
        photo_bytes: &[u8],
        w: i32,
        h: i32,
        caption: &str,
        ttl: i32,
    ) -> Result<i64, E2eError> {
        let (file_key, file_iv, ciphertext) = file_encrypt(photo_bytes);
        let fp = file_key_fingerprint(&file_key, &file_iv);

        let input_file = upload_encrypted_file(&self.client, &ciphertext, fp).await?;

        let random_id = rand::random::<i64>();

        let media = DecryptedMedia::Photo {
            thumb: Vec::new(),
            thumb_w: 0,
            thumb_h: 0,
            w,
            h,
            size: photo_bytes.len() as i32,
            key: file_key.to_vec(),
            iv: file_iv.to_vec(),
            caption: caption.to_owned(),
        };

        let msg = DecryptedMessage::Text {
            random_id,
            ttl,
            message: String::new(),
            media,
        };

        let (blob, access_hash) = {
            let mut store = self.store.lock().await;
            let state = store.get_mut(chat_id)?;

            let wire_seq_no = state.wire_out_seq_no();

            let layer_msg = DecryptedMessageLayer {
                random_bytes: make_random_bytes(),
                layer: DEFAULT_LAYER,
                in_seq_no: state.wire_in_seq_no(),
                out_seq_no: wire_seq_no,
                message: msg,
            };

            let plaintext = serialize_message_layer(&layer_msg)?;
            if plaintext.len() > 512 * 1024 {
                return Err(E2eError::MessageTooLarge);
            }

            let blob = encrypt_message(&state.auth_key, &plaintext, state.admin)?;
            state.push_sent(wire_seq_no, blob.clone());
            state.out_seq_no += 1;
            state.ttr -= 1;
            let access_hash = state.access_hash;
            store.save();
            (blob, access_hash)
        };

        self.maybe_rekey(chat_id).await?;

        self.client
            .invoke(&tl::functions::messages::SendEncryptedFile {
                silent: false,
                peer: tl::enums::InputEncryptedChat::InputEncryptedChat(
                    tl::types::InputEncryptedChat {
                        chat_id,
                        access_hash,
                    },
                ),
                random_id,
                data: blob,
                file: input_file,
            })
            .await?;

        self.schedule_expiry(chat_id, random_id, ttl);
        Ok(random_id)
    }

    /// Download and decrypt an encrypted attachment received in a secret chat.
    ///
    /// `enc_file` comes from `E2eEvent::Message { file: Some(enc_file), .. }`.
    /// `key` and `iv` come from the `DecryptedMedia` inside the same message.
    /// `size` is the original plaintext length from the `DecryptedMedia` size field.
    ///
    /// Returns the decrypted plaintext bytes ready for forwarding or processing.
    pub async fn download_decrypted_file(
        &self,
        enc_file: &tl::enums::EncryptedFile,
        key: &[u8],
        iv: &[u8],
        size: usize,
    ) -> Result<Vec<u8>, E2eError> {
        let (id, access_hash, dc_id) = match enc_file {
            tl::enums::EncryptedFile::EncryptedFile(f) => (f.id, f.access_hash, f.dc_id),
            tl::enums::EncryptedFile::Empty => {
                return Err(E2eError::Transport("encrypted file is empty".into()));
            }
        };

        let location = tl::enums::InputFileLocation::InputEncryptedFileLocation(
            tl::types::InputEncryptedFileLocation { id, access_hash },
        );

        let ciphertext = self.download_encrypted_bytes(location, dc_id).await?;

        file_decrypt(&ciphertext, key, iv, size)
    }

    /// Fetch the raw (still encrypted) bytes of a secret-chat file from `dc_id`.
    ///
    /// Secret-chat files live on the DC named in `EncryptedFile::dc_id`, so the
    /// chunks are pulled with `upload.getFile` through [`E2eTransport::invoke_on_dc`].
    async fn download_encrypted_bytes(
        &self,
        location: tl::enums::InputFileLocation,
        dc_id: i32,
    ) -> Result<Vec<u8>, E2eError> {
        // Offset must be a multiple of 4 KiB and limit a divisor of 1 MiB.
        const CHUNK: i32 = 512 * 1024;
        let mut bytes = Vec::new();
        let mut offset: i64 = 0;
        loop {
            let req = tl::functions::upload::GetFile {
                precise: true,
                cdn_supported: false,
                location: location.clone(),
                offset,
                limit: CHUNK,
            };
            match self.client.invoke_on_dc(dc_id, &req).await? {
                tl::enums::upload::File::File(f) => {
                    let done = (f.bytes.len() as i32) < CHUNK;
                    bytes.extend_from_slice(&f.bytes);
                    if done {
                        break;
                    }
                    offset += CHUNK as i64;
                }
                tl::enums::upload::File::CdnRedirect(_) => {
                    return Err(E2eError::Transport(
                        "unexpected CDN redirect for encrypted file".into(),
                    ));
                }
            }
        }
        Ok(bytes)
    }

    /// Send a service message to an established secret chat.
    pub async fn send_service(
        &self,
        chat_id: i32,
        action: DecryptedAction,
    ) -> Result<(), E2eError> {
        let random_id = rand::random::<i64>();
        let (blob, access_hash) = {
            let mut store = self.store.lock().await;
            let state = store.get_mut(chat_id)?;

            let wire_seq_no = state.wire_out_seq_no();

            let msg = DecryptedMessage::Service { random_id, action };

            let layer_msg = DecryptedMessageLayer {
                random_bytes: make_random_bytes(),
                layer: DEFAULT_LAYER,
                in_seq_no: state.wire_in_seq_no(),
                out_seq_no: wire_seq_no,
                message: msg,
            };

            let plaintext = serialize_message_layer(&layer_msg)?;
            let blob = encrypt_message(&state.auth_key, &plaintext, state.admin)?;
            state.push_sent(wire_seq_no, blob.clone());
            state.out_seq_no += 1;
            state.ttr -= 1;
            let access_hash = state.access_hash;
            store.save();
            (blob, access_hash)
        };

        self.do_send_service(chat_id, access_hash, blob).await
    }

    /// Discard (close) a secret chat.
    pub async fn discard(&self, chat_id: i32, delete_history: bool) -> Result<(), E2eError> {
        self.client
            .invoke(&tl::functions::messages::DiscardEncryption {
                delete_history,
                chat_id,
            })
            .await?;

        let mut store = self.store.lock().await;
        store.remove(chat_id);

        debug!(chat_id, "secret chat discarded");
        Ok(())
    }

    /// Return the 64-bit key fingerprint of a chat's `auth_key`.
    ///
    /// A sanity check for the key exchange, not a user-facing verification code.
    pub fn fingerprint(&self, chat_id: i32) -> Result<i64, E2eError> {
        let store = self
            .store
            .try_lock()
            .map_err(|_| E2eError::InvalidState("store lock contention".into()))?;
        let state = store.get(chat_id)?;
        Ok(key_fingerprint(&state.auth_key))
    }

    /// Return 4 emoji derived from the chat key, for comparing out of band.
    ///
    /// Both sides of the chat see the same 4 emoji (see
    /// [`crate::crypto::emoji_fingerprint_indices`]); display them side by side and
    /// let the users compare. A mismatch means the key exchange was tampered with.
    pub fn emoji_fingerprint(&self, chat_id: i32) -> Result<[&'static str; 4], E2eError> {
        let store = self
            .store
            .try_lock()
            .map_err(|_| E2eError::InvalidState("store lock contention".into()))?;
        let state = store.get(chat_id)?;
        let indices = crate::crypto::emoji_fingerprint_indices(&state.auth_key);
        Ok([
            crate::crypto::EMOJI_FINGERPRINT_TABLE[indices[0]],
            crate::crypto::EMOJI_FINGERPRINT_TABLE[indices[1]],
            crate::crypto::EMOJI_FINGERPRINT_TABLE[indices[2]],
            crate::crypto::EMOJI_FINGERPRINT_TABLE[indices[3]],
        ])
    }

    /// Send a typing indicator to a secret chat.
    pub async fn set_encrypted_typing(&self, chat_id: i32, typing: bool) -> Result<(), E2eError> {
        let access_hash = {
            let store = self.store.lock().await;
            store.get(chat_id)?.access_hash
        };
        self.client
            .invoke(&tl::functions::messages::SetEncryptedTyping {
                peer: tl::enums::InputEncryptedChat::InputEncryptedChat(
                    tl::types::InputEncryptedChat {
                        chat_id,
                        access_hash,
                    },
                ),
                typing,
            })
            .await?;
        Ok(())
    }

    /// Mark secret chat messages as read up to `max_date` (Unix timestamp).
    pub async fn read_encrypted_history(
        &self,
        chat_id: i32,
        max_date: i32,
    ) -> Result<(), E2eError> {
        let access_hash = {
            let store = self.store.lock().await;
            store.get(chat_id)?.access_hash
        };
        self.client
            .invoke(&tl::functions::messages::ReadEncryptedHistory {
                peer: tl::enums::InputEncryptedChat::InputEncryptedChat(
                    tl::types::InputEncryptedChat {
                        chat_id,
                        access_hash,
                    },
                ),
                max_date,
            })
            .await?;
        Ok(())
    }

    /// Acknowledge the secret chat message queue up to `max_qts`.
    pub async fn received_queue(&self, max_qts: i32) -> Result<(), E2eError> {
        self.client
            .invoke(&tl::functions::messages::ReceivedQueue { max_qts })
            .await?;
        Ok(())
    }

    /// Initiate a re-key exchange for a chat.
    pub async fn initiate_rekey(&self, chat_id: i32) -> Result<(), E2eError> {
        let (g, p_bytes) = self.get_dh_config().await?;
        let ctx = DhContext::generate(g, &p_bytes)?;

        let exchange_id = rand::random::<i64>();
        let action = DecryptedAction::RequestKey {
            exchange_id,
            g_a: ctx.public_value.clone(),
        };

        {
            let mut store = self.store.lock().await;
            let state = store.get_mut(chat_id)?;
            match state.rekeying_state {
                RekeyingState::Idle => {}
                _ => return Err(E2eError::RekeyInProgress),
            }
            state
                .temp_rekey_map
                .insert(exchange_id, ctx.private_key.to_vec());
            state.rekeying_state = RekeyingState::Requested { exchange_id };
        }

        self.send_service(chat_id, action).await
    }

    /// Handle an incoming `RequestKey` rekey action.
    pub async fn handle_rekey_request(
        &self,
        chat_id: i32,
        action: &DecryptedAction,
    ) -> Result<(), E2eError> {
        let DecryptedAction::RequestKey { exchange_id, g_a } = action else {
            return Ok(());
        };
        let exchange_id = *exchange_id;

        let (g, p_bytes) = self.get_dh_config().await?;
        let ctx = DhContext::generate(g, &p_bytes)?;
        let auth_key = ctx.compute_shared_secret(g_a)?;
        let fp = key_fingerprint(&auth_key);

        let should_ignore = {
            let store = self.store.lock().await;
            let state = store.get(chat_id)?;
            matches!(state.rekeying_state, RekeyingState::Requested { exchange_id: my_id } if my_id > exchange_id)
        };

        if should_ignore {
            return Ok(());
        }

        let reply_action = DecryptedAction::AcceptKey {
            exchange_id,
            g_b: ctx.public_value.clone(),
            key_fingerprint: fp,
        };

        {
            let mut store = self.store.lock().await;
            let state = store.get_mut(chat_id)?;
            state.temp_rekey_map.insert(exchange_id, auth_key.to_vec());
            state.rekeying_state = RekeyingState::Accepted { exchange_id };
        }

        self.send_service(chat_id, reply_action).await
    }

    /// Handle an incoming `AcceptKey` rekey action.
    pub async fn handle_rekey_accept(
        &self,
        chat_id: i32,
        action: &DecryptedAction,
    ) -> Result<(), E2eError> {
        let DecryptedAction::AcceptKey {
            exchange_id,
            g_b,
            key_fingerprint: received_fp,
        } = action
        else {
            return Ok(());
        };
        let exchange_id = *exchange_id;

        let private_key_bytes = {
            let store = self.store.lock().await;
            let state = store.get(chat_id)?;
            let RekeyingState::Requested { exchange_id: my_id } = state.rekeying_state else {
                return Ok(());
            };
            if my_id != exchange_id {
                return Ok(());
            }
            state
                .temp_rekey_map
                .get(&exchange_id)
                .cloned()
                .ok_or(E2eError::InvalidState("no temp rekey entry".into()))?
        };

        let (_, p) = self.get_dh_config().await?;

        let mut key_arr = [0u8; 256];
        if private_key_bytes.len() != 256 {
            return Err(E2eError::InvalidState("bad private key length".into()));
        }
        key_arr.copy_from_slice(&private_key_bytes);

        let new_auth_key = compute_shared_from_raw(&key_arr, g_b, &p)?;
        let computed_fp = key_fingerprint(&new_auth_key);

        if computed_fp != *received_fp {
            let abort = DecryptedAction::AbortKey { exchange_id };
            {
                let mut store = self.store.lock().await;
                let state = store.get_mut(chat_id)?;
                state.temp_rekey_map.remove(&exchange_id);
                state.rekeying_state = RekeyingState::Idle;
            }
            self.send_service(chat_id, abort).await?;
            return Err(E2eError::SecurityError("rekey fingerprint mismatch".into()));
        }

        let commit = DecryptedAction::CommitKey {
            exchange_id,
            key_fingerprint: computed_fp,
        };

        {
            let mut store = self.store.lock().await;
            let state = store.get_mut(chat_id)?;
            state.rotate_key(new_auth_key);
            state.temp_rekey_map.remove(&exchange_id);
            state.rekeying_state = RekeyingState::Idle;
            store.save();
        }

        self.send_service(chat_id, commit).await
    }

    /// Handle an incoming `CommitKey` rekey action.
    pub async fn handle_rekey_commit(
        &self,
        chat_id: i32,
        action: &DecryptedAction,
    ) -> Result<(), E2eError> {
        let DecryptedAction::CommitKey {
            exchange_id,
            key_fingerprint: received_fp,
        } = action
        else {
            return Ok(());
        };
        let exchange_id = *exchange_id;

        let (new_auth_key, fp_ok) = {
            let store = self.store.lock().await;
            let state = store.get(chat_id)?;
            let RekeyingState::Accepted { exchange_id: my_id } = state.rekeying_state else {
                return Ok(());
            };
            if my_id != exchange_id {
                return Ok(());
            }
            let key_bytes = state
                .temp_rekey_map
                .get(&exchange_id)
                .cloned()
                .ok_or(E2eError::InvalidState("no temp rekey entry".into()))?;
            if key_bytes.len() != 256 {
                return Err(E2eError::InvalidState("bad rekey auth_key length".into()));
            }
            let mut arr = [0u8; 256];
            arr.copy_from_slice(&key_bytes);
            let computed_fp = key_fingerprint(&arr);
            (arr, computed_fp == *received_fp)
        };

        if !fp_ok {
            let abort = DecryptedAction::AbortKey { exchange_id };
            {
                let mut store = self.store.lock().await;
                let state = store.get_mut(chat_id)?;
                state.temp_rekey_map.remove(&exchange_id);
                state.rekeying_state = RekeyingState::Idle;
            }
            self.send_service(chat_id, abort).await?;
            return Err(E2eError::SecurityError(
                "commit fingerprint mismatch".into(),
            ));
        }

        {
            let mut store = self.store.lock().await;
            let state = store.get_mut(chat_id)?;
            state.rotate_key(new_auth_key);
            state.temp_rekey_map.remove(&exchange_id);
            state.rekeying_state = RekeyingState::Idle;
            store.save();
        }

        self.send_service(chat_id, DecryptedAction::Noop).await
    }

    /// Check if a re-key should be initiated and do so if needed.
    pub(crate) async fn maybe_rekey(&self, chat_id: i32) -> Result<(), E2eError> {
        let should_rekey = {
            let store = self.store.lock().await;
            let state = match store.get(chat_id) {
                Ok(s) => s,
                Err(_) => return Ok(()),
            };
            let idle = matches!(state.rekeying_state, RekeyingState::Idle);
            let ttr_expired = state.ttr <= 0;
            let time_expired = state.secs_since_update() > 7 * 24 * 3600;
            idle && (ttr_expired || time_expired)
        };

        if should_rekey {
            self.initiate_rekey(chat_id).await?;
        }
        Ok(())
    }

    /// Decrypt an incoming encrypted message blob and return the inner message.
    ///
    /// Validates the incoming seq_no and sends a Resend action if a gap is
    /// detected. Handles service actions internally; returns `None` for them.
    pub async fn receive(
        &self,
        chat_id: i32,
        data: &[u8],
    ) -> Result<Option<DecryptedMessage>, E2eError> {
        let (auth_key, admin) = {
            let store = self.store.lock().await;
            let state = store.get(chat_id)?;
            (state.auth_key, state.admin)
        };

        let plaintext = decrypt_message(&auth_key, data, admin)?;
        let layer_msg = crate::tl::deserialize_message_layer(&plaintext)?;

        let resend = {
            let mut store = self.store.lock().await;
            let state = store.get_mut(chat_id)?;

            let expected = state.wire_in_seq_no();
            let got = layer_msg.out_seq_no;

            let resend = if got > expected {
                warn!(
                    chat_id,
                    expected, got, "secret chat seq_no gap; requesting resend"
                );
                Some(DecryptedAction::Resend {
                    start_seq_no: expected,
                    end_seq_no: got - 2,
                })
            } else {
                None
            };

            state.in_seq_no += 1;
            state.ttr -= 1;
            if layer_msg.layer > 0 {
                state.layer = layer_msg.layer;
            }
            store.save();
            resend
        };

        if let Some(resend_action) = resend
            && let Err(e) = self.send_service(chat_id, resend_action).await
        {
            warn!(chat_id, "failed to send Resend action: {e}");
        }

        self.maybe_rekey(chat_id).await?;

        match &layer_msg.message {
            DecryptedMessage::Service { action, .. } => {
                self.dispatch_action(chat_id, action.clone()).await?;
                Ok(None)
            }
            DecryptedMessage::Unknown { ctor, .. } => {
                debug!(chat_id, ctor, "ignoring unknown decrypted message ctor");
                Ok(None)
            }
            msg => {
                // Schedule TTL expiry on the recipient side if the message has one.
                if let DecryptedMessage::Text { random_id, ttl, .. } = msg {
                    self.schedule_expiry(chat_id, *random_id, *ttl);
                }
                Ok(Some(msg.clone()))
            }
        }
    }

    async fn dispatch_action(&self, chat_id: i32, action: DecryptedAction) -> Result<(), E2eError> {
        match &action {
            DecryptedAction::NotifyLayer { layer } => {
                let mut store = self.store.lock().await;
                if let Ok(state) = store.get_mut(chat_id) {
                    state.layer = *layer;
                    if *layer >= 73 {
                        state.mtproto = 2;
                    }
                }
            }
            DecryptedAction::SetTtl { ttl_seconds } => {
                let mut store = self.store.lock().await;
                if let Ok(state) = store.get_mut(chat_id) {
                    state.ttl = *ttl_seconds;
                }
            }
            DecryptedAction::RequestKey { .. } => {
                self.handle_rekey_request(chat_id, &action).await?;
            }
            DecryptedAction::AcceptKey { .. } => {
                self.handle_rekey_accept(chat_id, &action).await?;
            }
            DecryptedAction::CommitKey { .. } => {
                self.handle_rekey_commit(chat_id, &action).await?;
            }
            DecryptedAction::AbortKey { .. } => {
                let mut store = self.store.lock().await;
                if let Ok(state) = store.get_mut(chat_id) {
                    state.rekeying_state = RekeyingState::Idle;
                    state.temp_rekey_map.clear();
                }
            }
            DecryptedAction::Resend {
                start_seq_no,
                end_seq_no,
            } => {
                self.handle_resend(chat_id, *start_seq_no, *end_seq_no)
                    .await;
            }
            DecryptedAction::ReadMessages { .. }
            | DecryptedAction::DeleteMessages { .. }
            | DecryptedAction::ScreenshotMessages { .. }
            | DecryptedAction::FlushHistory
            | DecryptedAction::Typing { .. }
            | DecryptedAction::Noop
            | DecryptedAction::Unknown { .. } => {}
        }
        Ok(())
    }

    async fn handle_resend(&self, chat_id: i32, start_wire: i32, end_wire: i32) {
        let (blobs, access_hash) = {
            let store = self.store.lock().await;
            let Ok(state) = store.get(chat_id) else {
                return;
            };
            (state.lookup_sent(start_wire, end_wire), state.access_hash)
        };

        if blobs.is_empty() {
            warn!(
                chat_id,
                start_wire, end_wire, "Resend requested but no buffered blobs found"
            );
            return;
        }

        for (seq, blob) in blobs {
            debug!(chat_id, seq, "replaying sent blob for Resend");
            if let Err(e) = self.do_resend(chat_id, access_hash, blob).await {
                warn!(chat_id, seq, "failed to resend blob: {e}");
            }
        }
    }
}
