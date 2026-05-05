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

//! Unit tests for the `message_box` state machine.
//!
//! Covers gap detection, duplicate suppression, channel independence,
//! `getDifference` / `getChannelDifference` flows, timeout behaviour,
//! affected-messages accounting, and connection-closed recovery.
//!
//! All tests use the thread-local fake clock from `defs::fake_clock`; no
//! `thread::sleep` anywhere.  Call `reset_time()` at the top of every test.

use std::time::Duration;

use ferogram_tl_types as tl;

use super::defs::fake_clock::{Instant, advance_time_by, reset_time};
use super::defs::{
    ChannelState, Gap, MessageBoxes, NO_DATE, NO_SEQ, NO_UPDATES_TIMEOUT, POSSIBLE_GAP_TIMEOUT,
    UpdateAndPeers, UpdatesLike, UpdatesStateSnap,
};
use super::next_updates_deadline;
use super::{Key, PrematureEndReason};

// Helpers
fn state(date: i32, seq: i32, pts: i32, qts: i32) -> tl::types::updates::State {
    tl::types::updates::State {
        pts,
        qts,
        date,
        seq,
        unread_count: 0,
    }
}

fn update(pts: i32) -> tl::enums::Update {
    tl::enums::Update::DeleteMessages(tl::types::UpdateDeleteMessages {
        messages: Vec::new(),
        pts,
        pts_count: 1,
    })
}

fn updates(date: i32, seq: i32, pts: i32) -> UpdatesLike {
    UpdatesLike::Updates(Box::new(tl::enums::Updates::Updates(tl::types::Updates {
        updates: vec![update(pts)],
        users: Vec::new(),
        chats: Vec::new(),
        date,
        seq,
    })))
}

fn channel_update(channel_id: i64, pts: i32) -> tl::enums::Update {
    tl::enums::Update::DeleteChannelMessages(tl::types::UpdateDeleteChannelMessages {
        channel_id,
        messages: Vec::new(),
        pts,
        pts_count: 1,
    })
}

fn channel_updates(channel_id: i64, date: i32, seq: i32, pts: i32) -> UpdatesLike {
    UpdatesLike::Updates(Box::new(tl::enums::Updates::Updates(tl::types::Updates {
        updates: vec![channel_update(channel_id, pts)],
        users: Vec::new(),
        chats: Vec::new(),
        date,
        seq,
    })))
}

/// Happy-path result with no users/chats.
fn ok(upd: Vec<tl::enums::Update>) -> Result<UpdateAndPeers, Gap> {
    Ok((upd, Vec::new(), Vec::new()))
}

fn ok_empty() -> Result<UpdateAndPeers, Gap> {
    ok(Vec::new())
}

fn get_diff(date: i32, pts: i32, qts: i32) -> tl::functions::updates::GetDifference {
    tl::functions::updates::GetDifference {
        pts,
        pts_limit: None,
        pts_total_limit: None,
        date: date.max(1),
        qts,
        qts_limit: None,
    }
}

#[allow(dead_code)]
fn get_chan_diff(id: i64, pts: i32) -> tl::functions::updates::GetChannelDifference {
    tl::functions::updates::GetChannelDifference {
        force: false,
        channel: tl::enums::InputChannel::InputChannel(tl::types::InputChannel {
            channel_id: id,
            access_hash: 0,
        }),
        filter: tl::enums::ChannelMessagesFilter::Empty,
        pts,
        limit: 0,
    }
}

// SESSION LOAD / STATE INIT
/// Loading an empty snapshot produces an empty, diff-free box.
#[test]
fn test_load_empty_state() {
    reset_time();
    let snap = UpdatesStateSnap::default();
    let mb = MessageBoxes::load(snap.clone());

    assert!(mb.is_empty());
    assert_eq!(mb.get_difference(), None);
    assert_eq!(mb.get_channel_difference(), None);
    assert_eq!(mb.session_state(), snap);
}

/// Loading a real snapshot immediately queues diff for every entry.
#[test]
fn test_load_state_queues_diff() {
    reset_time();
    let snap = UpdatesStateSnap {
        pts: 12,
        qts: 34,
        date: 56,
        seq: 78,
        channels: vec![ChannelState { id: 43, pts: 21 }],
    };
    let mb = MessageBoxes::load(snap.clone());

    assert!(!mb.is_empty());
    assert_eq!(mb.get_difference(), Some(get_diff(56, 12, 34)));

    let (id, diff) = mb.get_channel_difference().unwrap();
    assert_eq!(id, 43);
    assert_eq!(diff.pts, 21);

    assert_eq!(mb.session_state(), snap);
}

