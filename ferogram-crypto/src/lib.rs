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

#![cfg_attr(docsrs, feature(doc_cfg))]
#![doc(html_root_url = "https://docs.rs/ferogram-crypto/0.4.6")]
//! Cryptographic primitives for Telegram MTProto.
//!
//! Provides:
//! - AES-256-IGE encryption/decryption
//! - SHA-1 / SHA-256 hash macros
//! - Pollard-rho PQ factorization
//! - RSA padding (MTProto RSA-PAD scheme)
//! - `AuthKey`: 256-byte session key
//! - MTProto 2.0 message encryption / decryption
//! - DH nonce→key derivation

#![deny(unsafe_code)]

pub mod aes;
mod auth_key;
mod deque_buffer;
mod factorize;
mod obfuscated;
pub mod rsa;
mod sha;

pub use auth_key::AuthKey;
pub use deque_buffer::DequeBuffer;
pub use factorize::factorize;
pub use obfuscated::ObfuscatedCipher;

/// Errors from [`decrypt_data_v2`].
#[derive(Clone, Debug, PartialEq)]
pub enum DecryptError {
    /// Ciphertext too short or not block-aligned.
    InvalidBuffer,
    /// The `auth_key_id` in the ciphertext does not match our key.
    AuthKeyMismatch,
    /// The `msg_key` in the ciphertext does not match our computed value.
    MessageKeyMismatch,
}

impl std::fmt::Display for DecryptError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidBuffer => write!(f, "invalid ciphertext buffer length"),
            Self::AuthKeyMismatch => write!(f, "auth_key_id mismatch"),
            Self::MessageKeyMismatch => write!(f, "msg_key mismatch"),
        }
    }
}
impl std::error::Error for DecryptError {}

enum Side {
    Client,
    Server,
}
impl Side {
    fn x(&self) -> usize {
        match self {
            Side::Client => 0,
            Side::Server => 8,
        }
    }
}

fn calc_key(auth_key: &AuthKey, msg_key: &[u8; 16], side: Side) -> ([u8; 32], [u8; 32]) {
    let x = side.x();
    let sha_a = sha256!(msg_key, &auth_key.data[x..x + 36]);
    let sha_b = sha256!(&auth_key.data[40 + x..40 + x + 36], msg_key);

    let mut aes_key = [0u8; 32];
    aes_key[..8].copy_from_slice(&sha_a[..8]);
    aes_key[8..24].copy_from_slice(&sha_b[8..24]);
    aes_key[24..].copy_from_slice(&sha_a[24..]);

    let mut aes_iv = [0u8; 32];
    aes_iv[..8].copy_from_slice(&sha_b[..8]);
    aes_iv[8..24].copy_from_slice(&sha_a[8..24]);
    aes_iv[24..].copy_from_slice(&sha_b[24..]);

    (aes_key, aes_iv)
}

fn padding_len(len: usize) -> usize {
    // MTProto 2.0 requires 12–1024 bytes of random padding, and the total
    // (payload + padding) must be a multiple of 16.
    // Minimum padding = 12; extra bytes to hit the next 16-byte boundary.
    let rem = (len + 12) % 16;
    if rem == 0 { 12 } else { 12 + (16 - rem) }
}

/// Encrypt `buffer` (in-place, with prepended header) using MTProto 2.0.
///
/// After this call `buffer` contains `key_id || msg_key || ciphertext`.
pub fn encrypt_data_v2(buffer: &mut DequeBuffer, auth_key: &AuthKey) {
    let mut rnd = [0u8; 32];
    getrandom::getrandom(&mut rnd).expect("getrandom failed");
    do_encrypt_data_v2(buffer, auth_key, &rnd);
}

pub(crate) fn do_encrypt_data_v2(buffer: &mut DequeBuffer, auth_key: &AuthKey, rnd: &[u8; 32]) {
    let pad = padding_len(buffer.len());
    buffer.extend(rnd.iter().take(pad).copied());

    let x = Side::Client.x();
    let msg_key_large = sha256!(&auth_key.data[88 + x..88 + x + 32], buffer.as_ref());
    let mut msg_key = [0u8; 16];
    msg_key.copy_from_slice(&msg_key_large[8..24]);

    let (key, iv) = calc_key(auth_key, &msg_key, Side::Client);
    aes::ige_encrypt(buffer.as_mut(), &key, &iv);

    buffer.extend_front(&msg_key);
    buffer.extend_front(&auth_key.key_id);
}

