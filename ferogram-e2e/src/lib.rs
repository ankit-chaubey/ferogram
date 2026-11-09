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

#![cfg_attr(docsrs, feature(doc_cfg))]
#![doc(html_root_url = "https://docs.rs/ferogram-e2e/0.1.0-b1")]
#![deny(unsafe_code)]

//! Telegram secret chats (end-to-end encryption) for ferogram.
//!
//! This crate is part of [ferogram](https://crates.io/crates/ferogram), an async Rust
//! MTProto client built by [Ankit Chaubey](https://github.com/ankit-chaubey).
//!
//! - Channel: [t.me/Ferogram](https://t.me/Ferogram)
//! - Chat: [t.me/FerogramChat](https://t.me/FerogramChat)
//!
//! Most users should enable the `e2e` feature of the `ferogram` crate and use
//! `ferogram::e2e`, which re-exports this crate and connects it to `ferogram::Client`.
//! Use `ferogram-e2e` directly only to drive secret chats from another client by
//! implementing [`E2eTransport`]; in that case [`E2eClient::request_secret_chat_raw`] takes the
//! peer's user id and access hash directly.
//!
//! # What's in here
//!
//! - **Key exchange**: Diffie-Hellman handshake with Telegram's validity checks ([`dh`]).
//! - **Messages**: MTProto 2.0 encryption of the inner TL layer ([`crypto`], [`tl`]).
//! - **Files**: per-file AES-256-IGE encryption plus chunked upload ([`upload`]).
//! - **Chat state**: sequence numbers, TTLs, re-keying, optional binary persistence ([`state`]).
//! - **High-level API**: [`E2eClient`] to request, accept and use secret chats, and
//!   [`E2eEvent`] for what comes back from the update stream.
//!
//! # Security notes
//!
//! Persisted chat state (see [`E2eClient::for_session`] and
//! [`E2eClient::with_persistence`]) contains the 256-byte `auth_key` of every chat,
//! unencrypted. The file is created with mode `0600` on Unix; on other platforms keep it
//! somewhere only your process can read.
//!
//! # Example
//!
//! ```rust,ignore
//! // With the `e2e` feature of `ferogram` enabled:
//! use ferogram::e2e::{E2eClient, E2eClientExt, E2eEvent};
//!
//! let e2e = E2eClient::new(client.clone());
//! let chat_id = e2e.request_secret_chat("@username").await?;
//!
//! let mut stream = client.stream_updates();
//! while let Some(update) = stream.next().await {
//!     if let Some(E2eEvent::ChatReady { chat_id }) = e2e.handle_client_update(&update).await? {
//!         e2e.send_text(chat_id, "hello", 0).await?;
//!     }
//! }
//! ```

pub mod chat;
pub mod crypto;
pub mod dh;
pub mod error;
pub mod state;
pub mod tl;
pub mod transport;
pub mod update;
pub mod upload;

pub use chat::E2eClient;
pub use error::E2eError;
pub use ferogram_tl_types::enums::EncryptedFile;
pub use tl::{
    DecryptedAction, DecryptedMedia, DecryptedMessage, DecryptedMessageLayer, DocumentAttribute,
};
pub use transport::E2eTransport;
pub use update::E2eEvent;