/// set_state() after new() must mark the box as non-empty but not queue a diff.
#[test]
fn test_set_state_after_new() {
    reset_time();
    let mut mb = MessageBoxes::new();
    assert!(mb.is_empty());

    mb.set_state(state(56, 78, 12, 34));

    assert!(!mb.is_empty());
    assert_eq!(mb.get_difference(), None);
    assert_eq!(mb.get_channel_difference(), None);
    assert_eq!(
        mb.session_state(),
        UpdatesStateSnap {
            pts: 12,
            qts: 34,
            date: 56,
            seq: 78,
            channels: Vec::new()
        }
    );
}

/// try_set_channel_state: first call wins; duplicates are ignored; sorted by id.
#[test]
fn test_try_set_channel_state_first_wins() {
    reset_time();
    let mut mb = MessageBoxes::new();

    mb.try_set_channel_state(98, 76);
    mb.try_set_channel_state(54, 32);
    mb.try_set_channel_state(98, 10); // ignored: 98 already registered

    assert_eq!(
        mb.session_state().channels,
        vec![
            ChannelState { id: 54, pts: 32 },
            ChannelState { id: 98, pts: 76 },
        ]
    );
}

// TIMEOUT / DEADLINE BEHAVIOUR
/// An empty box never queues a diff even after NO_UPDATES_TIMEOUT elapses.
#[test]
fn test_deadline_empty_box_never_diffs() {
    reset_time();
    let mut mb = MessageBoxes::new();

    let first_deadline = next_updates_deadline();
    assert_eq!(mb.check_deadlines(), first_deadline);
    assert_eq!(mb.get_difference(), None);
    assert_eq!(mb.get_channel_difference(), None);

    advance_time_by(NO_UPDATES_TIMEOUT / 2);
    assert_eq!(mb.check_deadlines(), first_deadline);
    assert_eq!(mb.get_difference(), None);

    advance_time_by(NO_UPDATES_TIMEOUT);
    assert_eq!(mb.check_deadlines(), next_updates_deadline());
    assert_eq!(mb.get_difference(), None);

    advance_time_by(NO_UPDATES_TIMEOUT + Duration::from_secs(1));
    assert_eq!(mb.check_deadlines(), next_updates_deadline());
    assert_eq!(mb.get_difference(), None);
}

/// A common entry times out and queues getDifference; applying DifferenceEmpty clears it.
#[test]
fn test_deadline_common_timeout_queues_diff() {
    reset_time();
    let mut mb = MessageBoxes::new();
    mb.set_state(state(56, 78, 12, 34));

    advance_time_by(NO_UPDATES_TIMEOUT);

    assert_eq!(mb.get_difference(), None);
    assert_eq!(mb.check_deadlines(), Instant::now());
    assert_eq!(mb.get_difference(), Some(get_diff(56, 12, 34)));

    mb.apply_difference(tl::types::updates::DifferenceEmpty { date: 90, seq: 91 }.into());

    assert_eq!(mb.get_difference(), None);
    assert_eq!(mb.check_deadlines(), next_updates_deadline());
    assert_eq!(
        mb.session_state(),
        UpdatesStateSnap {
            pts: 12,
            qts: 34,
            date: 90,
            seq: 91,
            channels: Vec::new()
        }
    );
}

/// A channel entry times out and queues getChannelDifference.
#[test]
fn test_deadline_channel_timeout_queues_diff() {
    reset_time();
    let mut mb = MessageBoxes::new();
    mb.try_set_channel_state(12, 34);

    advance_time_by(NO_UPDATES_TIMEOUT);

    assert_eq!(mb.get_channel_difference(), None);
    assert_eq!(mb.check_deadlines(), Instant::now());
    let (id, diff) = mb.get_channel_difference().unwrap();
    assert_eq!(id, 12);
    assert_eq!(diff.pts, 34);

    mb.apply_channel_difference(
        tl::types::updates::ChannelDifferenceEmpty {
            r#final: true,
            pts: 56,
            timeout: None,
        }
        .into(),
    );

    assert_eq!(mb.get_channel_difference(), None);
    assert_eq!(mb.check_deadlines(), next_updates_deadline());
    assert_eq!(
        mb.session_state().channels,
        vec![ChannelState { id: 12, pts: 56 }]
    );
}

