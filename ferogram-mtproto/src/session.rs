// Copyright (c) Ankit Chaubey <ankitchaubey.dev@gmail.com>
// SPDX-License-Identifier: MIT OR Apache-2.0
//
// ferogram: async Telegram MTProto client in Rust
// https://github.com/ankit-chaubey/ferogram
//
//
// If you use or modify this code, keep this notice at the top of your file
// and include the LICENSE-MIT or LICENSE-APACHE file from this repository:
// https://github.com/ankit-chaubey/ferogram

//! MTProto client session state.

use ferogram_tl_types::RemoteCall;

use crate::message::{Message, MessageId};

/// Tracks per-connection MTProto session state.
///
/// A `Session` is cheap to create and can be reset on reconnect.
///
/// # Example
///
/// ```rust
/// use ferogram_mtproto::Session;
/// use ferogram_tl_types::functions;
///
/// let mut session = Session::new();
/// // let msg = session.pack(&my_request);
/// // send(msg.to_plaintext_bytes()).await?;
/// ```
pub struct Session {
    /// Monotonically increasing counter used to generate unique message IDs.
    msg_counter: u32,
    /// The sequence number for the next message.
    /// Even for content-unrelated messages, odd for content-related (RPC calls).
    seq_no: i32,
}

impl Session {
    /// Create a fresh session.
    pub fn new() -> Self {
        Self {
            msg_counter: 0,
            seq_no: 0,
        }
    }

    /// Allocate a new message ID.
    pub fn next_msg_id(&mut self) -> MessageId {
        self.msg_counter = self.msg_counter.wrapping_add(1);
        MessageId::generate(self.msg_counter)
    }

    /// Return the next sequence number for a content-related message (RPC call).
    ///
    /// Increments by 2 after each call so that even slots remain available
    /// for content-unrelated messages (acks, pings, etc.).
    pub fn next_seq_no(&mut self) -> i32 {
        let n = self.seq_no;
        self.seq_no += 2;
        n | 1 // odd = content-related
    }

    /// Return the next sequence number for a content-*un*related message.
    pub fn next_seq_no_unrelated(&mut self) -> i32 {
        let n = self.seq_no;
        n & !1 // even = content-unrelated (don't increment)
    }

    /// Serialize an RPC function into a [`Message`] ready to send.
    ///
    /// The message body is just the TL-serialized `call`; the surrounding
    /// transport framing (auth_key_id, etc.) is applied in [`Message::to_plaintext_bytes`].
    pub fn pack<R: RemoteCall>(&mut self, call: &R) -> Message {
        let id = self.next_msg_id();
        let seq_no = self.next_seq_no();
        let body = call.to_bytes();
        Message::plaintext(id, seq_no, body)
    }
}

impl Default for Session {
    fn default() -> Self {
        Self::new()
    }
}
