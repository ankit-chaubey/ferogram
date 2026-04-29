// Copyright (c) Ankit Chaubey <ankitchaubey.dev@gmail.com>
// SPDX-License-Identifier: MIT OR Apache-2.0
//
// ferogram: async Telegram MTProto client in Rust
// https://github.com/ankit-chaubey/ferogram
//
// If you use or modify this code, keep this notice at the top of your file
// and include the LICENSE-MIT or LICENSE-APACHE file from this repository:
// https://github.com/ankit-chaubey/ferogram

// OVERVIEW
//
// This module implements the SINGLE UPDATE AUTHORITY for ferogram.
//
// Previously, ferogram scattered update-gap handling across:
//   - PtsState + PossibleGapBuffer (pts.rs)
//   - check_and_fill_gap / check_and_fill_channel_gap / check_and_fill_qts_gap (pts.rs)
//   - check_update_deadline (pts.rs)
//   - Hard-coded getDifference calls inside dispatch_updates (lib.rs)
//
// This caused overlapping recovery paths, races between concurrent diff calls,
// and PFS bind operations interfering with gap recovery.
//
//   - MessageBoxes is a PURE STATE MACHINE (no async, no RPC calls).
//   - ONE entry point: process_updates().
//   - The caller checks check_deadlines() to know when to force getDifference.
//   - The caller executes the RPC (get_difference / get_channel_difference)
//     and feeds the result back via apply_difference / apply_channel_difference.
//   - Gap buffering lives inside each LiveEntry (not in a separate global buffer).
//   - PFS / reconnect only interacts via UpdatesLike::ConnectionClosed.

mod adaptor;
pub mod defs;

use std::cmp::Ordering;
use std::time::Duration;
use std::time::Instant;

use defs::Key;
pub use defs::{ChannelState, Gap, MessageBoxes, UpdatesLike, UpdatesStateSnap};
use defs::{
    LiveEntry, NO_DATE, NO_PTS, NO_SEQ, POSSIBLE_GAP_TIMEOUT, PossibleGap, PtsInfo, UpdateAndPeers,
};
use ferogram_tl_types as tl;

// Helpers

fn next_updates_deadline() -> Instant {
    Instant::now() + defs::NO_UPDATES_TIMEOUT
}

fn update_sort_key(update: &tl::enums::Update) -> i32 {
    match PtsInfo::from_update(update) {
        Some(info) => info.pts - info.count,
        None => NO_PTS,
    }
}

// Creation and state management

impl Default for MessageBoxes {
    fn default() -> Self {
        Self::new()
    }
}

impl MessageBoxes {
    /// Create a new, empty [`MessageBoxes`] (no prior state).
    pub fn new() -> Self {
        tracing::trace!("[ferogram/msgbox] created new (no prior state)");
        Self {
            entries: Vec::new(),
            date: NO_DATE,
            seq: NO_SEQ,
            getting_diff_for: Vec::new(),
            next_deadline: next_updates_deadline(),
        }
    }

    /// Create a [`MessageBoxes`] from a previously-persisted snapshot.
    pub fn load(state: UpdatesStateSnap) -> Self {
        tracing::trace!("[ferogram/msgbox] loaded from state: {:?}", state);
        let mut entries = Vec::with_capacity(2 + state.channels.len());
        let mut getting_diff_for = Vec::with_capacity(2 + state.channels.len());
        let deadline = next_updates_deadline();

        if state.pts != NO_PTS {
            entries.push(LiveEntry {
                key: Key::Common,
                pts: state.pts,
                deadline,
                possible_gap: None,
            });
        }
        if state.qts != NO_PTS {
            entries.push(LiveEntry {
                key: Key::Secondary,
                pts: state.qts,
                deadline,
                possible_gap: None,
            });
        }
        entries.extend(state.channels.iter().map(|c| LiveEntry {
            key: Key::Channel(c.id),
            pts: c.pts,
            deadline,
            possible_gap: None,
        }));
        entries.sort_by_key(|e| e.key);

        // On load we need to reconcile; mark all entries as needing diff.
        getting_diff_for.extend(entries.iter().map(|e| e.key));

        Self {
            entries,
            date: state.date,
            seq: state.seq,
            getting_diff_for,
            next_deadline: deadline,
        }
    }