/// end_channel_difference(TemporaryServerIssues) keeps the channel entry.
#[test]
fn test_end_channel_difference_temporary_issues() {
    reset_time();
    let mut mb = MessageBoxes::new();
    mb.try_set_channel_state(12, 34);

    advance_time_by(NO_UPDATES_TIMEOUT);
    assert_eq!(mb.check_deadlines(), Instant::now());
    assert!(mb.get_channel_difference().is_some());

    mb.end_channel_difference(PrematureEndReason::TemporaryServerIssues);

    assert!(mb.get_channel_difference().is_none());
    assert_eq!(
        mb.session_state().channels,
        vec![ChannelState { id: 12, pts: 34 }]
    );
}

/// end_channel_difference(Banned) removes the channel entry entirely.
#[test]
fn test_end_channel_difference_banned() {
    reset_time();
    let mut mb = MessageBoxes::new();
    mb.try_set_channel_state(12, 34);

    advance_time_by(NO_UPDATES_TIMEOUT);
    assert_eq!(mb.check_deadlines(), Instant::now());
    assert!(mb.get_channel_difference().is_some());

    mb.end_channel_difference(PrematureEndReason::Banned);

    assert!(mb.get_channel_difference().is_none());
    assert!(mb.session_state().channels.is_empty());
}

/// Multiple channels at different deadlines: only one diff in flight at a time.
#[test]
fn test_deadline_staggered_channels_one_at_a_time() {
    reset_time();
    let mut mb = MessageBoxes::new();

    // t=0: channel 11 registers (deadline at t+15min)
    mb.try_set_channel_state(11, 12);
    let first_deadline = next_updates_deadline();

    // t=10min: channels 21 and 31 register (deadline at t+25min)
    advance_time_by(2 * (NO_UPDATES_TIMEOUT / 3));
    mb.try_set_channel_state(21, 22);
    mb.try_set_channel_state(31, 32);
    let second_deadline = next_updates_deadline();

    // t=20min: channel 11 timed out
    advance_time_by(2 * (NO_UPDATES_TIMEOUT / 3));
    assert_eq!(mb.check_deadlines(), first_deadline);
    let (id, diff) = mb.get_channel_difference().unwrap();
    assert_eq!(id, 11);
    assert_eq!(diff.pts, 12);

    // t=30min: channels 21/31 should have timed out but diff still in flight
    advance_time_by(2 * (NO_UPDATES_TIMEOUT / 3));
    mb.end_channel_difference(PrematureEndReason::TemporaryServerIssues);
    assert_eq!(mb.get_channel_difference(), None);

    assert_eq!(mb.check_deadlines(), second_deadline);

    let (id, diff) = mb.get_channel_difference().unwrap();
    assert_eq!(id, 21);
    assert_eq!(diff.pts, 22);
    mb.end_channel_difference(PrematureEndReason::TemporaryServerIssues);

    let (id, diff) = mb.get_channel_difference().unwrap();
    assert_eq!(id, 31);
    assert_eq!(diff.pts, 32);
    mb.end_channel_difference(PrematureEndReason::TemporaryServerIssues);

    assert_eq!(mb.get_channel_difference(), None);
}

// DUPLICATE / ALREADY-APPLIED SUPPRESSION
/// All combinations of stale seq and/or stale pts are silently dropped.
#[test]
fn test_already_applied_combos() {
    reset_time();
    let mut mb = MessageBoxes::new();
    mb.set_state(state(12, 34, 56, 78));

    let stale: &[(i32, i32)] = &[
        (33, 57),     // seq < current
        (34, 57),     // seq == current
        (35, 55),     // seq ok, pts behind
        (35, 56),     // seq ok, pts == current
        (NO_SEQ, 55), // no seq, pts behind
        (NO_SEQ, 56), // no seq, pts == current
    ];
    for &(seq, pts) in stale {
        assert_eq!(
            mb.process_updates(updates(13, seq, pts)),
            ok_empty(),
            "seq={seq} pts={pts} should be a no-op"
        );
    }
}