/// Decrypt an MTProto 2.0 ciphertext.
///
/// `buffer` must start with `key_id || msg_key || ciphertext`.
/// On success returns a slice of `buffer` containing the plaintext.
pub fn decrypt_data_v2<'a>(
    buffer: &'a mut [u8],
    auth_key: &AuthKey,
) -> Result<&'a mut [u8], DecryptError> {
    if buffer.len() < 24 || !(buffer.len() - 24).is_multiple_of(16) {
        return Err(DecryptError::InvalidBuffer);
    }
    if auth_key.key_id != buffer[..8] {
        return Err(DecryptError::AuthKeyMismatch);
    }
    let mut msg_key = [0u8; 16];
    msg_key.copy_from_slice(&buffer[8..24]);

    let (key, iv) = calc_key(auth_key, &msg_key, Side::Server);
    aes::ige_decrypt(&mut buffer[24..], &key, &iv);

    let x = Side::Server.x();
    let our_key = sha256!(&auth_key.data[88 + x..88 + x + 32], &buffer[24..]);
    if msg_key != our_key[8..24] {
        return Err(DecryptError::MessageKeyMismatch);
    }
    Ok(&mut buffer[24..])
}

/// Derive `(key, iv)` from nonces for decrypting `ServerDhParams.encrypted_answer`.
pub fn generate_key_data_from_nonce(
    server_nonce: &[u8; 16],
    new_nonce: &[u8; 32],
) -> ([u8; 32], [u8; 32]) {
    let h1 = sha1!(new_nonce, server_nonce);
    let h2 = sha1!(server_nonce, new_nonce);
    let h3 = sha1!(new_nonce, new_nonce);

    let mut key = [0u8; 32];
    key[..20].copy_from_slice(&h1);
    key[20..].copy_from_slice(&h2[..12]);

    let mut iv = [0u8; 32];
    iv[..8].copy_from_slice(&h2[12..]);
    iv[8..28].copy_from_slice(&h3);
    iv[28..].copy_from_slice(&new_nonce[..4]);

    (key, iv)
}

/// Derive the AES key and IV for **MTProto v1** (old-style, SHA-1-based).
///
/// Used exclusively for `auth.bindTempAuthKey` encrypted_message, which must
/// be encrypted with the permanent key using the legacy SHA-1 scheme - NOT the
/// SHA-256 MTProto 2.0 scheme used for all normal messages.
pub fn derive_aes_key_iv_v1(auth_key: &[u8; 256], msg_key: &[u8; 16]) -> ([u8; 32], [u8; 32]) {
    let sha1_a = sha1!(msg_key, &auth_key[0..32]);
    let sha1_b = sha1!(&auth_key[32..48], msg_key, &auth_key[48..64]);
    let sha1_c = sha1!(&auth_key[64..96], msg_key);
    let sha1_d = sha1!(msg_key, &auth_key[96..128]);

    let mut key = [0u8; 32];
    key[..8].copy_from_slice(&sha1_a[..8]);
    key[8..20].copy_from_slice(&sha1_b[8..20]);
    key[20..32].copy_from_slice(&sha1_c[4..16]);

    let mut iv = [0u8; 32];
    iv[..12].copy_from_slice(&sha1_a[8..20]);
    iv[12..20].copy_from_slice(&sha1_b[..8]);
    iv[20..24].copy_from_slice(&sha1_c[16..20]);
    iv[24..32].copy_from_slice(&sha1_d[..8]);

    (key, iv)
}

