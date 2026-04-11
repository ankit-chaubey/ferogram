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

use ferogram_mtproto::{
    Session,
    transport::{AbridgedTransport, Transport},
};

#[test]
fn session_seq_no_increments() {
    let mut s = Session::new();
    let a = s.next_seq_no();
    let b = s.next_seq_no();
    assert!(a & 1 == 1, "content-related seq_no must be odd");
    assert!(b & 1 == 1);
    assert!(b > a, "seq_no must increase");
}

#[test]
fn session_unrelated_seq_no_is_even() {
    let mut s = Session::new();
    let n = s.next_seq_no_unrelated();
    assert_eq!(n & 1, 0, "unrelated seq_no must be even");
}

#[test]
fn message_plaintext_bytes_layout() {
    let mut s = Session::new();
    // Use a zero-length body to inspect the fixed header
    use ferogram_mtproto::Message;
    let id = s.next_msg_id();
    let msg = Message::plaintext(id, 1, vec![0xAA, 0xBB]);
    let wire = msg.to_plaintext_bytes();

    // auth_key_id (8 bytes) + msg_id (8 bytes) + length (4 bytes) + body (2 bytes)
    assert_eq!(wire.len(), 8 + 8 + 4 + 2);
    // auth_key_id must be 0 for plaintext
    assert_eq!(&wire[..8], &[0u8; 8]);
    // length field must match body
    assert_eq!(u32::from_le_bytes(wire[16..20].try_into().unwrap()), 2);
    // body is intact
    assert_eq!(&wire[20..], &[0xAA, 0xBB]);
}

// AbridgedTransport

struct MemTransport {
    inbox: Vec<u8>,
    outbox: Vec<u8>,
}

impl Transport for MemTransport {
    type Error = std::io::Error;
    fn send(&mut self, data: &[u8]) -> Result<(), Self::Error> {
        self.outbox.extend_from_slice(data);
        Ok(())
    }
    fn recv(&mut self) -> Result<Vec<u8>, Self::Error> {
        Ok(std::mem::take(&mut self.inbox))
    }
}

#[test]
fn abridged_sends_init_byte_once() {
    let inner = MemTransport {
        inbox: vec![],
        outbox: vec![],
    };
    let mut t = AbridgedTransport::new(inner);

    let payload = vec![0u8; 4]; // 4 bytes = 1 word
    t.send_message(&payload).unwrap();
    // First byte must be 0xef (init)
    assert_eq!(t.inner_mut().outbox[0], 0xef);

    let prev_len = t.inner_mut().outbox.len();
    // Second call must NOT send 0xef again
    t.send_message(&payload).unwrap();
    let second_byte = t.inner_mut().outbox[prev_len];
    assert_ne!(second_byte, 0xef, "init byte must only be sent once");
}
