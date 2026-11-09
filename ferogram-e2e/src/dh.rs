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

//! Diffie-Hellman helpers for the secret chat key exchange.

use num_bigint::BigUint;
use num_traits::{One, Zero};
use rand::Rng;
use sha1::{Digest, Sha1};

use crate::error::E2eError;

/// Secret chat layer this crate advertises to the peer via `notify_layer`.
pub const DEFAULT_LAYER: i32 = 101;

/// Cached DH configuration from `messages.getDhConfig`.
pub struct DhCacheEntry {
    /// Generator.
    pub g: u32,
    /// Safe prime, big-endian.
    pub p: Vec<u8>,
    /// Config version, sent back to the server to detect changes.
    pub version: i32,
}

/// Active DH state for one side of a key exchange.
/// Holds the private exponent and the computed public value (g^private mod p).
pub struct DhContext {
    /// Private exponent, 256 bytes.
    pub private_key: [u8; 256],
    /// g^private mod p, exactly 256 bytes (left-padded with zeros if needed).
    pub public_value: Vec<u8>,
    /// Generator.
    pub g: u32,
    /// Safe prime, big-endian.
    pub p: Vec<u8>,
}

fn to_256_bytes(n: &BigUint) -> [u8; 256] {
    let raw = n.to_bytes_be();
    let mut out = [0u8; 256];
    let start = 256usize.saturating_sub(raw.len());
    let src_start = raw.len().saturating_sub(256);
    out[start..].copy_from_slice(&raw[src_start..]);
    out
}

/// Validate a DH public value against the prime.
///
/// Applies to both `g_a` and `g_b`: the value must lie strictly between 1 and
/// `p - 1`, and at least 2^(2048-64) away from both ends of that range.
pub fn validate_dh_value(val: &BigUint, p: &BigUint) -> Result<(), E2eError> {
    if val <= &BigUint::one() {
        return Err(E2eError::DhValidationFailed);
    }
    if val >= &(p - BigUint::one()) {
        return Err(E2eError::DhValidationFailed);
    }
    let bound = BigUint::one() << 1984u32;
    if val < &bound {
        return Err(E2eError::DhValidationFailed);
    }
    if val > &(p - &bound) {
        return Err(E2eError::DhValidationFailed);
    }
    Ok(())
}

/// Compute key_fingerprint = last 8 bytes of SHA1(auth_key) as little-endian i64.
pub fn compute_fingerprint(auth_key: &[u8; 256]) -> i64 {
    let hash: [u8; 20] = Sha1::digest(auth_key).into();
    let mut b = [0u8; 8];
    b.copy_from_slice(&hash[12..20]);
    i64::from_le_bytes(b)
}

impl DhContext {
    /// Generate a fresh DH context: random private key, compute public value.
    pub fn generate(g: u32, p_bytes: &[u8]) -> Result<Self, E2eError> {
        let mut private_key = [0u8; 256];
        rand::rng().fill_bytes(&mut private_key);

        let p = BigUint::from_bytes_be(p_bytes);
        let a = BigUint::from_bytes_be(&private_key);
        let public = BigUint::from(g).modpow(&a, &p);

        validate_dh_value(&public, &p)?;

        Ok(Self {
            private_key,
            public_value: to_256_bytes(&public).to_vec(),
            g,
            p: p_bytes.to_vec(),
        })
    }

    /// Compute shared secret: remote_public^private mod p.
    /// Returns exactly 256 bytes (left-padded).
    pub fn compute_shared_secret(&self, remote_public: &[u8]) -> Result<[u8; 256], E2eError> {
        let p = BigUint::from_bytes_be(&self.p);
        let remote = BigUint::from_bytes_be(remote_public);

        validate_dh_value(&remote, &p)?;

        let private = BigUint::from_bytes_be(&self.private_key);
        let shared = remote.modpow(&private, &p);

        if shared.is_zero() || shared == BigUint::one() {
            return Err(E2eError::SharedSecretInvalid);
        }

        Ok(to_256_bytes(&shared))
    }
}

/// Compute the shared secret from raw private key bytes and the remote public value.
///
/// Used when completing a chat on the initiator side (User A), where only the
/// stored private exponent is available rather than a full [`DhContext`].
pub fn compute_shared_from_raw(
    private_key: &[u8; 256],
    remote_public: &[u8],
    p_bytes: &[u8],
) -> Result<[u8; 256], E2eError> {
    let p = BigUint::from_bytes_be(p_bytes);
    let remote = BigUint::from_bytes_be(remote_public);

    validate_dh_value(&remote, &p)?;

    let a = BigUint::from_bytes_be(private_key);
    let shared = remote.modpow(&a, &p);

    if shared.is_zero() || shared == BigUint::one() {
        return Err(E2eError::SharedSecretInvalid);
    }

    Ok(to_256_bytes(&shared))
}
