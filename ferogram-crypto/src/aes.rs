// Copyright (c) Ankit Chaubey <ankitchaubey.dev@gmail.com>
// SPDX-License-Identifier: MIT OR Apache-2.0
//
// ferogram: async Telegram MTProto client in Rust
// https://github.com/ankit-chaubey/ferogram
//
// If you use or modify this code, keep this notice at the top of your file
// and include the LICENSE-MIT or LICENSE-APACHE file from this repository:
// https://github.com/ankit-chaubey/ferogram

#![allow(deprecated)]

use aes::Aes256;
use aes::cipher::generic_array::GenericArray;
use aes::cipher::{BlockDecrypt, BlockEncrypt, KeyInit};

/// Encrypt `buffer` in-place with AES-256-IGE.
/// `buffer.len()` must be a multiple of 16.
pub fn ige_encrypt(buffer: &mut [u8], key: &[u8; 32], iv: &[u8; 32]) {
    assert_eq!(buffer.len() % 16, 0);
    let cipher = Aes256::new(GenericArray::from_slice(key));

    let mut iv1: [u8; 16] = iv[..16].try_into().unwrap();
    let mut iv2: [u8; 16] = iv[16..].try_into().unwrap();
    let mut next_iv2 = [0u8; 16];

    for block in buffer.chunks_mut(16) {
        next_iv2.copy_from_slice(block);
        for i in 0..16 {
            block[i] ^= iv1[i];
        }
        cipher.encrypt_block(GenericArray::from_mut_slice(block));
        for i in 0..16 {
            block[i] ^= iv2[i];
        }
        iv1.copy_from_slice(block);
        std::mem::swap(&mut iv2, &mut next_iv2);
    }
}

/// Encrypt/decrypt `buffer` in-place with AES-256-CTR (symmetric).
/// `key` = 32 bytes, `iv` = 16 bytes (full block = counter starting value).
pub fn ctr_crypt(buffer: &mut [u8], key: &[u8; 32], iv: &[u8; 16]) {
    use ctr::Ctr128BE;
    use ctr::cipher::{KeyIvInit, StreamCipher};
    let mut cipher =
        Ctr128BE::<Aes256>::new(GenericArray::from_slice(key), GenericArray::from_slice(iv));
    cipher.apply_keystream(buffer);
}

/// Return the effective AES-CTR IV for a CDN chunk starting at `byte_offset`.
/// Telegram CDN increments the counter (big-endian uint128) by `byte_offset / 16`.
pub fn ctr_iv_at_offset(base_iv: &[u8; 16], byte_offset: u64) -> [u8; 16] {
    let block_offset = byte_offset / 16;
    let iv_int = u128::from_be_bytes(*base_iv);
    iv_int.wrapping_add(block_offset as u128).to_be_bytes()
}

/// Decrypt `buffer` in-place with AES-256-IGE.
/// `buffer.len()` must be a multiple of 16.
pub fn ige_decrypt(buffer: &mut [u8], key: &[u8; 32], iv: &[u8; 32]) {
    assert_eq!(buffer.len() % 16, 0);
    let cipher = Aes256::new(GenericArray::from_slice(key));

    let mut iv1: [u8; 16] = iv[..16].try_into().unwrap();
    let mut iv2: [u8; 16] = iv[16..].try_into().unwrap();
    let mut next_iv1 = [0u8; 16];

    for block in buffer.chunks_mut(16) {
        next_iv1.copy_from_slice(block);
        for i in 0..16 {
            block[i] ^= iv2[i];
        }
        cipher.decrypt_block(GenericArray::from_mut_slice(block));
        for i in 0..16 {
            block[i] ^= iv1[i];
        }
        std::mem::swap(&mut iv1, &mut next_iv1);
        iv2.copy_from_slice(block);
    }
}