    fn entry(&self, key: Key) -> Option<&LiveEntry> {
        self.entries
            .binary_search_by_key(&key, |e| e.key)
            .map(|i| &self.entries[i])
            .ok()
    }

    fn update_entry(&mut self, key: Key, f: impl FnOnce(&mut LiveEntry)) -> bool {
        match self.entries.binary_search_by_key(&key, |e| e.key) {
            Ok(i) => {
                f(&mut self.entries[i]);
                true
            }
            Err(_) => false,
        }
    }

    fn force_update_entry(&mut self, mut entry: LiveEntry, f: impl FnOnce(&mut LiveEntry)) {
        match self.entries.binary_search_by_key(&entry.key, |e| e.key) {
            Ok(i) => f(&mut self.entries[i]),
            Err(i) => {
                f(&mut entry);
                self.entries.insert(i, entry);
            }
        }
    }

    fn set_entry(&mut self, entry: LiveEntry) {
        match self.entries.binary_search_by_key(&entry.key, |e| e.key) {
            Ok(i) => self.entries[i] = entry,
            Err(i) => self.entries.insert(i, entry),
        }
    }

    fn set_pts(&mut self, key: Key, pts: i32) {
        if !self.update_entry(key, |e| e.pts = pts) {
            self.set_entry(LiveEntry {
                key,
                pts,
                deadline: next_updates_deadline(),
                possible_gap: None,
            });
        }
    }

    fn pop_entry(&mut self, key: Key) -> Option<LiveEntry> {
        match self.entries.binary_search_by_key(&key, |e| e.key) {
            Ok(i) => Some(self.entries.remove(i)),
            Err(_) => None,
        }
    }

    fn reset_deadline(&mut self, key: Key, deadline: Instant) {
        let mut old_deadline = self.next_deadline;
        self.update_entry(key, |e| {
            old_deadline = e.deadline;
            e.deadline = deadline;
        });
        if self.next_deadline == old_deadline {
            self.next_deadline = self
                .entries
                .iter()
                .fold(deadline, |d, e| d.min(e.effective_deadline()));
        }
    }

    fn reset_timeout(&mut self, key: Key, timeout: Option<i32>) {
        self.reset_deadline(
            key,
            timeout
                .map(|t| Instant::now() + Duration::from_secs(t as _))
                .unwrap_or_else(next_updates_deadline),
        );
    }

    fn try_begin_get_diff(&mut self, key: Key) {
        if !self.getting_diff_for.contains(&key) {
            if self.update_entry(key, |e| e.possible_gap = None) {
                self.getting_diff_for.push(key);
            } else {
                tracing::info!(
                    "[ferogram/msgbox] tried begin_get_diff but no entry for {:?}",
                    key
                );
            }
        }
    }

    fn try_end_get_diff(&mut self, key: Key) {
        let i = match self.getting_diff_for.iter().position(|&k| k == key) {
            Some(i) => i,
            None => return,
        };
        self.getting_diff_for.remove(i);
        self.reset_deadline(key, next_updates_deadline());
        debug_assert!(
            self.entry(key).is_none_or(|e| e.possible_gap.is_none()),
            "gaps shouldn't be created while getting difference"
        );
    }

    /// Whether this box has any state yet.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Set state right after login (must only call when [`is_empty`] is true).
    pub fn set_state(&mut self, state: tl::types::updates::State) {
        debug_assert!(self.is_empty());
        let deadline = next_updates_deadline();
        self.set_entry(LiveEntry {
            key: Key::Common,
            pts: state.pts,
            deadline,
            possible_gap: None,
        });
        self.set_entry(LiveEntry {
            key: Key::Secondary,
            pts: state.qts,
            deadline,
            possible_gap: None,
        });
        self.date = state.date;
        self.seq = state.seq;
        self.next_deadline = deadline;
    }

    /// Record channel pts from `GetDialogs` - does nothing if the channel is already tracked.
    pub fn try_set_channel_state(&mut self, id: i64, pts: i32) {
        if self.entry(Key::Channel(id)).is_none() {
            self.set_entry(LiveEntry {
                key: Key::Channel(id),
                pts,
                deadline: next_updates_deadline(),
                possible_gap: None,
            });
        }
    }

