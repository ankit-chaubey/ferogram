// Copyright (c) Ankit Chaubey <ankitchaubey.dev@gmail.com>
// SPDX-License-Identifier: MIT OR Apache-2.0
//
// ferogram: async Telegram MTProto client in Rust
// https://github.com/ankit-chaubey/ferogram
//
// If you use or modify this code, keep this notice at the top of your file
// and include the LICENSE-MIT or LICENSE-APACHE file from this repository:
// https://github.com/ankit-chaubey/ferogram

use std::time::Duration;

pub trait ConnectionRestartPolicy: Send + Sync + 'static {
    fn restart_interval(&self) -> Option<Duration>;
}

pub struct NeverRestart;

impl ConnectionRestartPolicy for NeverRestart {
    fn restart_interval(&self) -> Option<Duration> {
        None
    }
}

pub struct FixedInterval {
    pub interval: Duration,
}

impl ConnectionRestartPolicy for FixedInterval {
    fn restart_interval(&self) -> Option<Duration> {
        Some(self.interval)
    }
}

/// Exponential backoff with jitter.
///
/// Delay formula: `clamp(base * 2^attempt, base, max) * (1 +/- jitter_factor)`.
/// Jitter is deterministic per-attempt using a simple hash of the attempt
/// count.
///
/// # Example
/// ```
/// let policy = ExponentialBackoff::default(); // 1s base, 60s max, 30% jitter
/// ```
pub struct ExponentialBackoff {
    /// Starting delay (e.g. `Duration::from_secs(1)`).
    pub base: Duration,
    /// Upper bound on delay (e.g. `Duration::from_secs(60)`).
    pub max: Duration,
    /// Jitter fraction in `[0.0, 1.0)`. 0.3 → ±30% of the computed delay.
    pub jitter_factor: f64,
    /// Current attempt counter; incremented each call to `restart_interval`.
    attempt: u32,
}

impl ExponentialBackoff {
    pub fn new(base: Duration, max: Duration, jitter_factor: f64) -> Self {
        Self {
            base,
            max,
            jitter_factor: jitter_factor.clamp(0.0, 0.99),
            attempt: 0,
        }
    }

    /// Reset the attempt counter (call after a successful connection).
    pub fn reset(&mut self) {
        self.attempt = 0;
    }
}

impl Default for ExponentialBackoff {
    /// 1 s base, 60 s max, 30 % jitter; suitable for most Telegram clients.
    fn default() -> Self {
        Self::new(Duration::from_secs(1), Duration::from_secs(60), 0.3)
    }
}

impl ConnectionRestartPolicy for ExponentialBackoff {
    fn restart_interval(&self) -> Option<Duration> {
        // 2^attempt, capped at 2^30 to avoid overflow.
        let factor = 1u64.checked_shl(self.attempt.min(30)).unwrap_or(u64::MAX);
        let base_ms = self.base.as_millis() as u64;
        let max_ms = self.max.as_millis() as u64;
        let delay_ms = (base_ms.saturating_mul(factor)).min(max_ms);

        // Deterministic jitter: hash the attempt number into a +/-jitter window.
        let jitter_range = (delay_ms as f64 * self.jitter_factor) as i64;
        // Simple deterministic hash: mix the attempt bits.
        let pseudo = {
            let mut h = self.attempt as u64 ^ 0x9e37_79b9_7f4a_7c15;
            h ^= h >> 30;
            h = h.wrapping_mul(0xbf58_476d_1ce4_e5b9);
            h ^= h >> 27;
            h = h.wrapping_mul(0x94d0_49bb_1331_11eb);
            h ^= h >> 31;
            h
        };
        // Map pseudo into [-jitter_range, +jitter_range].
        let jitter_ms = if jitter_range > 0 {
            (pseudo % (2 * jitter_range as u64)) as i64 - jitter_range
        } else {
            0
        };
        let final_ms = ((delay_ms as i64).saturating_add(jitter_ms)).max(0) as u64;
        Some(Duration::from_millis(final_ms))
    }
}
