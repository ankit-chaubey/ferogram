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

//! Build the `encrypted_message` payload for `auth.bindTempAuthKey`.
//!
//! The inner message must be encrypted with the **permanent** auth key using
//! the legacy **MTProto v1** scheme (SHA-1-based key derivation + AES-256-IGE),
//! NOT the normal MTProto 2.0 (SHA-256) scheme.
//!
//! Wire layout of `encrypted_message`:
//! ```text
//!   perm_key_id (8 bytes)
//!   msg_key     (16 bytes)  = SHA1(plaintext_no_pad)[4..20]
//!   ciphertext  (N bytes)   = AES-IGE(plaintext, derived from perm_key + msg_key)
//!
//! plaintext =
//!   server_salt   (8 bytes, random)
//!   session_id    (8 bytes, random)
//!   msg_id        (8 bytes)
//!   seq_no        (4 bytes, = 0)
//!   data_len      (4 bytes)
//!   bind_auth_key_inner (40 bytes)
//!   padding       (0-15 bytes to reach 16-byte alignment)
//! ```

use ferogram_crypto::{aes, derive_aes_key_iv_v1};
use sha1::{Digest, Sha1};

fn serialize_inner(
    nonce: i64,
    temp_auth_key_id: i64,
    perm_auth_key_id: i64,
    temp_session_id: i64,
    expires_at: i32,
) -> [u8; 40] {
    let mut out = [0u8; 40];
    out[0..4].copy_from_slice(&0x75a3f765_u32.to_le_bytes());
    out[4..12].copy_from_slice(&nonce.to_le_bytes());
    out[12..20].copy_from_slice(&temp_auth_key_id.to_le_bytes());
    out[20..28].copy_from_slice(&perm_auth_key_id.to_le_bytes());
    out[28..36].copy_from_slice(&temp_session_id.to_le_bytes());
    out[36..40].copy_from_slice(&expires_at.to_le_bytes());
    out
}

/// Build the `encrypted_message` bytes for `auth.bindTempAuthKey`.
pub fn encrypt_bind_inner(
    perm_auth_key: &[u8; 256],
    msg_id: i64,
    nonce: i64,
    temp_auth_key_id: i64,
    perm_auth_key_id: i64,
    temp_session_id: i64,
    expires_at: i32,
) -> Vec<u8> {
    let inner = serialize_inner(
        nonce,
        temp_auth_key_id,
        perm_auth_key_id,
        temp_session_id,
        expires_at,
    );

    let header_len = 32usize;
    let content_len = header_len + 40;
    let pad_len = (16 - content_len % 16) % 16;
    let total = content_len + pad_len;

    let mut rnd = [0u8; 24];
    getrandom::getrandom(&mut rnd).expect("getrandom");

    let mut plaintext = Vec::with_capacity(total);
    plaintext.extend_from_slice(&rnd[..8]);
    plaintext.extend_from_slice(&rnd[8..16]);
    plaintext.extend_from_slice(&msg_id.to_le_bytes());
    plaintext.extend_from_slice(&0i32.to_le_bytes());
    plaintext.extend_from_slice(&40u32.to_le_bytes());
    plaintext.extend_from_slice(&inner);
    plaintext.extend_from_slice(&rnd[16..16 + pad_len]);
    assert_eq!(plaintext.len(), total);

    let hash: [u8; 20] = {
        let mut h = Sha1::new();
        h.update(&plaintext[..content_len]);
        h.finalize().into()
    };
    let mut msg_key = [0u8; 16];
    msg_key.copy_from_slice(&hash[4..20]);

    let (aes_key, aes_iv) = derive_aes_key_iv_v1(perm_auth_key, &msg_key);
    aes::ige_encrypt(&mut plaintext, &aes_key, &aes_iv);

    let key_sha: [u8; 20] = {
        let mut h = Sha1::new();
        h.update(perm_auth_key);
        h.finalize().into()
    };

    let mut result = Vec::with_capacity(8 + 16 + plaintext.len());
    result.extend_from_slice(&key_sha[12..20]);
    result.extend_from_slice(&msg_key);
    result.extend_from_slice(&plaintext);
    result
}

/// Serialize `auth.bindTempAuthKey#cdd42a05` to raw TL bytes.
pub fn serialize_bind_temp_auth_key(
    perm_auth_key_id: i64,
    nonce: i64,
    expires_at: i32,
    encrypted_message: &[u8],
) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&0xcdd42a05_u32.to_le_bytes());
    out.extend_from_slice(&perm_auth_key_id.to_le_bytes());
    out.extend_from_slice(&nonce.to_le_bytes());
    out.extend_from_slice(&expires_at.to_le_bytes());
    tl_write_bytes(&mut out, encrypted_message);
    out
}

/// Generate a monotonic MTProto message ID from the current system clock.
pub fn gen_msg_id() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap();
    ((now.as_secs() << 32) | (now.subsec_nanos() as u64 & !3)) as i64
}

fn tl_write_bytes(out: &mut Vec<u8>, data: &[u8]) {
    let len = data.len();
    if len < 254 {
        out.push(len as u8);
        out.extend_from_slice(data);
        let pad = (4 - (1 + len) % 4) % 4;
        out.extend(std::iter::repeat_n(0u8, pad));
    } else {
        out.push(0xfe);
        out.push((len & 0xff) as u8);
        out.push(((len >> 8) & 0xff) as u8);
        out.push(((len >> 16) & 0xff) as u8);
        out.extend_from_slice(data);
        let pad = (4 - (4 + len) % 4) % 4;
        out.extend(std::iter::repeat_n(0u8, pad));
    }
}