    /// Current snapshot suitable for session persistence.
    pub fn session_state(&self) -> UpdatesStateSnap {
        UpdatesStateSnap {
            pts: self.entry(Key::Common).map(|e| e.pts).unwrap_or(NO_PTS),
            qts: self.entry(Key::Secondary).map(|e| e.pts).unwrap_or(NO_PTS),
            date: self.date,
            seq: self.seq,
            channels: self
                .entries
                .iter()
                .filter_map(|e| match e.key {
                    Key::Channel(id) => Some(ChannelState { id, pts: e.pts }),
                    _ => None,
                })
                .collect(),
        }
    }

    /// Return the next deadline instant.
    ///
    /// Call this in the select! loop to know when to wake up and check for
    /// expired gaps or timeouts.  When entries need diff, returns `now` so the
    /// caller immediately runs the diff.
    pub fn check_deadlines(&mut self) -> Instant {
        let now = Instant::now();

        if !self.getting_diff_for.is_empty() {
            return now; // fire the deadline arm immediately when diff is pending
        }

        if now >= self.next_deadline {
            // Mark entries whose deadlines have elapsed.
            self.getting_diff_for
                .extend(self.entries.iter().filter_map(|e| {
                    if now >= e.effective_deadline() {
                        tracing::debug!(
                            "[ferogram/msgbox] deadline met for {:?}; forcing diff",
                            e.key
                        );
                        Some(e.key)
                    } else {
                        None
                    }
                }));

            // Clear possible-gaps for entries we're about to diff.
            for i in 0..self.getting_diff_for.len() {
                self.update_entry(self.getting_diff_for[i], |e| e.possible_gap = None);
            }

            if self.getting_diff_for.is_empty() {
                self.next_deadline = next_updates_deadline();
            }
        }

        self.next_deadline
    }

    /// Return the `GetDifference` request to execute, if any.
    ///
    /// The caller is responsible for executing the RPC and calling
    /// [`apply_difference`] with the result.
    pub fn get_difference(&self) -> Option<tl::functions::updates::GetDifference> {
        for key in [Key::Common, Key::Secondary] {
            if self.getting_diff_for.contains(&key) {
                let pts = self
                    .entry(Key::Common)
                    .map(|e| e.pts)
                    .expect("Common entry must exist when diffing it");

                return Some(tl::functions::updates::GetDifference {
                    pts,
                    pts_limit: None,
                    pts_total_limit: None,
                    date: self.date.max(1),
                    qts: self.entry(Key::Secondary).map(|e| e.pts).unwrap_or(NO_PTS),
                    qts_limit: None,
                });
            }
        }
        None
    }

    /// Return the skeleton of a `GetChannelDifference` request, if any channel needs it.
    ///
    /// The caller must fill in `access_hash` and `limit` before executing.
    /// Call [`apply_channel_difference`] or [`end_channel_difference`] with the result.
    pub fn get_channel_difference(
        &self,
    ) -> Option<(i64, tl::functions::updates::GetChannelDifference)> {
        let (key, channel_id) = self.getting_diff_for.iter().find_map(|&k| match k {
            Key::Channel(id) => Some((k, id)),
            _ => None,
        })?;

        let pts = self
            .entry(key)
            .map(|e| e.pts)
            .expect("Channel entry must exist when diffing it");

        Some((
            channel_id,
            tl::functions::updates::GetChannelDifference {
                force: false,
                channel: tl::enums::InputChannel::InputChannel(tl::types::InputChannel {
                    channel_id,
                    access_hash: 0, // caller must fill this in
                }),
                filter: tl::enums::ChannelMessagesFilter::Empty,
                pts,
                limit: 0, // caller must fill this in
            },
        ))
    }
}

// Normal update flow