/// Repeated seq values (≤ local) are dropped; next seq is accepted.
#[test]
fn test_dedup_seq() {
    reset_time();
    let mut mb = MessageBoxes::new();
    mb.set_state(state(1, 5, 10, 1));

    assert_eq!(
        mb.process_updates(updates(1, 5, 11)),
        ok_empty(),
        "seq == local must drop"
    );
    assert_eq!(
        mb.process_updates(updates(1, 4, 12)),
        ok_empty(),
        "seq < local must drop"
    );
    assert_eq!(
        mb.process_updates(updates(1, 6, 11)),
        ok(vec![update(11)]),
        "seq = local+1 must accept"
    );
}

/// A pts value already applied is dropped; next one succeeds.
#[test]
fn test_pts_dedup() {
    reset_time();
    let mut mb = MessageBoxes::new();
    mb.set_state(state(1, 1, 10, 1));

    assert_eq!(
        mb.process_updates(updates(NO_DATE, NO_SEQ, 11)),
        ok(vec![update(11)])
    );
    assert_eq!(
        mb.process_updates(updates(NO_DATE, NO_SEQ, 11)),
        ok_empty(),
        "same pts must drop"
    );
    assert_eq!(
        mb.process_updates(updates(NO_DATE, NO_SEQ, 10)),
        ok_empty(),
        "stale pts must drop"
    );
    assert_eq!(
        mb.process_updates(updates(NO_DATE, NO_SEQ, 12)),
        ok(vec![update(12)])
    );
}

// IN-ORDER / HAPPY PATH
/// Sequential in-order updates all dispatch immediately, no diff queued.
#[test]
fn test_pts_in_order_no_gap() {
    reset_time();
    let mut mb = MessageBoxes::new();
    mb.set_state(state(1, 1, 10, 1));

    assert_eq!(
        mb.process_updates(updates(NO_DATE, NO_SEQ, 11)),
        ok(vec![update(11)])
    );
    assert_eq!(
        mb.process_updates(updates(NO_DATE, NO_SEQ, 12)),
        ok(vec![update(12)])
    );
    assert_eq!(
        mb.process_updates(updates(NO_DATE, NO_SEQ, 13)),
        ok(vec![update(13)])
    );
    assert_eq!(mb.get_difference(), None);
}

/// In-order updates advance date/seq/pts in the session snapshot.
#[test]
fn test_in_order_updates_advance_state() {
    reset_time();
    let mut mb = MessageBoxes::new();
    mb.set_state(state(12, 34, 56, 78));

    assert_eq!(
        mb.process_updates(updates(NO_DATE, NO_SEQ, 57)),
        ok(vec![update(57)])
    );
    assert_eq!(
        mb.process_updates(updates(NO_DATE, 35, 58)),
        ok(vec![update(58)])
    );
    assert_eq!(
        mb.process_updates(updates(13, 36, 59)),
        ok(vec![update(59)])
    );
    assert_eq!(
        mb.process_updates(updates(14, NO_SEQ, 60)),
        ok(vec![update(60)])
    );

    let snap = mb.session_state();
    assert_eq!(snap.pts, 60);
    assert_eq!(snap.seq, 36);
    assert_eq!(snap.date, 14);
}

/// While getDifference is in flight, in-order updates still flow through.
#[test]
fn test_in_order_during_diff_in_flight() {
    reset_time();
    let mut mb = MessageBoxes::new();
    mb.set_state(state(12, 34, 56, 78));
    mb.try_begin_get_diff(Key::Common);

    assert!(mb.get_difference().is_some());
    assert_eq!(
        mb.process_updates(updates(NO_DATE, NO_SEQ, 57)),
        ok(vec![update(57)])
    );
}

// GAP DETECTION: COMMON (pts / seq)
/// A pts gap buffers the update; after POSSIBLE_GAP_TIMEOUT, getDifference queued.
#[test]
fn test_pts_gap_triggers_get_difference() {
    reset_time();
    let mut mb = MessageBoxes::new();
    mb.set_state(state(1, 1, 10, 1));

    assert_eq!(mb.process_updates(updates(NO_DATE, NO_SEQ, 12)), ok_empty());
    assert_eq!(mb.get_difference(), None);

    advance_time_by(POSSIBLE_GAP_TIMEOUT / 2);
    mb.check_deadlines();
    assert_eq!(mb.get_difference(), None, "still within gap window");

    advance_time_by(3 * (POSSIBLE_GAP_TIMEOUT / 2));
    mb.check_deadlines();
    assert!(
        mb.get_difference().is_some(),
        "getDifference must be queued after gap timeout"
    );
}

