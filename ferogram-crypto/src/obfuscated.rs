// Copyright (c) Ankit Chaubey <ankitchaubey.dev@gmail.com>
// SPDX-License-Identifier: MIT OR Apache-2.0
//
// ferogram: async Telegram MTProto client in Rust
// https://github.com/ankit-chaubey/ferogram
//
// If you use or modify this code, keep this notice at the top of your file
// and include the LICENSE-MIT or LICENSE-APACHE file from this repository:
// https://github.com/ankit-chaubey/ferogram

#[allow(deprecated)]
use aes::cipher::{KeyIvInit, StreamCipher, generic_array::GenericArray};

/// AES-256-CTR stream cipher pair for MTProto obfuscated transport.
pub struct ObfuscatedCipher {
    #[allow(deprecated)]
    rx: ctr::Ctr128BE<aes::Aes256>,
    #[allow(deprecated)]
    tx: ctr::Ctr128BE<aes::Aes256>,
}

impl ObfuscatedCipher {
    /// Build cipher state from the 64-byte random init buffer.
    #[allow(deprecated)]
    pub fn new(init: &[u8; 64]) -> Self {
        let rev: Vec<u8> = init.iter().copied().rev().collect();
        Self {
            rx: ctr::Ctr128BE::<aes::Aes256>::new(
                GenericArray::from_slice(&rev[8..40]),
                GenericArray::from_slice(&rev[40..56]),
            ),
            tx: ctr::Ctr128BE::<aes::Aes256>::new(
                GenericArray::from_slice(&init[8..40]),
                GenericArray::from_slice(&init[40..56]),
            ),
        }
    }

    /// Build cipher from explicit key/IV pairs (used when MTProxy secret
    /// mixing has already been applied externally via SHA-256).
    #[allow(deprecated)]
    pub fn from_keys(
        tx_key: &[u8; 32],
        tx_iv: &[u8; 16],
        rx_key: &[u8; 32],
        rx_iv: &[u8; 16],
    ) -> Self {
        Self {
            tx: ctr::Ctr128BE::<aes::Aes256>::new(
                GenericArray::from_slice(tx_key),
                GenericArray::from_slice(tx_iv),
            ),
            rx: ctr::Ctr128BE::<aes::Aes256>::new(
                GenericArray::from_slice(rx_key),
                GenericArray::from_slice(rx_iv),
            ),
        }
    }

    /// Encrypt outgoing bytes in-place (TX direction).
    pub fn encrypt(&mut self, buf: &mut [u8]) {
        self.tx.apply_keystream(buf);
    }

    /// Decrypt incoming bytes in-place (RX direction).
    pub fn decrypt(&mut self, buf: &mut [u8]) {
        self.rx.apply_keystream(buf);
    }
}
