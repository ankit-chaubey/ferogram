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

//! Cryptographic primitives for Telegram secret chats (MTProto 2.0).
//!
//! - Message encryption: [`encrypt_message`] / [`decrypt_message`].
//! - Per-file encryption: [`file_encrypt`] / [`file_decrypt`] and
//!   [`file_key_fingerprint`].
//! - Key identification: [`key_fingerprint`] and [`emoji_fingerprint_indices`].

use ferogram_crypto::aes::{ige_decrypt, ige_encrypt};
use rand::Rng;
use sha1::{Digest as _, Sha1};
use sha2::Sha256;

use crate::error::E2eError;

/// Minimum number of random padding bytes in MTProto 2.0.
const MIN_PADDING: usize = 12;

/// Maximum number of random padding bytes in MTProto 2.0.
const MAX_PADDING: usize = 1024;

/// Number of padding bytes (12..=27) that bring `msg_len + padding` to a multiple of 16.
fn padding_len(msg_len: usize) -> usize {
    let rem = (msg_len + MIN_PADDING) % 16;
    if rem == 0 {
        MIN_PADDING
    } else {
        MIN_PADDING + (16 - rem)
    }
}

/// Derive the AES-256-IGE key and IV from `auth_key` and `msg_key`.
///
/// `x` is 0 for messages sent by the chat originator and 8 for the opposite direction.
fn derive_key_iv(auth_key: &[u8; 256], msg_key: &[u8; 16], x: usize) -> ([u8; 32], [u8; 32]) {
    let mut input_a = Vec::with_capacity(16 + 36);
    input_a.extend_from_slice(msg_key);
    input_a.extend_from_slice(&auth_key[x..x + 36]);
    let sha_a: [u8; 32] = Sha256::digest(&input_a).into();

    let mut input_b = Vec::with_capacity(36 + 16);
    input_b.extend_from_slice(&auth_key[40 + x..40 + x + 36]);
    input_b.extend_from_slice(msg_key);
    let sha_b: [u8; 32] = Sha256::digest(&input_b).into();

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

/// Compute the 64-bit `key_fingerprint`: the last 8 bytes of `SHA1(auth_key)`, read as a
/// little-endian `i64`.
///
/// This is a sanity check on the key exchange, not the key visualization shown to users.
pub fn key_fingerprint(auth_key: &[u8; 256]) -> i64 {
    let hash: [u8; 20] = Sha1::digest(auth_key).into();
    let mut b = [0u8; 8];
    b.copy_from_slice(&hash[12..20]);
    i64::from_le_bytes(b)
}

/// Encrypt a secret chat message blob.
///
/// `plaintext` is the fully serialized inner TL (`DecryptedMessageLayer` bytes
/// for layer 17 and above, or bare layer-8 service bytes for `notify_layer`).
/// `admin` is `true` when we initiated the chat.
///
/// Returns `key_fingerprint (8, LE) || msg_key (16) || ciphertext`.
pub fn encrypt_message(
    auth_key: &[u8; 256],
    plaintext: &[u8],
    admin: bool,
) -> Result<Vec<u8>, E2eError> {
    let x: usize = if admin { 0 } else { 8 };

    let current_len = 4 + plaintext.len();
    let pad_len = padding_len(current_len);

    let mut buf = Vec::with_capacity(current_len + pad_len);
    buf.extend_from_slice(&(plaintext.len() as u32).to_le_bytes());
    buf.extend_from_slice(plaintext);
    let mut pad = vec![0u8; pad_len];
    rand::rng().fill_bytes(&mut pad);
    buf.extend_from_slice(&pad);

    let mut key_input = Vec::with_capacity(32 + buf.len());
    key_input.extend_from_slice(&auth_key[88 + x..88 + x + 32]);
    key_input.extend_from_slice(&buf);
    let msg_key_large: [u8; 32] = Sha256::digest(&key_input).into();
    let mut msg_key = [0u8; 16];
    msg_key.copy_from_slice(&msg_key_large[8..24]);

    let (aes_key, aes_iv) = derive_key_iv(auth_key, &msg_key, x);
    ige_encrypt(&mut buf, &aes_key, &aes_iv);

    let fp = key_fingerprint(auth_key);
    let mut out = Vec::with_capacity(8 + 16 + buf.len());
    out.extend_from_slice(&fp.to_le_bytes());
    out.extend_from_slice(&msg_key);
    out.extend_from_slice(&buf);

    Ok(out)
}

/// Decrypt an incoming secret chat blob.
///
/// `data` is `key_fingerprint (8) || msg_key (16) || ciphertext`.
/// `admin` is `true` when we initiated the chat.
///
/// Verifies the key fingerprint, the `msg_key` (SHA-256 over a fragment of
/// `auth_key` and the decrypted body), the length prefix and the padding size
/// (12..=1024 bytes). Returns the inner TL bytes without the 4-byte length prefix.
pub fn decrypt_message(
    auth_key: &[u8; 256],
    data: &[u8],
    admin: bool,
) -> Result<Vec<u8>, E2eError> {
    // Decrypt uses opposite x from encrypt.
    let x: usize = if admin { 8 } else { 0 };

    if data.len() < 24 {
        return Err(E2eError::DecryptionFailed);
    }

    let mut fp_bytes = [0u8; 8];
    fp_bytes.copy_from_slice(&data[0..8]);
    let received_fp = i64::from_le_bytes(fp_bytes);
    let expected_fp = key_fingerprint(auth_key);
    if received_fp != expected_fp {
        return Err(E2eError::KeyFingerprintMismatch);
    }

    let mut msg_key = [0u8; 16];
    msg_key.copy_from_slice(&data[8..24]);

    let ciphertext = &data[24..];
    if !ciphertext.len().is_multiple_of(16) || ciphertext.is_empty() {
        return Err(E2eError::DecryptionFailed);
    }

    let (aes_key, aes_iv) = derive_key_iv(auth_key, &msg_key, x);

    let mut decrypted = ciphertext.to_vec();
    ige_decrypt(&mut decrypted, &aes_key, &aes_iv);

    let mut verify_input = Vec::with_capacity(32 + decrypted.len());
    verify_input.extend_from_slice(&auth_key[88 + x..88 + x + 32]);
    verify_input.extend_from_slice(&decrypted);
    let expected_large: [u8; 32] = Sha256::digest(&verify_input).into();
    if msg_key != expected_large[8..24] {
        return Err(E2eError::DecryptionFailed);
    }

    if decrypted.len() < 4 {
        return Err(E2eError::DecryptionFailed);
    }

    let mut len_bytes = [0u8; 4];
    len_bytes.copy_from_slice(&decrypted[0..4]);
    let msg_len = u32::from_le_bytes(len_bytes) as usize;

    if 4 + msg_len > decrypted.len() {
        return Err(E2eError::DecryptionFailed);
    }

    // The message length is always a multiple of 4 (TL serialization).
    if !msg_len.is_multiple_of(4) {
        return Err(E2eError::DecryptionFailed);
    }

    let padding = decrypted.len() - 4 - msg_len;
    if !(MIN_PADDING..=MAX_PADDING).contains(&padding) {
        return Err(E2eError::DecryptionFailed);
    }

    Ok(decrypted[4..4 + msg_len].to_vec())
}

// Per-file AES-256-IGE encryption for `messages.sendEncryptedFile`.

/// Encrypt file bytes with a freshly generated AES-256-IGE key and IV.
///
/// Returns `(key, iv, ciphertext)`.
///
/// Key and IV are both 32 bytes. IGE uses the full 32-byte IV as `iv1 || iv2`,
/// the layout `ferogram_crypto::aes::ige_encrypt` expects. The plaintext is
/// padded to a multiple of 16 bytes with random bytes before encryption; the
/// receiver truncates to the `size` field of the `DecryptedMedia` TL.
pub fn file_encrypt(plaintext: &[u8]) -> ([u8; 32], [u8; 32], Vec<u8>) {
    let mut key = [0u8; 32];
    let mut iv = [0u8; 32];
    rand::rng().fill_bytes(&mut key);
    rand::rng().fill_bytes(&mut iv);

    let pad_len = (16 - (plaintext.len() % 16)) % 16;
    let mut buf = plaintext.to_vec();
    if pad_len > 0 {
        let mut tail = vec![0u8; pad_len];
        rand::rng().fill_bytes(&mut tail);
        buf.extend_from_slice(&tail);
    }

    ige_encrypt(&mut buf, &key, &iv);

    (key, iv, buf)
}

/// Decrypt file ciphertext with the key and iv from the inner TL media field.
///
/// `size` is the original plaintext length from the `DecryptedMedia` `size` field.
/// Decrypts with AES-256-IGE and truncates the result to `size`.
///
/// Fails with [`E2eError::DecryptionFailed`] if `key` or `iv` is shorter than
/// 32 bytes, the ciphertext is empty or not a multiple of 16 bytes, or `size`
/// exceeds the decrypted length.
pub fn file_decrypt(
    ciphertext: &[u8],
    key: &[u8],
    iv: &[u8],
    size: usize,
) -> Result<Vec<u8>, crate::error::E2eError> {
    if key.len() < 32 || iv.len() < 32 {
        return Err(crate::error::E2eError::DecryptionFailed);
    }
    if !ciphertext.len().is_multiple_of(16) || ciphertext.is_empty() {
        return Err(crate::error::E2eError::DecryptionFailed);
    }

    let key32: &[u8; 32] = key[..32]
        .try_into()
        .map_err(|_| crate::error::E2eError::DecryptionFailed)?;
    let iv32: &[u8; 32] = iv[..32]
        .try_into()
        .map_err(|_| crate::error::E2eError::DecryptionFailed)?;

    let mut buf = ciphertext.to_vec();
    ige_decrypt(&mut buf, key32, iv32);

    if size > buf.len() {
        return Err(crate::error::E2eError::DecryptionFailed);
    }
    buf.truncate(size);
    Ok(buf)
}

/// File key fingerprint, as defined by the secret chat spec.
///
/// ```text
/// digest      = md5(key || iv)
/// fingerprint = LE_i32(digest[0..4]) XOR LE_i32(digest[4..8])
/// ```
///
/// Used as `key_fingerprint` in `InputEncryptedFileUploaded` and
/// `InputEncryptedFileBigUploaded`. This is not the chat `auth_key` fingerprint.
pub fn file_key_fingerprint(key: &[u8; 32], iv: &[u8; 32]) -> i32 {
    let mut input = [0u8; 64];
    input[..32].copy_from_slice(key);
    input[32..].copy_from_slice(iv);
    let digest: [u8; 16] = md5::compute(input).into();
    let a = i32::from_le_bytes([digest[0], digest[1], digest[2], digest[3]]);
    let b = i32::from_le_bytes([digest[4], digest[5], digest[6], digest[7]]);
    a ^ b
}

/// Compute four indices into [`EMOJI_FINGERPRINT_TABLE`] from `auth_key`.
///
/// ```text
/// SHA1(auth_key)  ->  20 bytes
/// bytes[0..8]     ->  four little-endian u16 values
/// value % 333     ->  index into EMOJI_FINGERPRINT_TABLE
/// ```
///
/// Both sides of a chat derive the same four emojis from the shared key, so
/// users can compare them out of band to detect a man-in-the-middle.
pub fn emoji_fingerprint_indices(auth_key: &[u8; 256]) -> [usize; 4] {
    let hash: [u8; 20] = Sha1::digest(auth_key).into();
    [
        (u16::from_le_bytes([hash[0], hash[1]]) % 333) as usize,
        (u16::from_le_bytes([hash[2], hash[3]]) % 333) as usize,
        (u16::from_le_bytes([hash[4], hash[5]]) % 333) as usize,
        (u16::from_le_bytes([hash[6], hash[7]]) % 333) as usize,
    ]
}

/// The 333-emoji table that [`emoji_fingerprint_indices`] indexes into.
///
/// Taken from Telegram Desktop (`kFingerprintEmojis` in
/// `Telegram/SourceFiles/data/data_emoji_fingerprint.cpp`,
/// <https://github.com/telegramdesktop/tdesktop>). The order must not change.
pub const EMOJI_FINGERPRINT_TABLE: [&str; 333] = [
    "\u{1F601}",
    "\u{1F602}",
    "\u{1F603}",
    "\u{1F604}",
    "\u{1F605}",
    "\u{1F606}",
    "\u{1F609}",
    "\u{1F60A}",
    "\u{1F60B}",
    "\u{1F60C}",
    "\u{1F60D}",
    "\u{1F60E}",
    "\u{1F60F}",
    "\u{1F610}",
    "\u{1F612}",
    "\u{1F613}",
    "\u{1F614}",
    "\u{1F616}",
    "\u{1F618}",
    "\u{1F61A}",
    "\u{1F61C}",
    "\u{1F61D}",
    "\u{1F61E}",
    "\u{1F620}",
    "\u{1F621}",
    "\u{1F622}",
    "\u{1F623}",
    "\u{1F624}",
    "\u{1F625}",
    "\u{1F628}",
    "\u{1F629}",
    "\u{1F62A}",
    "\u{1F62B}",
    "\u{1F62D}",
    "\u{1F630}",
    "\u{1F631}",
    "\u{1F632}",
    "\u{1F633}",
    "\u{1F635}",
    "\u{1F637}",
    "\u{1F638}",
    "\u{1F639}",
    "\u{1F63A}",
    "\u{1F63B}",
    "\u{1F63C}",
    "\u{1F63D}",
    "\u{1F63E}",
    "\u{1F63F}",
    "\u{1F640}",
    "\u{1F645}",
    "\u{1F646}",
    "\u{1F647}",
    "\u{1F648}",
    "\u{1F649}",
    "\u{1F64A}",
    "\u{1F64B}",
    "\u{1F64C}",
    "\u{1F64D}",
    "\u{1F64E}",
    "\u{1F64F}",
    "\u{1F40C}",
    "\u{1F40D}",
    "\u{1F40E}",
    "\u{1F414}",
    "\u{1F417}",
    "\u{1F418}",
    "\u{1F419}",
    "\u{1F41A}",
    "\u{1F41B}",
    "\u{1F41C}",
    "\u{1F41D}",
    "\u{1F41E}",
    "\u{1F41F}",
    "\u{1F420}",
    "\u{1F421}",
    "\u{1F422}",
    "\u{1F423}",
    "\u{1F424}",
    "\u{1F425}",
    "\u{1F426}",
    "\u{1F427}",
    "\u{1F428}",
    "\u{1F429}",
    "\u{1F42B}",
    "\u{1F42C}",
    "\u{1F42D}",
    "\u{1F42E}",
    "\u{1F42F}",
    "\u{1F430}",
    "\u{1F431}",
    "\u{1F432}",
    "\u{1F433}",
    "\u{1F434}",
    "\u{1F435}",
    "\u{1F436}",
    "\u{1F437}",
    "\u{1F438}",
    "\u{1F439}",
    "\u{1F43A}",
    "\u{1F43B}",
    "\u{1F43C}",
    "\u{1F43D}",
    "\u{1F43E}",
    "\u{1F340}",
    "\u{1F341}",
    "\u{1F342}",
    "\u{1F343}",
    "\u{1F344}",
    "\u{1F345}",
    "\u{1F346}",
    "\u{1F347}",
    "\u{1F348}",
    "\u{1F349}",
    "\u{1F34A}",
    "\u{1F34B}",
    "\u{1F34C}",
    "\u{1F34D}",
    "\u{1F34E}",
    "\u{1F34F}",
    "\u{1F351}",
    "\u{1F352}",
    "\u{1F353}",
    "\u{1F354}",
    "\u{1F355}",
    "\u{1F356}",
    "\u{1F357}",
    "\u{1F358}",
    "\u{1F359}",
    "\u{1F35A}",
    "\u{1F35B}",
    "\u{1F35C}",
    "\u{1F35D}",
    "\u{1F35E}",
    "\u{1F35F}",
    "\u{1F360}",
    "\u{1F361}",
    "\u{1F362}",
    "\u{1F363}",
    "\u{1F364}",
    "\u{1F365}",
    "\u{1F366}",
    "\u{1F367}",
    "\u{1F368}",
    "\u{1F369}",
    "\u{1F36A}",
    "\u{1F36B}",
    "\u{1F36C}",
    "\u{1F36D}",
    "\u{1F36E}",
    "\u{1F36F}",
    "\u{1F370}",
    "\u{1F371}",
    "\u{1F372}",
    "\u{1F373}",
    "\u{1F374}",
    "\u{1F375}",
    "\u{1F376}",
    "\u{1F377}",
    "\u{1F378}",
    "\u{1F379}",
    "\u{1F37A}",
    "\u{1F37B}",
    "\u{1F380}",
    "\u{1F381}",
    "\u{1F382}",
    "\u{1F383}",
    "\u{1F384}",
    "\u{1F385}",
    "\u{1F386}",
    "\u{1F387}",
    "\u{1F388}",
    "\u{1F389}",
    "\u{1F38A}",
    "\u{1F38B}",
    "\u{1F38C}",
    "\u{1F38D}",
    "\u{1F38E}",
    "\u{1F38F}",
    "\u{1F390}",
    "\u{1F391}",
    "\u{1F392}",
    "\u{1F393}",
    "\u{1F3A0}",
    "\u{1F3A1}",
    "\u{1F3A2}",
    "\u{1F3A3}",
    "\u{1F3A4}",
    "\u{1F3A5}",
    "\u{1F3A6}",
    "\u{1F3A7}",
    "\u{1F3A8}",
    "\u{1F3A9}",
    "\u{1F3AA}",
    "\u{1F3AB}",
    "\u{1F3AC}",
    "\u{1F3AD}",
    "\u{1F3AE}",
    "\u{1F3AF}",
    "\u{1F3B0}",
    "\u{1F3B1}",
    "\u{1F3B2}",
    "\u{1F3B3}",
    "\u{1F3B4}",
    "\u{1F3B5}",
    "\u{1F3B6}",
    "\u{1F3B7}",
    "\u{1F3B8}",
    "\u{1F3B9}",
    "\u{1F3BA}",
    "\u{1F3BB}",
    "\u{1F3BC}",
    "\u{1F3BD}",
    "\u{1F3BE}",
    "\u{1F3BF}",
    "\u{1F3C0}",
    "\u{1F3C1}",
    "\u{1F3C2}",
    "\u{1F3C3}",
    "\u{1F3C4}",
    "\u{1F3C5}",
    "\u{1F3C6}",
    "\u{1F3C7}",
    "\u{1F3C8}",
    "\u{1F3C9}",
    "\u{1F3CA}",
    "\u{1F3E0}",
    "\u{1F3E1}",
    "\u{1F3E2}",
    "\u{1F3E3}",
    "\u{1F3E4}",
    "\u{1F3E5}",
    "\u{1F3E6}",
    "\u{1F3E7}",
    "\u{1F3E8}",
    "\u{1F3E9}",
    "\u{1F3EA}",
    "\u{1F3EB}",
    "\u{1F3EC}",
    "\u{1F3ED}",
    "\u{1F3EF}",
    "\u{1F3F0}",
    "\u{1F400}",
    "\u{1F401}",
    "\u{1F402}",
    "\u{1F403}",
    "\u{1F404}",
    "\u{1F405}",
    "\u{1F406}",
    "\u{1F407}",
    "\u{1F408}",
    "\u{1F409}",
    "\u{1F40A}",
    "\u{1F40B}",
    "\u{1F480}",
    "\u{1F484}",
    "\u{1F485}",
    "\u{1F486}",
    "\u{1F487}",
    "\u{1F488}",
    "\u{1F489}",
    "\u{1F48A}",
    "\u{1F48B}",
    "\u{1F48C}",
    "\u{1F48D}",
    "\u{1F48E}",
    "\u{1F48F}",
    "\u{1F490}",
    "\u{1F491}",
    "\u{1F492}",
    "\u{1F493}",
    "\u{1F494}",
    "\u{1F495}",
    "\u{1F496}",
    "\u{1F497}",
    "\u{1F498}",
    "\u{1F499}",
    "\u{1F49A}",
    "\u{1F49B}",
    "\u{1F49C}",
    "\u{1F49D}",
    "\u{1F49E}",
    "\u{1F49F}",
    "\u{1F4A0}",
    "\u{1F4A1}",
    "\u{1F4A2}",
    "\u{1F4A3}",
    "\u{1F4A4}",
    "\u{1F4A5}",
    "\u{1F4A6}",
    "\u{1F4A7}",
    "\u{1F4A8}",
    "\u{1F4A9}",
    "\u{1F4AA}",
    "\u{1F4AB}",
    "\u{1F4AC}",
    "\u{1F4AD}",
    "\u{1F4AE}",
    "\u{1F4AF}",
    "\u{1F4B0}",
    "\u{1F4B1}",
    "\u{1F4B2}",
    "\u{1F4B3}",
    "\u{1F4B4}",
    "\u{1F4B5}",
    "\u{1F4B6}",
    "\u{1F4B7}",
    "\u{1F4B8}",
    "\u{1F4B9}",
    "\u{1F4BA}",
    "\u{1F4BB}",
    "\u{1F4BC}",
    "\u{1F4BD}",
    "\u{1F4BE}",
    "\u{1F4BF}",
    "\u{1F4C0}",
    "\u{1F4C1}",
    "\u{1F4C2}",
    "\u{1F4C3}",
    "\u{1F4C4}",
    "\u{1F4C5}",
    "\u{1F4C6}",
    "\u{1F4C7}",
    "\u{1F4C8}",
    "\u{1F4C9}",
    "\u{1F4CA}",
    "\u{1F4CB}",
    "\u{1F4CC}",
    "\u{1F4CD}",
    "\u{1F4CE}",
    "\u{1F4CF}",
    "\u{1F4D0}",
    "\u{1F4D1}",
    "\u{1F4D2}",
];

#[cfg(test)]
mod tests {
    use super::*;

    fn test_key() -> [u8; 256] {
        let mut k = [0u8; 256];
        for (i, b) in k.iter_mut().enumerate() {
            *b = (i * 7 + 3) as u8;
        }
        k
    }

    #[test]
    fn message_round_trip_both_directions() {
        let key = test_key();
        let msg = b"hello secret chat, length divisible by four!".to_vec();
        assert_eq!(msg.len() % 4, 0);

        // Originator -> peer.
        let blob = encrypt_message(&key, &msg, true).unwrap();
        assert_eq!(decrypt_message(&key, &blob, false).unwrap(), msg);

        // Peer -> originator.
        let blob = encrypt_message(&key, &msg, false).unwrap();
        assert_eq!(decrypt_message(&key, &blob, true).unwrap(), msg);
    }

    #[test]
    fn wrong_direction_is_rejected() {
        let key = test_key();
        let blob = encrypt_message(&key, &[0u8; 16], true).unwrap();
        assert!(decrypt_message(&key, &blob, true).is_err());
    }

    #[test]
    fn tampering_is_rejected() {
        let key = test_key();
        let mut blob = encrypt_message(&key, &[1u8; 32], true).unwrap();
        let last = blob.len() - 1;
        blob[last] ^= 1;
        assert!(matches!(
            decrypt_message(&key, &blob, false),
            Err(E2eError::DecryptionFailed)
        ));
    }

    #[test]
    fn wrong_key_fingerprint_is_rejected() {
        let key = test_key();
        let mut other = key;
        other[0] ^= 0xff;
        let blob = encrypt_message(&key, &[1u8; 32], true).unwrap();
        assert!(matches!(
            decrypt_message(&other, &blob, false),
            Err(E2eError::KeyFingerprintMismatch)
        ));
    }

    #[test]
    fn short_blob_is_rejected() {
        let key = test_key();
        assert!(decrypt_message(&key, &[0u8; 23], false).is_err());
    }

    #[test]
    fn padding_length_is_in_range() {
        for len in 0..200 {
            let pad = padding_len(len);
            assert!((MIN_PADDING..=MAX_PADDING).contains(&pad));
            assert_eq!((len + pad) % 16, 0);
        }
    }

    #[test]
    fn file_round_trip() {
        let data = (0..1000u32).map(|i| i as u8).collect::<Vec<_>>();
        let (key, iv, ct) = file_encrypt(&data);
        assert_eq!(file_decrypt(&ct, &key, &iv, data.len()).unwrap(), data);
        assert!(file_decrypt(&ct, &key[..16], &iv, data.len()).is_err());
        assert!(file_decrypt(&ct, &key, &iv, ct.len() + 1).is_err());
    }

    #[test]
    fn emoji_indices_are_in_table() {
        for idx in emoji_fingerprint_indices(&test_key()) {
            assert!(idx < EMOJI_FINGERPRINT_TABLE.len());
        }
    }
}