/// A seq gap immediately returns Err(Gap) and queues getDifference.
#[test]
fn test_seq_gap_triggers_get_difference_immediately() {
    reset_time();
    let mut mb = MessageBoxes::new();
    mb.set_state(state(12, 34, 56, 78));

    assert_eq!(mb.process_updates(updates(13, 36, 57)), Err(Gap));
    assert_eq!(mb.get_difference(), Some(get_diff(12, 56, 78)));
}

/// Possible gap resolves when fill-in update arrives before the deadline.
#[test]
fn test_possible_gap_resolves_before_timeout() {
    reset_time();
    let mut mb = MessageBoxes::new();
    mb.set_state(state(1, 1, 10, 1));

    assert_eq!(mb.process_updates(updates(NO_DATE, NO_SEQ, 12)), ok_empty());
    assert_eq!(mb.get_difference(), None);

    advance_time_by(POSSIBLE_GAP_TIMEOUT / 2);
    mb.check_deadlines();

    let (out, u, c) = mb.process_updates(updates(NO_DATE, NO_SEQ, 11)).unwrap();
    assert!(u.is_empty());
    assert!(c.is_empty());
    assert_eq!(out, vec![update(11), update(12)]);

    assert_eq!(mb.get_difference(), None);
}

/// Multiple out-of-order updates buffer and flush in pts order on resolution.
#[test]
fn test_possible_gap_multi_buffer_resolves() {
    reset_time();
    let mut mb = MessageBoxes::new();
    mb.set_state(state(12, 34, 56, 78));

    // pts 58, 60, 61 arrive before 57 and 59
    assert_eq!(mb.process_updates(updates(NO_DATE, NO_SEQ, 58)), ok_empty());
    advance_time_by(POSSIBLE_GAP_TIMEOUT / 4);
    mb.check_deadlines();
    assert_eq!(mb.process_updates(updates(NO_DATE, NO_SEQ, 60)), ok_empty());
    advance_time_by(POSSIBLE_GAP_TIMEOUT / 4);
    mb.check_deadlines();
    assert_eq!(mb.process_updates(updates(NO_DATE, NO_SEQ, 61)), ok_empty());

    advance_time_by(POSSIBLE_GAP_TIMEOUT / 4);
    mb.check_deadlines();
    let (out, _, _) = mb.process_updates(updates(NO_DATE, NO_SEQ, 57)).unwrap();
    assert_eq!(out, vec![update(57), update(58)]);

    let (out, _, _) = mb.process_updates(updates(NO_DATE, NO_SEQ, 59)).unwrap();
    assert_eq!(out, vec![update(59), update(60), update(61)]);

    assert_eq!(mb.get_difference(), None);
}

/// Trickling updates extend the possible-gap window but don't reset the
/// original deadline; the gap eventually fires.
#[test]
fn test_trickle_causes_gap_after_original_deadline() {
    reset_time();
    let gap_deadline = Instant::now() + POSSIBLE_GAP_TIMEOUT;
    let mut mb = MessageBoxes::new();
    mb.set_state(state(12, 34, 56, 78));

    assert_eq!(mb.process_updates(updates(NO_DATE, NO_SEQ, 58)), ok_empty());

    advance_time_by(2 * (POSSIBLE_GAP_TIMEOUT / 5));
    assert_eq!(mb.check_deadlines(), gap_deadline);
    assert_eq!(mb.process_updates(updates(NO_DATE, NO_SEQ, 59)), ok_empty());

    advance_time_by(2 * (POSSIBLE_GAP_TIMEOUT / 5));
    assert_eq!(mb.check_deadlines(), gap_deadline);
    assert_eq!(mb.process_updates(updates(NO_DATE, NO_SEQ, 60)), ok_empty());

    advance_time_by(2 * (POSSIBLE_GAP_TIMEOUT / 5));
    assert_eq!(mb.check_deadlines(), gap_deadline);
    assert!(mb.get_difference().is_some());
}

// GAP DETECTION: CHANNEL (pts)
/// A channel pts gap buffers the update; after timeout, getChannelDifference.
#[test]
fn test_channel_pts_gap_triggers_channel_diff() {
    reset_time();
    let mut mb = MessageBoxes::new();
    mb.try_set_channel_state(111, 100);

    assert_eq!(
        mb.process_updates(channel_updates(111, NO_DATE, NO_SEQ, 102)),
        ok_empty()
    );
    assert_eq!(mb.get_channel_difference(), None);

    advance_time_by(3 * (POSSIBLE_GAP_TIMEOUT / 2));
    mb.check_deadlines();

    let (id, diff) = mb.get_channel_difference().unwrap();
    assert_eq!(id, 111);
    assert_eq!(diff.pts, 100);
}

