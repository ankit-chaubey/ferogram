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

//! Unit tests for PtsState gap detection and recovery logic.
//! These tests use a pure synchronous PtsState (no async, no mocks needed)
//! so they run instantly without any network or tokio overhead.

#[cfg(test)]
mod tests {
    use crate::pts::{PtsCheckResult, PtsState};
    use std::collections::{HashMap, HashSet};

    fn fresh(pts: i32) -> PtsState {
        PtsState {
            pts,
            qts: 0,
            date: 0,
            seq: 0,
            channel_pts: HashMap::new(),
            last_update_at: None,
            getting_diff_for: HashSet::new(),
            getting_global_diff: false,
        }
    }

    // Global pts

    #[test]
    fn global_in_order() {
        let state = fresh(100);
        // pts=102, pts_count=2 → expected=102 → Ok
        assert_eq!(state.check_pts(102, 2), PtsCheckResult::Ok);
    }

    #[test]
    fn global_gap() {
        let state = fresh(100);
        // pts=105, pts_count=1 → expected=101, got=105 → Gap
        assert_eq!(
            state.check_pts(105, 1),
            PtsCheckResult::Gap {
                expected: 101,
                got: 105
            }
        );
    }

    #[test]
    fn global_duplicate() {
        let state = fresh(100);
        // pts=99, pts_count=1 → expected=101, got=99 → Duplicate
        assert_eq!(state.check_pts(99, 1), PtsCheckResult::Duplicate);
    }

    #[test]
    fn global_advance_monotone() {
        let mut state = fresh(50);
        state.advance(60);
        assert_eq!(state.pts, 60);
        // Attempting to regress pts must not lower it.
        state.advance(55);
        assert_eq!(state.pts, 60);
    }

    #[test]
    fn global_pts_zero_is_uninitialised() {
        // pts=0 means we haven't synced yet; any update should look like a gap.
        let state = fresh(0);
        // pts=5, count=5 → expected=5, got=5 → Ok (0+5=5)
        assert_eq!(state.check_pts(5, 5), PtsCheckResult::Ok);
    }

    // QTS

    #[test]
    fn qts_in_order() {
        let mut state = fresh(0);
        state.qts = 10;
        assert_eq!(state.check_qts(11, 1), PtsCheckResult::Ok);
    }

    #[test]
    fn qts_gap() {
        let mut state = fresh(0);
        state.qts = 10;
        assert_eq!(
            state.check_qts(15, 1),
            PtsCheckResult::Gap {
                expected: 11,
                got: 15
            }
        );
    }

    #[test]
    fn qts_duplicate() {
        let mut state = fresh(0);
        state.qts = 10;
        assert_eq!(state.check_qts(9, 1), PtsCheckResult::Duplicate);
    }

    // SEQ

    #[test]
    fn seq_uninitialised_accepts_any() {
        let mut state = fresh(0);
        state.seq = 0;
        // seq=0 → always Ok (uninitialised)
        assert_eq!(state.check_seq(5, 5), PtsCheckResult::Ok);
    }

    #[test]
    fn seq_in_order() {
        let mut state = fresh(0);
        state.seq = 5;
        // next expected = 6; seq_start=6 → Ok
        assert_eq!(state.check_seq(6, 6), PtsCheckResult::Ok);
    }

    #[test]
    fn seq_gap() {
        let mut state = fresh(0);
        state.seq = 5;
        // next expected = 6; seq_start=8 → Gap
        assert_eq!(
            state.check_seq(8, 8),
            PtsCheckResult::Gap {
                expected: 6,
                got: 8
            }
        );
    }

    #[test]
    fn seq_duplicate() {
        let mut state = fresh(0);
        state.seq = 5;
        // seq_start=4 → Duplicate
        assert_eq!(state.check_seq(5, 4), PtsCheckResult::Duplicate);
    }

    // Per-channel pts

    #[test]
    fn channel_unseen_accepts_any() {
        let state = fresh(0);
        // channel never seen → local=0 → always Ok
        assert_eq!(state.check_channel_pts(999, 10, 1), PtsCheckResult::Ok);
    }

    #[test]
    fn channel_in_order() {
        let mut state = fresh(0);
        state.channel_pts.insert(42, 100);
        assert_eq!(state.check_channel_pts(42, 101, 1), PtsCheckResult::Ok);
    }

    #[test]
    fn channel_gap() {
        let mut state = fresh(0);
        state.channel_pts.insert(42, 100);
        assert_eq!(
            state.check_channel_pts(42, 105, 1),
            PtsCheckResult::Gap {
                expected: 101,
                got: 105
            }
        );
    }

    #[test]
    fn channel_duplicate() {
        let mut state = fresh(0);
        state.channel_pts.insert(42, 100);
        assert_eq!(
            state.check_channel_pts(42, 99, 1),
            PtsCheckResult::Duplicate
        );
    }

    #[test]
    fn channel_advance_independent() {
        let mut state = fresh(100);
        state.channel_pts.insert(1, 50);
        state.channel_pts.insert(2, 200);
        state.advance_channel(1, 55);
        assert_eq!(state.channel_pts[&1], 55);
        assert_eq!(state.channel_pts[&2], 200); // unaffected
        assert_eq!(state.pts, 100); // global pts unaffected
    }

    #[test]
    fn channel_advance_monotone() {
        let mut state = fresh(0);
        state.channel_pts.insert(7, 80);
        state.advance_channel(7, 90);
        state.advance_channel(7, 85); // regress attempt: must not lower
        assert_eq!(state.channel_pts[&7], 90);
    }

    // getting_diff_for guard ()

    #[test]
    fn getting_diff_for_starts_empty() {
        let state = fresh(0);
        assert!(!state.getting_diff_for.contains(&42));
    }

    #[test]
    fn getting_diff_for_insert_remove() {
        let mut state = fresh(0);
        state.getting_diff_for.insert(42);
        assert!(state.getting_diff_for.contains(&42));
        state.getting_diff_for.remove(&42);
        assert!(!state.getting_diff_for.contains(&42));
    }

    #[test]
    fn getting_diff_for_independent_channels() {
        let mut state = fresh(0);
        state.getting_diff_for.insert(1);
        state.getting_diff_for.insert(2);
        assert!(state.getting_diff_for.contains(&1));
        assert!(state.getting_diff_for.contains(&2));
        state.getting_diff_for.remove(&1);
        assert!(!state.getting_diff_for.contains(&1));
        assert!(state.getting_diff_for.contains(&2)); // unaffected
    }

    // Deadline

    #[test]
    fn deadline_not_exceeded_when_uninitialised() {
        // last_update_at = None → not exceeded (we've never received anything)
        let state = fresh(0);
        assert!(!state.deadline_exceeded());
    }

    #[test]
    fn deadline_not_exceeded_when_recently_touched() {
        let mut state = fresh(0);
        state.touch();
        // Just touched → well within 15-minute window
        assert!(!state.deadline_exceeded());
    }
}
