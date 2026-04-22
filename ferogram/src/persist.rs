// Copyright (c) Ankit Chaubey <ankitchaubey.dev@gmail.com>
// SPDX-License-Identifier: MIT OR Apache-2.0
//
// ferogram: async Telegram MTProto client in Rust
// https://github.com/ankit-chaubey/ferogram
//
// If you use or modify this code, keep this notice at the top of your file
// and include the LICENSE-MIT or LICENSE-APACHE file from this repository:
// https://github.com/ankit-chaubey/ferogram

use std::collections::VecDeque;

/// Bounded ring-buffer dedup cache. Sits beneath the pts machinery as a
/// last-resort guard against edge-case duplicates (e.g. a live socket update
/// racing a diff replay that covers the same message).
///
/// Keyed by (canonical_peer_id, msg_id). Capacity-bounded: evicts the oldest
/// entry on overflow so memory stays O(1).
pub struct BoundedDedupeCache {
    entries: VecDeque<(i64, i32)>,
    capacity: usize,
    /// Total duplicates suppressed since creation.
    pub suppressed: u64,
}

impl BoundedDedupeCache {
    pub fn new(capacity: usize) -> Self {
        Self {
            entries: VecDeque::with_capacity(capacity),
            capacity,
            suppressed: 0,
        }
    }

    /// Returns true if (peer_id, msg_id) was already seen, meaning the update
    /// is a duplicate and should be dropped. Otherwise inserts and returns false.
    #[inline]
    pub fn check_and_insert(&mut self, peer_id: i64, msg_id: i32) -> bool {
        if self.entries.contains(&(peer_id, msg_id)) {
            self.suppressed += 1;
            tracing::debug!(
                "[ferogram/dedup] duplicate suppressed msg_id={msg_id} peer={peer_id} \
                 (total={})",
                self.suppressed
            );
            return true;
        }
        if self.entries.len() >= self.capacity {
            self.entries.pop_front();
        }
        self.entries.push_back((peer_id, msg_id));
        false
    }
}

impl Default for BoundedDedupeCache {
    fn default() -> Self {
        Self::new(512)
    }
}
