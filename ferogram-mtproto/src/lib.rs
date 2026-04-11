// Copyright (c) Ankit Chaubey <ankitchaubey.dev@gmail.com>
// SPDX-License-Identifier: MIT OR Apache-2.0
//
// ferogram: async Telegram MTProto client in Rust
// https://github.com/ankit-chaubey/ferogram
//
// Based on layer: https://github.com/ankit-chaubey/layer
// Follows official Telegram client behaviour (tdesktop, TDLib).
//
// If you use or modify this code, keep this notice at the top of your file
// and include the LICENSE-MIT or LICENSE-APACHE file from this repository:
// https://github.com/ankit-chaubey/ferogram

#![cfg_attr(docsrs, feature(doc_cfg))]
#![doc(html_root_url = "https://docs.rs/ferogram-mtproto/0.4.6")]
//! MTProto session and transport abstractions.
//!
//! This crate handles:
//! * Message framing (sequence numbers, message IDs)
//! * Plaintext transport (for initial handshake / key exchange)
//! * Encrypted transport skeleton (requires a crypto backend)
//!
//! It is intentionally transport-agnostic: bring your own TCP/WebSocket.

#![deny(unsafe_code)]
#![warn(missing_docs)]

pub mod authentication;
pub mod encrypted;
pub mod message;
pub mod session;
pub mod transport;

pub use authentication::{Finished, finish, step1, step2, step3};
pub use encrypted::EncryptedSession;
pub use message::{Message, MessageId};
pub use session::Session;