impl MessageBoxes {
    /// Process an update batch.  Returns `Ok((updates, users, chats))` or
    /// `Err(Gap)` when a gap has been detected and `get_difference` must be called.
    ///
    /// This is the **single entry point** for all incoming updates.
    pub fn process_updates(&mut self, updates: UpdatesLike) -> Result<UpdateAndPeers, Gap> {
        let deadline = next_updates_deadline();

        let tl::types::UpdatesCombined {
            date,
            seq_start,
            seq,
            mut updates,
            users,
            chats,
        } = match adaptor::adapt(updates) {
            Ok(combined) => combined,
            Err(Gap) => {
                self.try_begin_get_diff(Key::Common);
                return Err(Gap);
            }
        };

        let new_date = if date == NO_DATE { self.date } else { date };
        let new_seq = if seq == NO_SEQ { self.seq } else { seq };

        // Check seq for Updates / UpdatesCombined containers.
        if seq_start != NO_SEQ {
            match (self.seq + 1).cmp(&seq_start) {
                Ordering::Equal => {} // apply
                Ordering::Greater => {
                    tracing::debug!(
                        "[ferogram/msgbox] duplicate seq (local={}, remote={}), skipping",
                        self.seq,
                        seq_start
                    );
                    return Ok((Vec::new(), users, chats));
                }
                Ordering::Less => {
                    tracing::debug!(
                        "[ferogram/msgbox] seq gap (local={}, remote={})",
                        self.seq,
                        seq_start
                    );
                    self.try_begin_get_diff(Key::Common);
                    return Err(Gap);
                }
            }
        }

        // Sort so out-of-order updates within a container are applied in pts order.
        updates.sort_by_key(update_sort_key);

        let mut result: Vec<tl::enums::Update> = Vec::with_capacity(updates.len());
        let mut have_unresolved_gaps = false;

        for update in updates {
            // ChannelTooLong is handled specially.
            if let tl::enums::Update::ChannelTooLong(ref u) = update {
                let key = Key::Channel(u.channel_id);
                if let Some(pts) = u.pts {
                    self.set_entry(LiveEntry {
                        key,
                        pts,
                        deadline,
                        possible_gap: None,
                    });
                }
                self.try_begin_get_diff(key);
                continue;
            }

            let info = match PtsInfo::from_update(&update) {
                Some(info) => info,
                None => {
                    // No pts: can be applied in any order.
                    result.push(update);
                    continue;
                }
            };

            // While getting diff for this key, ignore matching updates (they
            // will arrive again via apply_difference / apply_channel_difference).
            if self.getting_diff_for.contains(&info.key) {
                tracing::debug!(
                    "[ferogram/msgbox] ignoring update for {:?} (diff in flight)",
                    info.key
                );
                self.reset_deadline(info.key, next_updates_deadline());
                result.push(update);
                continue;
            }

            let mut gap_deadline = None;

            self.force_update_entry(
                LiveEntry {
                    key: info.key,
                    pts: info.pts - info.count,
                    deadline,
                    possible_gap: None,
                },
                |entry| {
                    match (entry.pts + info.count).cmp(&info.pts) {
                        Ordering::Equal => {
                            // Normal in-order update.
                            entry.pts = info.pts;
                            entry.deadline = deadline;
                            result.push(update);
                        }
                        Ordering::Greater => {
                            // Duplicate.
                            tracing::debug!(
                                "[ferogram/msgbox] duplicate update for {:?} \
                                 (local={}, count={}, remote={})",
                                info.key,
                                entry.pts,
                                info.count,
                                info.pts
                            );
                        }
                        Ordering::Less => {
                            // Gap: buffer and wait.
                            tracing::info!(
                                "[ferogram/msgbox] gap for {:?} \
                                 (local={}, count={}, remote={})",
                                info.key,
                                entry.pts,
                                info.count,
                                info.pts
                            );
                            entry
                                .possible_gap
                                .get_or_insert_with(|| PossibleGap {
                                    deadline: Instant::now() + POSSIBLE_GAP_TIMEOUT,
                                    updates: Vec::new(),
                                })
                                .updates
                                .push(update.clone());
                        }
                    }

                    // Try to drain the possible_gap buffer now that pts advanced.
                    if let Some(mut gap) = entry.possible_gap.take() {
                        gap.updates.sort_by_key(|u| -update_sort_key(u));
                        while let Some(gap_update) = gap.updates.pop() {
                            let gap_info = PtsInfo::from_update(&gap_update)
                                .expect("only updates with pts may be buffered as gaps");
                            match (entry.pts + gap_info.count).cmp(&gap_info.pts) {
                                Ordering::Equal => {
                                    entry.pts = gap_info.pts;
                                    result.push(gap_update);
                                }
                                Ordering::Greater => {}
                                Ordering::Less => {
                                    gap.updates.push(gap_update);
                                    break;
                                }
                            }
                        }
                        if !gap.updates.is_empty() {
                            gap_deadline = Some(gap.deadline);
                            entry.possible_gap = Some(gap);
                            have_unresolved_gaps = true;
                        }
                    }
                },
            );

            self.next_deadline = self.next_deadline.min(gap_deadline.unwrap_or(deadline));
        }

        if !result.is_empty() && !have_unresolved_gaps {
            self.date = new_date;
            self.seq = new_seq;
        }

        Ok((result, users, chats))
    }
}

