// Copyright (c) Ankit Chaubey <ankitchaubey.dev@gmail.com>
// SPDX-License-Identifier: MIT OR Apache-2.0
//
// ferogram: async Telegram MTProto client in Rust
// https://github.com/ankit-chaubey/ferogram
//
// If you use or modify this code, keep this notice at the top of your file
// and include the LICENSE-MIT or LICENSE-APACHE file from this repository:
// https://github.com/ankit-chaubey/ferogram

use hmac::Hmac;
use num_bigint::{BigInt, Sign};
use num_traits::ops::euclid::Euclid;
use sha2::{Digest, Sha256, Sha512};

fn sha256(parts: &[&[u8]]) -> [u8; 32] {
    let mut h = Sha256::new();
    for p in parts {
        h.update(p);
    }
    h.finalize().into()
}

fn sh(data: &[u8], salt: &[u8]) -> [u8; 32] {
    sha256(&[salt, data, salt])
}

fn ph1(password: &[u8], salt1: &[u8], salt2: &[u8]) -> [u8; 32] {
    sh(&sh(password, salt1), salt2)
}

fn ph2(password: &[u8], salt1: &[u8], salt2: &[u8]) -> [u8; 32] {
    let hash1 = ph1(password, salt1, salt2);
    let mut dk = [0u8; 64];
    pbkdf2::pbkdf2::<Hmac<Sha512>>(&hash1, salt1, 100_000, &mut dk).unwrap();
    sh(&dk, salt2)
}

fn pad256(data: &[u8]) -> [u8; 256] {
    let mut out = [0u8; 256];
    let start = 256usize.saturating_sub(data.len());
    out[start..].copy_from_slice(&data[data.len().saturating_sub(256)..]);
    out
}

fn xor32(a: &[u8; 32], b: &[u8; 32]) -> [u8; 32] {
    let mut out = [0u8; 32];
    for i in 0..32 {
        out[i] = a[i] ^ b[i];
    }
    out
}

/// Error returned when SRP parameter validation fails.
#[derive(Debug)]
pub enum SrpError {
    /// Server's g_b outside (1, p-1). RFC 5054 s2.6: exposes verifier offline.
    GbOutOfRange,
    /// Client's ephemeral g_a is outside (1, p-1). Degenerate exponentiation.
    GaOutOfRange,
}

impl std::fmt::Display for SrpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SrpError::GbOutOfRange => write!(f, "SRP: server g_b out of safe range"),
            SrpError::GaOutOfRange => write!(f, "SRP: client g_a out of safe range"),
        }
    }
}

/// Compute SRP `(M1, g_a)` for Telegram 2FA.
///
/// Returns `Err(SrpError)` if g_b or g_a is outside (1, p-1).
/// RFC 5054 s2.6: out-of-range g_b lets a server recover the verifier offline.
pub fn calculate_2fa(
    salt1: &[u8],
    salt2: &[u8],
    p: &[u8],
    g: i32,
    g_b: &[u8],
    a: &[u8],
    password: impl AsRef<[u8]>,
) -> Result<([u8; 32], [u8; 256]), SrpError> {
    let big_p = BigInt::from_bytes_be(Sign::Plus, p);
    let g_b = pad256(g_b);
    let a = pad256(a);
    let g_hash = pad256(&[g as u8]);

    let big_g_b = BigInt::from_bytes_be(Sign::Plus, &g_b);
    let big_g = BigInt::from(g as u32);
    let big_a = BigInt::from_bytes_be(Sign::Plus, &a);

    // Validate g_b in (1, p-1) as required by spec.
    {
        let one = BigInt::from(1u32);
        let p_minus_one = &big_p - &one;
        if big_g_b <= one || big_g_b >= p_minus_one {
            return Err(SrpError::GbOutOfRange);
        }
    }

    let k = sha256(&[p, &g_hash]);
    let big_k = BigInt::from_bytes_be(Sign::Plus, &k);

    let g_a = big_g.modpow(&big_a, &big_p);
    let g_a = pad256(&g_a.to_bytes_be().1);

    // Validate our own g_a in (1, p-1).
    {
        let big_g_a = BigInt::from_bytes_be(Sign::Plus, &g_a);
        let one = BigInt::from(1u32);
        let p_minus_one = &big_p - &one;
        if big_g_a <= one || big_g_a >= p_minus_one {
            return Err(SrpError::GaOutOfRange);
        }
    }

    let u = sha256(&[&g_a, &g_b]);
    let big_u = BigInt::from_bytes_be(Sign::Plus, &u);

    let x = ph2(password.as_ref(), salt1, salt2);
    let big_x = BigInt::from_bytes_be(Sign::Plus, &x);

    let big_v = big_g.modpow(&big_x, &big_p);
    let big_kv = (big_k * big_v) % &big_p;

    let big_t = (big_g_b - big_kv).rem_euclid(&big_p);

    let exp = big_a + big_u * big_x;
    let big_sa = big_t.modpow(&exp, &big_p);

    let k_a = sha256(&[&pad256(&big_sa.to_bytes_be().1)]);

    let h_p = sha256(&[p]);
    let h_g = sha256(&[&g_hash]);
    let p_xg = xor32(&h_p, &h_g);
    let m1 = sha256(&[
        &p_xg,
        &sha256(&[salt1]),
        &sha256(&[salt2]),
        &g_a,
        &g_b,
        &k_a,
    ]);

    Ok((m1, g_a))
}
