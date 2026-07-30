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