// Applying getDifference results

impl MessageBoxes {
    /// Apply the result of a `GetDifference` RPC call.
    pub fn apply_difference(
        &mut self,
        difference: tl::enums::updates::Difference,
    ) -> UpdateAndPeers {
        tracing::trace!("[ferogram/msgbox] applying account difference");
        if !self.getting_diff_for.contains(&Key::Common)
            && !self.getting_diff_for.contains(&Key::Secondary)
        {
            tracing::warn!(
                "[ferogram/msgbox] apply_difference called but no diff was pending \
                 (concurrent call already completed?); ignoring"
            );
            return (Vec::new(), Vec::new(), Vec::new());
        }

        let finish: bool;
        let result = match difference {
            tl::enums::updates::Difference::Empty(e) => {
                tracing::debug!(
                    "[ferogram/msgbox] difference empty (date={}, seq={})",
                    e.date,
                    e.seq
                );
                finish = true;
                self.date = e.date;
                self.seq = e.seq;
                (Vec::new(), Vec::new(), Vec::new())
            }
            tl::enums::updates::Difference::Difference(d) => {
                tracing::debug!("[ferogram/msgbox] full difference");
                finish = true;
                self.apply_difference_type(d)
            }
            tl::enums::updates::Difference::Slice(tl::types::updates::DifferenceSlice {
                new_messages,
                new_encrypted_messages,
                other_updates,
                chats,
                users,
                intermediate_state: state,
            }) => {
                tracing::debug!("[ferogram/msgbox] slice difference");
                finish = false;
                self.apply_difference_type(tl::types::updates::Difference {
                    new_messages,
                    new_encrypted_messages,
                    other_updates,
                    chats,
                    users,
                    state,
                })
            }
            tl::enums::updates::Difference::TooLong(d) => {
                tracing::warn!("[ferogram/msgbox] difference TooLong (pts={})", d.pts);
                finish = true;
                self.set_pts(Key::Common, d.pts);
                (Vec::new(), Vec::new(), Vec::new())
            }
        };

        if finish {
            self.try_end_get_diff(Key::Common);
            self.try_end_get_diff(Key::Secondary);
        }

        result
    }

    fn apply_difference_type(
        &mut self,
        tl::types::updates::Difference {
            new_messages,
            new_encrypted_messages,
            other_updates: updates,
            chats,
            users,
            state: tl::enums::updates::State::State(state),
        }: tl::types::updates::Difference,
    ) -> UpdateAndPeers {
        self.date = state.date;
        self.seq = state.seq;
        self.set_pts(Key::Common, state.pts);
        self.set_pts(Key::Secondary, state.qts);

        // Process other_updates through the normal path (handles ChannelTooLong etc).
        let us = UpdatesLike::Updates(Box::new(tl::enums::Updates::Updates(tl::types::Updates {
            updates,
            users,
            chats,
            date: NO_DATE,
            seq: NO_SEQ,
        })));
        let (mut result_updates, users, chats) = self
            .process_updates(us)
            .expect("gap detected while applying difference - should not happen");

        // Prepend new_messages as UpdateNewMessage with NO_PTS so they bypass pts checks.
        let msgs: Vec<tl::enums::Update> = new_messages
            .into_iter()
            .map(|msg| {
                tl::enums::Update::NewMessage(tl::types::UpdateNewMessage {
                    message: msg,
                    pts: NO_PTS,
                    pts_count: 0,
                })
            })
            .chain(new_encrypted_messages.into_iter().map(|msg| {
                tl::enums::Update::NewEncryptedMessage(tl::types::UpdateNewEncryptedMessage {
                    message: msg,
                    qts: NO_PTS,
                })
            }))
            .collect();

        result_updates.splice(0..0, msgs);
        (result_updates, users, chats)
    }
}

// Applying getChannelDifference results

