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

//! AES-IGE (Infinite Garble Extension): used by Telegram's MTProto.

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