/// A gap in channel A must not affect in-order delivery for channel B.
#[test]
fn test_channel_state_independent() {
    reset_time();
    let (chan_a, chan_b) = (111_i64, 222_i64);
    let mut mb = MessageBoxes::new();
    mb.try_set_channel_state(chan_a, 100);
    mb.try_set_channel_state(chan_b, 100);

    assert_eq!(
        mb.process_updates(channel_updates(chan_a, NO_DATE, NO_SEQ, 102)),
        ok_empty()
    );

    assert_eq!(
        mb.process_updates(channel_updates(chan_b, NO_DATE, NO_SEQ, 101)),
        ok(vec![channel_update(chan_b, 101)])
    );
    assert_eq!(
        mb.process_updates(channel_updates(chan_b, NO_DATE, NO_SEQ, 102)),
        ok(vec![channel_update(chan_b, 102)])
    );

    assert_eq!(mb.get_channel_difference(), None, "gap not timed out yet");
}

/// First update for a brand-new channel is dispatched immediately.
#[test]
fn test_new_channel_first_update_dispatched() {
    reset_time();
    let mut mb = MessageBoxes::new();

    let result = mb.process_updates(channel_updates(12, 0, 0, 78));
    let (out, users, chats) = result.unwrap();
    assert!(users.is_empty());
    assert!(chats.is_empty());
    assert_eq!(out, vec![channel_update(12, 78)]);
}

// APPLY_DIFFERENCE
/// After a seq gap, applying DifferenceEmpty clears the diff and advances date/seq.
#[test]
fn test_get_difference_fills_gap() {
    reset_time();
    let mut mb = MessageBoxes::new();
    mb.set_state(state(100, 50, 10, 1));

    assert_eq!(mb.process_updates(updates(101, 52, 11)), Err(Gap));
    assert!(mb.get_difference().is_some());

    mb.apply_difference(tl::types::updates::DifferenceEmpty { date: 200, seq: 60 }.into());

    assert_eq!(mb.get_difference(), None);
    let snap = mb.session_state();
    assert_eq!(snap.date, 200);
    assert_eq!(snap.seq, 60);
    assert_eq!(snap.pts, 10);
}

/// apply_difference(DifferenceTooLong) resets pts to the server-provided value.
#[test]
fn test_apply_difference_too_long() {
    reset_time();
    let mut mb = MessageBoxes::new();
    mb.set_state(state(1, 1, 10, 0));
    mb.try_begin_get_diff(Key::Common);
    assert!(mb.get_difference().is_some());

    mb.apply_difference(tl::types::updates::DifferenceTooLong { pts: 999 }.into());

    assert_eq!(mb.get_difference(), None);
    assert_eq!(mb.session_state().pts, 999);
}

// SPECIAL UPDATESLIKE VARIANTS
/// ConnectionClosed returns Err(Gap) and queues getDifference when state exists.
#[test]
fn test_connection_closed_triggers_diff() {
    reset_time();
    let mut mb = MessageBoxes::new();
    mb.set_state(state(10, 5, 20, 0));

    assert_eq!(mb.process_updates(UpdatesLike::ConnectionClosed), Err(Gap));
    assert!(mb.get_difference().is_some());
}

/// ConnectionClosed on an empty box: Err(Gap) but no diff queued.
#[test]
fn test_connection_closed_empty_box_no_diff() {
    reset_time();
    let mut mb = MessageBoxes::new();

    assert_eq!(mb.process_updates(UpdatesLike::ConnectionClosed), Err(Gap));
    assert_eq!(mb.get_difference(), None);
}

/// MalformedUpdates returns Err(Gap) and queues getDifference.
#[test]
fn test_malformed_updates_triggers_diff() {
    reset_time();
    let mut mb = MessageBoxes::new();
    mb.set_state(state(10, 5, 20, 0));

    assert_eq!(mb.process_updates(UpdatesLike::MalformedUpdates), Err(Gap));
    assert!(mb.get_difference().is_some());
}

/// AffectedMessages updates the common pts correctly.
#[test]
fn test_affected_messages_updates_pts() {
    reset_time();
    let mut mb = MessageBoxes::new();
    mb.set_state(state(1, 0, 10, 0));

    let affected = UpdatesLike::AffectedMessages(tl::types::messages::AffectedMessages {
        pts: 11,
        pts_count: 1,
    });
    assert_eq!(mb.process_updates(affected), ok(vec![update(11)]));
    assert_eq!(mb.session_state().pts, 11);
}

