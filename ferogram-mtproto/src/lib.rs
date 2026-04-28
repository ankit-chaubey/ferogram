// Copyright (c) Ankit Chaubey <ankitchaubey.dev@gmail.com>
// SPDX-License-Identifier: MIT OR Apache-2.0
//
// ferogram: async Telegram MTProto client in Rust
// https://github.com/ankit-chaubey/ferogram
//
// If you use or modify this code, keep this notice at the top of your file
// and include the LICENSE-MIT or LICENSE-APACHE file from this repository:
// https://github.com/ankit-chaubey/ferogram

#![cfg_attr(docsrs, feature(doc_cfg))]
#![doc(html_root_url = "https://docs.rs/ferogram-mtproto/0.3.3")]
//! MTProto 2.0 session management, message framing, DH key exchange, and transport abstractions.
//!
//! This crate is part of [ferogram](https://crates.io/crates/ferogram), an async Rust
//! MTProto client built by [Ankit Chaubey](https://github.com/ankit-chaubey).
//!
//! - Channel: [t.me/Ferogram](https://t.me/Ferogram)
//! - Chat: [t.me/FerogramChat](https://t.me/FerogramChat)
//!
//! Most users do not need this crate directly. Use the `ferogram` crate.
//! This is for anyone building a lower-level MTProto stack on top of the
//! crypto and framing primitives.
//!
//! # Modules
//!
//! - [`authentication`]: Sans-IO DH key exchange (steps 1-3 + finish).
//!   Does no I/O itself; you drive it by passing the serialized requests
//!   over your own transport and feeding back the responses.
//! - [`encrypted`]: [`EncryptedSession`]: packs and unpacks MTProto 2.0
//!   encrypted messages once you have a finished `AuthKey`.
//! - [`session`]: [`Session`]: tracks plaintext sequence numbers and
//!   message IDs for the pre-auth handshake phase.
//! - [`transport`]: [`Transport`] trait + [`AbridgedTransport`] and
//!   [`ObfuscatedAbridged`] implementations over any `Read + Write` stream.
//! - [`message`]: [`Message`] and [`MessageId`] framing types.
//! - [`bind_temp_key`]: Helpers for binding a temporary auth key to a
//!   permanent one (used by CDN and multi-DC flows).
//!
//! # DH handshake flow
//!
//! ```text
//! let (req, s1) = authentication::step1()?;
//! // serialize req, send over transport, receive resp (ResPQ)
//! let (req, s2) = authentication::step2(s1, resp, dc_id)?;
//! // serialize req, send, receive resp (ServerDhParams)
//! let (req, s3) = authentication::step3(s2, resp)?;
//! // serialize req, send, receive resp (SetClientDhParamsAnswer)
//! let result = authentication::finish(s3, resp)?;
//! // FinishResult::Done(d)  =>  d.auth_key is your 256-byte session key
//! // FinishResult::Retry    =>  call retry_step3() + finish(), up to 5 times
//! ```
//!
//! # Encrypted session
//!
//! ```rust,no_run
//! use ferogram_mtproto::{EncryptedSession, authentication};
//!
//! # fn example(auth_key: [u8; 256], first_salt: i64, time_offset: i32) {
//! let mut session = EncryptedSession::new(auth_key, first_salt, time_offset);
//!
//! // Pack an RPC call into an encrypted MTProto message
//! // let wire = session.pack(&my_tl_function);
//! // transport.send_message(&wire)?;
//!
//! // Unpack a received message
//! // let decrypted = session.unpack(&mut raw_bytes)?;
//! // decrypted.body contains the TL-serialized response
//! # }
//! ```
//!
//! # Transport
//!
//! Implement [`transport::Transport`] over any byte stream to get MTProto
//! framing for free. Two built-in implementations are provided:
//!
//! - [`transport::AbridgedTransport`]: direct connection, no ISP protection.
//! - [`transport::ObfuscatedAbridged`]: AES-CTR obfuscation that defeats
//!   DPI-based blocking of plain Telegram traffic.

#![deny(unsafe_code)]
#![warn(missing_docs)]

/// MTProto authentication key generation (DH handshake steps).
pub mod authentication;
/// Temporary/permanent auth key binding via `bindTempAuthKey`.
pub mod bind_temp_key;
/// Encrypted MTProto message construction and parsing.
pub mod encrypted;
/// MTProto message framing and container types.
pub mod message;
/// Session state: sequence numbers, salt, and server time.
pub mod session;
/// Transport-layer encoding (abridged, intermediate, padded).
pub mod transport;

pub use authentication::{
    FinishResult, Finished, finish, retry_step3, step1, step2, step2_temp, step3,
};
pub use bind_temp_key::{
    auth_key_id_from_key, encrypt_bind_inner, gen_msg_id, serialize_bind_temp_auth_key,
};
pub use encrypted::{DecryptedMessage, EncryptedSession, SeenMsgIds, new_seen_msg_ids};
pub use message::{Message, MessageId};
pub use session::Session;