/// Telegram's published 2048-bit safe DH prime (big-endian, 256 bytes).
///
/// Source: <https://core.telegram.org/mtproto/auth_key>
#[rustfmt::skip]
const TELEGRAM_DH_PRIME: [u8; 256] = [
    0xC7, 0x1C, 0xAE, 0xB9, 0xC6, 0xB1, 0xC9, 0x04,
    0x8E, 0x6C, 0x52, 0x2F, 0x70, 0xF1, 0x3F, 0x73,
    0x98, 0x0D, 0x40, 0x23, 0x8E, 0x3E, 0x21, 0xC1,
    0x49, 0x34, 0xD0, 0x37, 0x56, 0x3D, 0x93, 0x0F,
    0x48, 0x19, 0x8A, 0x0A, 0xA7, 0xC1, 0x40, 0x58,
    0x22, 0x94, 0x93, 0xD2, 0x25, 0x30, 0xF4, 0xDB,
    0xFA, 0x33, 0x6F, 0x6E, 0x0A, 0xC9, 0x25, 0x13,
    0x95, 0x43, 0xAE, 0xD4, 0x4C, 0xCE, 0x7C, 0x37,
    0x20, 0xFD, 0x51, 0xF6, 0x94, 0x58, 0x70, 0x5A,
    0xC6, 0x8C, 0xD4, 0xFE, 0x6B, 0x6B, 0x13, 0xAB,
    0xDC, 0x97, 0x46, 0x51, 0x29, 0x69, 0x32, 0x84,
    0x54, 0xF1, 0x8F, 0xAF, 0x8C, 0x59, 0x5F, 0x64,
    0x24, 0x77, 0xFE, 0x96, 0xBB, 0x2A, 0x94, 0x1D,
    0x5B, 0xCD, 0x1D, 0x4A, 0xC8, 0xCC, 0x49, 0x88,
    0x07, 0x08, 0xFA, 0x9B, 0x37, 0x8E, 0x3C, 0x4F,
    0x3A, 0x90, 0x60, 0xBE, 0xE6, 0x7C, 0xF9, 0xA4,
    0xA4, 0xA6, 0x95, 0x81, 0x10, 0x51, 0x90, 0x7E,
    0x16, 0x27, 0x53, 0xB5, 0x6B, 0x0F, 0x6B, 0x41,
    0x0D, 0xBA, 0x74, 0xD8, 0xA8, 0x4B, 0x2A, 0x14,
    0xB3, 0x14, 0x4E, 0x0E, 0xF1, 0x28, 0x47, 0x54,
    0xFD, 0x17, 0xED, 0x95, 0x0D, 0x59, 0x65, 0xB4,
    0xB9, 0xDD, 0x46, 0x58, 0x2D, 0xB1, 0x17, 0x8D,
    0x16, 0x9C, 0x6B, 0xC4, 0x65, 0xB0, 0xD6, 0xFF,
    0x9C, 0xA3, 0x92, 0x8F, 0xEF, 0x5B, 0x9A, 0xE4,
    0xE4, 0x18, 0xFC, 0x15, 0xE8, 0x3E, 0xBE, 0xA0,
    0xF8, 0x7F, 0xA9, 0xFF, 0x5E, 0xED, 0x70, 0x05,
    0x0D, 0xED, 0x28, 0x49, 0xF4, 0x7B, 0xF9, 0x59,
    0xD9, 0x56, 0x85, 0x0C, 0xE9, 0x29, 0x85, 0x1F,
    0x0D, 0x81, 0x15, 0xF6, 0x35, 0xB1, 0x05, 0xEE,
    0x2E, 0x4E, 0x15, 0xD0, 0x4B, 0x24, 0x54, 0xBF,
    0x6F, 0x4F, 0xAD, 0xF0, 0x34, 0xB1, 0x04, 0x03,
    0x11, 0x9C, 0xD8, 0xE3, 0xB9, 0x2F, 0xCC, 0x5B,
];

/// Errors returned by [`check_p_and_g`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DhError {
    /// `dh_prime` is not exactly 256 bytes (2048 bits).
    PrimeLengthInvalid,
    /// The most-significant bit of `dh_prime` is zero, so it is actually
    /// shorter than 2048 bits.
    PrimeTooSmall,
    /// `dh_prime` does not match Telegram's published safe prime.
    PrimeUnknown,
    /// `g` is outside the set {2, 3, 4, 5, 6, 7}.
    GeneratorOutOfRange,
    /// The modular-residue condition required by `g` and the prime is not
    /// satisfied (see MTProto spec §4.5).
    GeneratorInvalid,
}

