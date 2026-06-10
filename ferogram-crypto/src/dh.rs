// Copyright (c) Ankit Chaubey <ankitchaubey.dev@gmail.com>
//
// ferogram: async Telegram MTProto client in Rust
// https://github.com/ankit-chaubey/ferogram
//
// Licensed under either the MIT License or the Apache License 2.0.
// See the LICENSE-MIT or LICENSE-APACHE file in this repository:
// https://github.com/ankit-chaubey/ferogram
//
// Feel free to use, modify, and share this code.
// Please keep this notice when redistributing.

use num_bigint::BigUint;

/// Compute `base^exp mod modulus` over arbitrary-precision big-endian byte slices.
///
/// All three inputs are big-endian byte slices. Returns big-endian bytes,
/// zero-padded to nothing (caller pads if needed).
///
/// Used for MTProto DH key exchange: `g^b mod p` and `g_a^b mod p`.
pub fn dh_modpow(base: &[u8], exp: &[u8], modulus: &[u8]) -> Vec<u8> {
    BigUint::from_bytes_be(base)
        .modpow(
            &BigUint::from_bytes_be(exp),
            &BigUint::from_bytes_be(modulus),
        )
        .to_bytes_be()
}