impl MessageBoxes {
    /// Apply the result of a `GetChannelDifference` RPC call.
    pub fn apply_channel_difference(
        &mut self,
        difference: tl::enums::updates::ChannelDifference,
    ) -> UpdateAndPeers {
        let (key, channel_id) = self
            .getting_diff_for
            .iter()
            .find_map(|&k| match k {
                Key::Channel(id) => Some((k, id)),
                _ => None,
            })
            .expect("apply_channel_difference: no channel in getting_diff_for");

        tracing::trace!(
            "[ferogram/msgbox] applying channel {} difference",
            channel_id
        );
        self.update_entry(key, |e| e.possible_gap = None);

        let tl::types::updates::ChannelDifference {
            r#final,
            pts,
            timeout,
            new_messages,
            other_updates: updates,
            chats,
            users,
        } = adaptor::adapt_channel_difference(difference);

        if r#final {
            tracing::debug!(
                "[ferogram/msgbox] channel {} diff final; no longer getting diff",
                channel_id
            );
            self.try_end_get_diff(key);
        } else {
            tracing::debug!("[ferogram/msgbox] channel {} diff slice", channel_id);
        }

        self.set_pts(key, pts);

        let us = UpdatesLike::Updates(Box::new(tl::enums::Updates::Updates(tl::types::Updates {
            updates,
            users,
            chats,
            date: NO_DATE,
            seq: NO_SEQ,
        })));
        let (mut result_updates, users, chats) = self
            .process_updates(us)
            .expect("gap detected while applying channel difference");

        // Prepend new_messages.
        let msgs: Vec<tl::enums::Update> = new_messages
            .into_iter()
            .map(|msg| {
                tl::enums::Update::NewChannelMessage(tl::types::UpdateNewChannelMessage {
                    message: msg,
                    pts: NO_PTS,
                    pts_count: 0,
                })
            })
            .collect();
        result_updates.splice(0..0, msgs);

        self.reset_timeout(key, timeout);

        (result_updates, users, chats)
    }

    /// Mark a channel diff as ended prematurely (access error or ban).
    /// Abort a pending global difference after a parse or RPC failure.
    ///
    /// Clears `Key::Common` and `Key::Secondary` from `getting_diff_for` so
    /// that `check_deadlines()` stops returning `Instant::now()` and the
    /// deadline arm stops spawning hundreds of no-op tasks while the previous
    /// attempt's backoff sleep is still running.
    ///
    /// The pts counters are left unchanged; the next real update gap will
    /// re-queue getDifference automatically.
    pub fn abort_difference(&mut self) {
        for key in [Key::Common, Key::Secondary] {
            self.update_entry(key, |e| e.possible_gap = None);
            self.try_end_get_diff(key);
        }
        tracing::debug!(
            "[ferogram/msgbox] abort_difference: cleared Common+Secondary diff pending"
        );
    }

    /// Forcibly advance Common + Secondary pts to the server-reported values.
    ///
    /// Called after a getDifference parse failure (unknown TL constructor from a
    /// newer Telegram layer) so the stale pts gap does not re-trigger getDifference
    /// in an infinite loop.  Channel entries are left unchanged.
    pub fn force_reset_common_pts(&mut self, pts: i32, qts: i32, date: i32, seq: i32) {
        self.set_pts(Key::Common, pts);
        self.set_pts(Key::Secondary, qts);
        self.date = date;
        self.seq = seq;
        tracing::debug!(
            "[ferogram/msgbox] force_reset_common_pts: pts={pts}, qts={qts}, seq={seq}"
        );
    }

    pub fn end_channel_difference(&mut self, reason: PrematureEndReason) {
        let Some((key, channel_id)) = self.getting_diff_for.iter().find_map(|&k| match k {
            Key::Channel(id) => Some((k, id)),
            _ => None,
        }) else {
            tracing::warn!(
                "[ferogram/msgbox] end_channel_difference called but no channel pending \
                 (already ended? duplicate error path)"
            );
            return;
        };

        tracing::trace!(
            "[ferogram/msgbox] ending channel {} diff: {:?}",
            channel_id,
            reason
        );

        match reason {
            PrematureEndReason::TemporaryServerIssues => {
                self.update_entry(key, |e| e.possible_gap = None);
                self.try_end_get_diff(key);
            }
            PrematureEndReason::Banned => {
                self.update_entry(key, |e| e.possible_gap = None);
                self.try_end_get_diff(key);
                self.pop_entry(key);
            }
        }
    }
}

/// Reason for calling [`MessageBoxes::end_channel_difference`].
#[derive(Debug)]
pub enum PrematureEndReason {
    /// Temporary failure; keep the entry and retry later.
    TemporaryServerIssues,
    /// The account has been banned; remove the entry permanently.
    Banned,
}
