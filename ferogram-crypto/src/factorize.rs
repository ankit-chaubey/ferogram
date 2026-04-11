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

//! Pollard-rho (Brent variant) integer factorization: used for PQ step.

fn gcd(mut a: u128, mut b: u128) -> u128 {
    while b != 0 {
        let t = b;
        b = a % b;
        a = t;
    }
    a
}

fn modpow(mut n: u128, mut e: u128, m: u128) -> u128 {
    if m == 1 {
        return 0;
    }
    let mut result = 1;
    n %= m;
    while e > 0 {
        if e & 1 == 1 {
            result = result * n % m;
        }
        e >>= 1;
        n = n * n % m;
    }
    result
}

fn abs_sub(a: u128, b: u128) -> u128 {
    a.max(b) - a.min(b)
}

fn factorize_with(pq: u128, c: u128) -> (u64, u64) {
    if pq.is_multiple_of(2) {
        return (2, (pq / 2) as u64);
    }

    let mut y = 3 * (pq / 7);
    let m = 7 * (pq / 13);
    let mut g = 1u128;
    let mut r = 1u128;
    let mut q = 1u128;
    let mut x = 0u128;
    let mut ys = 0u128;

    while g == 1 {
        x = y;
        for _ in 0..r {
            y = (modpow(y, 2, pq) + c) % pq;
        }
        let mut k = 0;
        while k < r && g == 1 {
            ys = y;
            for _ in 0..m.min(r - k) {
                y = (modpow(y, 2, pq) + c) % pq;
                q = q * abs_sub(x, y) % pq;
            }
            g = gcd(q, pq);
            k += m;
        }
        r *= 2;
    }

    if g == pq {
        loop {
            ys = (modpow(ys, 2, pq) + c) % pq;
            g = gcd(abs_sub(x, ys), pq);
            if g > 1 {
                break;
            }
        }
    }

    let p = g as u64;
    let q = (pq / g) as u64;
    (p.min(q), p.max(q))
}

/// Factorize `pq` into two prime factors `(p, q)` where `p ≤ q`.
pub fn factorize(pq: u64) -> (u64, u64) {
    let n = pq as u128;
    for attempt in [43u128, 47, 53, 59, 61] {
        let c = attempt * (n / 103);
        let (p, q) = factorize_with(n, c);
        if p != 1 {
            return (p, q);
        }
    }
    panic!("factorize failed after fixed attempts");
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn t1() {
        assert_eq!(factorize(1470626929934143021), (1206429347, 1218991343));
    }
    #[test]
    fn t2() {
        assert_eq!(factorize(2363612107535801713), (1518968219, 1556064227));
    }
}
