// Copyright (c) Ankit Chaubey <ankitchaubey.dev@gmail.com>
// SPDX-License-Identifier: MIT OR Apache-2.0
//
// ferogram: async Telegram MTProto client in Rust
// https://github.com/ankit-chaubey/ferogram
//
// If you use or modify this code, keep this notice at the top of your file
// and include the LICENSE-MIT or LICENSE-APACHE file from this repository:
// https://github.com/ankit-chaubey/ferogram

use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

/// Process-wide monotonically increasing counter for plaintext message IDs.
///
/// A global atomic ensures msg_id monotonicity across concurrent sessions.
static GLOBAL_MSG_COUNTER: AtomicU32 = AtomicU32::new(1);

/// A 64-bit MTProto message identifier.
///
/// Per the spec: the lower 32 bits are derived from the current Unix time;
/// the upper 32 bits are a monotonically increasing counter within the second.
/// The least significant two bits must be zero for client messages.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct MessageId(pub u64);

impl MessageId {
    /// Generate a new message ID using the system clock and a global counter.
    ///
    /// MTProto msg_id layout:
    ///   bits 63–32: Unix timestamp in seconds (upper 32 bits)
    ///   bits 31–2:  intra-second sequencing counter (lower 30 bits, × 4)
    ///   bits 1–0:   must be 0b00 for client messages
    ///
    pub(crate) fn generate(_counter: u32) -> Self {
        let unix_secs = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        // Global atomic: wrapping_add ensures monotonicity across concurrent sessions.
        let counter = GLOBAL_MSG_COUNTER.fetch_add(1, Ordering::Relaxed);
        // upper 32 bits = seconds, lower 30 bits = counter × 4 (bits 1-0 = 0b00)
        let id = (unix_secs << 32) | (u64::from(counter) << 2);
        Self(id)
    }
}

/// A framed MTProto message ready to be sent.
#[derive(Debug)]
pub struct Message {
    /// Unique identifier for this message.
    pub id: MessageId,
    /// Session-scoped sequence number (even for content-unrelated, odd for content-related).
    pub seq_no: i32,
    /// The serialized TL body (constructor ID + fields).
    pub body: Vec<u8>,
}

impl Message {
    /// Construct a new plaintext message (used before key exchange).
    pub fn plaintext(id: MessageId, seq_no: i32, body: Vec<u8>) -> Self {
        Self { id, seq_no, body }
    }

    /// Serialize the message into the plaintext wire format:
    ///
    /// ```text
    /// auth_key_id:long  (0 for plaintext)
    /// message_id:long
    /// message_data_length:int
    /// message_data:bytes
    /// ```
    pub fn to_plaintext_bytes(&self) -> Vec<u8> {
        let mut buf = Vec::with_capacity(8 + 8 + 4 + self.body.len());
        buf.extend(0i64.to_le_bytes()); // auth_key_id = 0
        buf.extend(self.id.0.to_le_bytes()); // message_id
        buf.extend((self.body.len() as u32).to_le_bytes()); // length
        buf.extend(&self.body);
        buf
    }
}