/// AffectedChannelMessages updates the channel pts correctly.
#[test]
fn test_affected_channel_messages_updates_pts() {
    reset_time();
    let channel_id = 42_i64;
    let mut mb = MessageBoxes::new();
    mb.try_set_channel_state(channel_id, 10);

    let affected = UpdatesLike::AffectedChannelMessages {
        affected: tl::types::messages::AffectedMessages {
            pts: 11,
            pts_count: 1,
        },
        channel_id,
    };
    let (out, _, _) = mb.process_updates(affected).unwrap();
    assert_eq!(out, vec![channel_update(channel_id, 11)]);
    let chan = mb
        .session_state()
        .channels
        .into_iter()
        .find(|c| c.id == channel_id)
        .unwrap();
    assert_eq!(chan.pts, 11);
}

// ABORT / FORCE-RESET
/// abort_difference() clears all pending diff state without changing pts.
#[test]
fn test_abort_difference_clears_state() {
    reset_time();
    let mut mb = MessageBoxes::new();
    mb.set_state(state(1, 1, 10, 0));
    mb.try_begin_get_diff(Key::Common);

    assert!(mb.get_difference().is_some());
    mb.abort_difference();
    assert_eq!(mb.get_difference(), None);
    assert_eq!(mb.session_state().pts, 10);
}

/// force_reset_common_pts brings pts/qts/date/seq to exact given values.
#[test]
fn test_force_reset_common_pts() {
    reset_time();
    let mut mb = MessageBoxes::new();
    mb.set_state(state(1, 1, 10, 5));

    mb.force_reset_common_pts(999, 888, 777, 666);

    let snap = mb.session_state();
    assert_eq!(snap.pts, 999);
    assert_eq!(snap.qts, 888);
    assert_eq!(snap.date, 777);
    assert_eq!(snap.seq, 666);
}

// EDGE CASES
/// A channel pts gap must not corrupt common state; common updates continue.
#[test]
fn test_channel_gap_does_not_corrupt_common_pts() {
    reset_time();
    let mut mb = MessageBoxes::new();
    mb.set_state(state(1, 1, 10, 0));
    mb.try_set_channel_state(555, 50);

    assert_eq!(
        mb.process_updates(channel_updates(555, NO_DATE, NO_SEQ, 52)),
        ok_empty()
    );
    assert_eq!(
        mb.process_updates(updates(NO_DATE, NO_SEQ, 11)),
        ok(vec![update(11)])
    );
    assert_eq!(mb.session_state().pts, 11);
    assert_eq!(mb.get_difference(), None);
}

/// Two consecutive seq gaps must not double-push to getting_diff_for.
#[test]
fn test_double_seq_gap_no_duplicate_diff_entry() {
    reset_time();
    let mut mb = MessageBoxes::new();
    mb.set_state(state(1, 1, 10, 0));

    assert_eq!(mb.process_updates(updates(1, 3, 11)), Err(Gap));
    assert_eq!(mb.process_updates(updates(1, 5, 12)), Err(Gap));

    assert!(mb.get_difference().is_some());
    assert_eq!(mb.session_state().pts, 10, "pts must not advance on gap");
}

/// check_deadlines returns Instant::now() immediately when a diff is in flight.
#[test]
fn test_check_deadlines_returns_now_when_diff_in_flight() {
    reset_time();
    let mut mb = MessageBoxes::new();
    mb.set_state(state(1, 1, 10, 0));
    mb.try_begin_get_diff(Key::Common);

    let before = Instant::now();
    assert_eq!(mb.check_deadlines(), before);
}

/// A possible gap that times out must still leave common pts unchanged.
#[test]
fn test_pts_gap_does_not_advance_pts_before_apply() {
    reset_time();
    let mut mb = MessageBoxes::new();
    mb.set_state(state(1, 1, 10, 0));

    assert_eq!(mb.process_updates(updates(NO_DATE, NO_SEQ, 12)), ok_empty());
    advance_time_by(3 * (POSSIBLE_GAP_TIMEOUT / 2));
    mb.check_deadlines();

    assert_eq!(
        mb.session_state().pts,
        10,
        "pts must not advance until diff is applied"
    );
}
