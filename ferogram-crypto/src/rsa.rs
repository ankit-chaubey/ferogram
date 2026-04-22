// Copyright (c) Ankit Chaubey <ankitchaubey.dev@gmail.com>
// SPDX-License-Identifier: MIT OR Apache-2.0
//
// ferogram: async Telegram MTProto client in Rust
// https://github.com/ankit-chaubey/ferogram
//
// If you use or modify this code, keep this notice at the top of your file
// and include the LICENSE-MIT or LICENSE-APACHE file from this repository:
// https://github.com/ankit-chaubey/ferogram

use crate::{aes, sha256};
use num_bigint::BigUint;

/// An RSA public key (n, e).
pub struct Key {
    n: BigUint,
    e: BigUint,
}

impl Key {
    /// Parse decimal `n` and `e` strings.
    pub fn new(n: &str, e: &str) -> Option<Self> {
        Some(Self {
            n: BigUint::parse_bytes(n.as_bytes(), 10)?,
            e: BigUint::parse_bytes(e.as_bytes(), 10)?,
        })
    }
}

fn increment(data: &mut [u8]) {
    let mut i = data.len() - 1;
    loop {
        let (n, overflow) = data[i].overflowing_add(1);
        data[i] = n;
        if overflow {
            i = i.checked_sub(1).unwrap_or(data.len() - 1);
        } else {
            break;
        }
    }
}

/// RSA-encrypt `data` using the MTProto RSA-PAD scheme.
///
/// `random_bytes` must be exactly 224 bytes of secure random data.
/// `data` must be ≤ 144 bytes.
pub fn encrypt_hashed(data: &[u8], key: &Key, random_bytes: &[u8; 224]) -> Vec<u8> {
    assert!(data.len() <= 144, "data too large for RSA-PAD");

    // data_with_padding: 192 bytes
    let mut data_with_padding = Vec::with_capacity(192);
    data_with_padding.extend_from_slice(data);
    data_with_padding.extend_from_slice(&random_bytes[..192 - data.len()]);

    // data_pad_reversed
    let data_pad_reversed: Vec<u8> = data_with_padding.iter().copied().rev().collect();

    let mut temp_key: [u8; 32] = random_bytes[192..].try_into().unwrap();

    let key_aes_encrypted = loop {
        // data_with_hash = data_pad_reversed + SHA256(temp_key + data_with_padding)
        let mut data_with_hash = Vec::with_capacity(224);
        data_with_hash.extend_from_slice(&data_pad_reversed);
        data_with_hash.extend_from_slice(&sha256!(&temp_key, &data_with_padding));

        aes::ige_encrypt(&mut data_with_hash, &temp_key, &[0u8; 32]);

        // temp_key_xor = temp_key XOR SHA256(aes_encrypted)
        let hash = sha256!(&data_with_hash);
        let mut xored = temp_key;
        for (a, b) in xored.iter_mut().zip(hash.iter()) {
            *a ^= b;
        }

        let mut candidate = Vec::with_capacity(256);
        candidate.extend_from_slice(&xored);
        candidate.extend_from_slice(&data_with_hash);

        if BigUint::from_bytes_be(&candidate) < key.n {
            break candidate;
        }
        increment(&mut temp_key);
    };

    let payload = BigUint::from_bytes_be(&key_aes_encrypted);
    let encrypted = payload.modpow(&key.e, &key.n);
    let mut block = encrypted.to_bytes_be();
    while block.len() < 256 {
        block.insert(0, 0);
    }
    block
}
