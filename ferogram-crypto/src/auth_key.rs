// Copyright (c) Ankit Chaubey <ankitchaubey.dev@gmail.com>
// SPDX-License-Identifier: MIT OR Apache-2.0
//
// ferogram: async Telegram MTProto client in Rust
// https://github.com/ankit-chaubey/ferogram
//
// If you use or modify this code, keep this notice at the top of your file
// and include the LICENSE-MIT or LICENSE-APACHE file from this repository:
// https://github.com/ankit-chaubey/ferogram

use crate::sha1;

/// A Telegram authorization key (256 bytes) plus pre-computed identifiers.
#[derive(Clone)]
pub struct AuthKey {
    pub(crate) data: [u8; 256],
    pub(crate) aux_hash: [u8; 8],
    pub(crate) key_id: [u8; 8],
}

impl AuthKey {
    /// Construct from raw 256-byte DH output.
    pub fn from_bytes(data: [u8; 256]) -> Self {
        let sha = sha1!(&data);
        let mut aux_hash = [0u8; 8];
        aux_hash.copy_from_slice(&sha[..8]);
        let mut key_id = [0u8; 8];
        key_id.copy_from_slice(&sha[12..20]);
        Self {
            data,
            aux_hash,
            key_id,
        }
    }

    /// Return the raw 256-byte representation.
    pub fn to_bytes(&self) -> [u8; 256] {
        self.data
    }

    /// The 8-byte key identifier (SHA-1(key)[12..20]).
    pub fn key_id(&self) -> [u8; 8] {
        self.key_id
    }

    /// Compute the new-nonce hash needed for `DhGenOk/Retry/Fail` verification.
    pub fn calc_new_nonce_hash(&self, new_nonce: &[u8; 32], number: u8) -> [u8; 16] {
        let data: Vec<u8> = new_nonce
            .iter()
            .copied()
            .chain([number])
            .chain(self.aux_hash.iter().copied())
            .collect();
        let sha = sha1!(&data);
        let mut out = [0u8; 16];
        out.copy_from_slice(&sha[4..]);
        out
    }
}

impl std::fmt::Debug for AuthKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "AuthKey(id={})", u64::from_le_bytes(self.key_id))
    }
}

impl PartialEq for AuthKey {
    fn eq(&self, other: &Self) -> bool {
        self.key_id == other.key_id
    }
}
