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
use std::time::Instant;

use ferogram_tl_types as tl;

/// Telegram sends `seq` equal to `0` when "it doesn't matter", so we use that value too.
pub(super) const NO_SEQ: i32 = 0;

/// `qts` of `0` means "ordering should be ignored" for that update.
pub(super) const NO_PTS: i32 = 0;

/// Sentinel `date` value when constructing dummy Updates containers.
pub(super) const NO_DATE: i32 = 0;

/// Wait up to 0.5 s before declaring a gap a real gap.
pub(super) const POSSIBLE_GAP_TIMEOUT: Duration = Duration::from_millis(500);

/// After how long without updates the client will proactively fetch updates.
///
/// Documentation recommends 15 minutes without updates.
pub(super) const NO_UPDATES_TIMEOUT: Duration = Duration::from_secs(15 * 60);

// Keys

/// A sortable message-box entry key.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Key {
    Common,
    Secondary,
    Channel(i64),
}

// Live entry

/// A single live entry inside [`MessageBoxes`].
#[derive(Debug)]
pub(super) struct LiveEntry {
    pub(super) key: Key,
    pub(super) pts: i32,
    /// Next instant when we forcibly fetch difference if no updates arrived by then.
    pub(super) deadline: Instant,
    /// If set, we detected a possible gap and are waiting to see if it resolves itself.
    pub(super) possible_gap: Option<PossibleGap>,
}

impl LiveEntry {
    pub(super) fn effective_deadline(&self) -> Instant {
        match &self.possible_gap {
            Some(gap) => gap.deadline.min(self.deadline),
            None => self.deadline,
        }
    }
}

// PossibleGap

#[derive(Debug)]
pub(super) struct PossibleGap {
    pub(super) deadline: Instant,
    /// Pending updates (those with a higher pts that are creating the gap).
    pub(super) updates: Vec<tl::enums::Update>,
}

// MessageBoxes (container)

/// All live message boxes.  The single authority for update-gap detection.
///
/// See <https://core.telegram.org/api/updates#message-related-event-sequences>.
#[derive(Debug)]
pub struct MessageBoxes {
    /// Live entries sorted by key.
    pub(super) entries: Vec<LiveEntry>,

    pub(super) date: i32,
    pub(super) seq: i32,

    /// Entries for which we must currently fetch difference.
    pub(super) getting_diff_for: Vec<Key>,

    /// Cached minimum deadline across all entries.
    pub(super) next_deadline: Instant,
}

// PtsInfo – per-update pts metadata

#[derive(Debug)]
pub(super) struct PtsInfo {
    pub(super) key: Key,
    pub(super) pts: i32,
    pub(super) count: i32,
}

// Gap error

/// Returned by [`MessageBoxes::process_updates`] when a gap is detected.
#[derive(Debug, PartialEq, Eq)]
pub struct Gap;

// UpdatesLike

/// Anything that should be treated as an update batch.
#[derive(Debug)]
pub enum UpdatesLike {
    /// Normal push update from the socket.
    Updates(Box<tl::enums::Updates>),
    /// The connection was closed; a gap may now exist.
    ConnectionClosed,
    /// A received update could not be parsed (unknown constructor, truncation).
    MalformedUpdates,
    /// RPC response for `messages.deleteMessages` / `messages.readHistory` etc.
    AffectedMessages(tl::types::messages::AffectedMessages),
    /// Same as above but channel-specific.
    AffectedChannelMessages {
        affected: tl::types::messages::AffectedMessages,
        channel_id: i64,
    },
}

// Public update state types (for persisting)

/// Per-channel pts snapshot.
#[derive(Debug, Clone)]
pub struct ChannelState {
    pub id: i64,
    pub pts: i32,
}

/// Full snapshot of the update state for session persistence.
#[derive(Debug, Clone, Default)]
pub struct UpdatesStateSnap {
    pub pts: i32,
    pub qts: i32,
    pub date: i32,
    pub seq: i32,
    pub channels: Vec<ChannelState>,
}

// Pair type

pub(super) type UpdateAndPeers = (
    Vec<tl::enums::Update>,
    Vec<tl::enums::User>,
    Vec<tl::enums::Chat>,
);