impl std::fmt::Display for DhError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::PrimeLengthInvalid => write!(f, "dh_prime must be exactly 256 bytes"),
            Self::PrimeTooSmall => write!(f, "dh_prime high bit is clear (< 2048 bits)"),
            Self::PrimeUnknown => {
                write!(f, "dh_prime does not match any known Telegram safe prime")
            }
            Self::GeneratorOutOfRange => write!(f, "generator g must be 2, 3, 4, 5, 6, or 7"),
            Self::GeneratorInvalid => write!(
                f,
                "g fails the required modular-residue check for this prime"
            ),
        }
    }
}

impl std::error::Error for DhError {}

/// Compute `big_endian_bytes mod modulus` (all values < 2^64).
fn prime_residue(bytes: &[u8], modulus: u64) -> u64 {
    bytes
        .iter()
        .fold(0u64, |acc, &b| (acc * 256 + b as u64) % modulus)
}

/// Validate the Diffie-Hellman prime `p` and generator `g` received from
/// the Telegram server during MTProto key exchange.
///
/// Checks performed (per MTProto spec §4.5):
///
/// 1. `dh_prime` is exactly 256 bytes (2048 bits).
/// 2. The most-significant bit is set: the number is truly 2048 bits.
/// 3. `dh_prime` matches Telegram's published safe prime exactly.
/// 4. `g` ∈ {2, 3, 4, 5, 6, 7}.
/// 5. The residue condition for `g` and the prime holds:
///
///    | g | condition           |
///    |---|---------------------|
///    | 2 | p mod 8 = 7         |
///    | 3 | p mod 3 = 2         |
///    | 4 | always valid        |
///    | 5 | p mod 5 ∈ {1, 4}    |
///    | 6 | p mod 24 ∈ {19, 23} |
///    | 7 | p mod 7 ∈ {3, 5, 6} |
pub fn check_p_and_g(dh_prime: &[u8], g: u32) -> Result<(), DhError> {
    // 1. Length
    if dh_prime.len() != 256 {
        return Err(DhError::PrimeLengthInvalid);
    }

    // 2. High bit set
    if dh_prime[0] & 0x80 == 0 {
        return Err(DhError::PrimeTooSmall);
    }

    // 3. Known prime: exact match guarantees the residue conditions below
    //  are deterministic constants, so check 5 is redundant after this.
    if dh_prime != &TELEGRAM_DH_PRIME[..] {
        return Err(DhError::PrimeUnknown);
    }

    // 4. Generator range
    if !(2..=7).contains(&g) {
        return Err(DhError::GeneratorOutOfRange);
    }

    // 5. Residue condition per MTProto spec §4.5.
    let valid = match g {
        2 => prime_residue(dh_prime, 8) == 7,
        3 => prime_residue(dh_prime, 3) == 2,
        4 => true,
        5 => {
            let r = prime_residue(dh_prime, 5);
            r == 1 || r == 4
        }
        6 => {
            let r = prime_residue(dh_prime, 24);
            r == 19 || r == 23
        }
        7 => {
            let r = prime_residue(dh_prime, 7);
            r == 3 || r == 5 || r == 6
        }
        _ => unreachable!(),
    };
    if !valid {
        return Err(DhError::GeneratorInvalid);
    }

    Ok(())
}

#[cfg(test)]
mod dh_tests {
    use super::*;

    #[test]
    fn known_prime_g3_valid() {
        // Telegram almost always sends g=3 with this prime.
        assert_eq!(check_p_and_g(&TELEGRAM_DH_PRIME, 3), Ok(()));
    }

    #[test]
    fn wrong_length_rejected() {
        assert_eq!(
            check_p_and_g(&[0u8; 128], 3),
            Err(DhError::PrimeLengthInvalid)
        );
    }

    #[test]
    fn unknown_prime_rejected() {
        let mut fake = TELEGRAM_DH_PRIME;
        fake[255] ^= 0x01; // flip last bit
        assert_eq!(check_p_and_g(&fake, 3), Err(DhError::PrimeUnknown));
    }

    #[test]
    fn out_of_range_g_rejected() {
        assert_eq!(
            check_p_and_g(&TELEGRAM_DH_PRIME, 1),
            Err(DhError::GeneratorOutOfRange)
        );
        assert_eq!(
            check_p_and_g(&TELEGRAM_DH_PRIME, 8),
            Err(DhError::GeneratorOutOfRange)
        );
    }
}
