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

//! Update gap detection and recovery.
//!
//! Tracks `pts` / `qts` / `seq` / `date` plus per-channel pts, and
//! fills gaps via `updates.getDifference` (global) and
//! `updates.getChannelDifference` (per-channel).
//!
//! ## What "gap" means
//! Telegram guarantees updates arrive in order within a pts counter.
//! If `new_pts != local_pts + pts_count` there is a gap and we must
//! ask the server for the missed updates before processing this one.

use std::collections::{HashMap, HashSet};
use std::time::Instant;

use ferogram_tl_types as tl;
use ferogram_tl_types::{Cursor, Deserializable};

use crate::{Client, InvocationError, RpcError, attach_client_to_update, update};

/// How long to wait before declaring a pts jump a real gap (ms).
const POSSIBLE_GAP_DEADLINE_MS: u64 = 1_000;

/// Bots are allowed a much larger diff window (Telegram server-side limit).
const CHANNEL_DIFF_LIMIT_BOT: i32 = 100_000;

/// Buffers updates received during a possible-gap window so we don't fire
/// getDifference on every slightly out-of-order update.
#[derive(Default)]
pub struct PossibleGapBuffer {
    /// channel_id → (buffered_updates, window_start)
    channel: HashMap<i64, (Vec<update::Update>, Instant)>,
    /// Global buffered updates (non-channel pts gaps)
    global: Option<(Vec<update::Update>, Instant)>,
}

impl PossibleGapBuffer {
    pub fn new() -> Self {
        Self::default()
    }

    /// Buffer a global update during a possible-gap window.
    pub fn push_global(&mut self, upd: update::Update) {
        let entry = self
            .global
            .get_or_insert_with(|| (Vec::new(), Instant::now()));
        entry.0.push(upd);
    }

    /// Buffer a channel update during a possible-gap window.
    pub fn push_channel(&mut self, channel_id: i64, upd: update::Update) {
        let entry = self
            .channel
            .entry(channel_id)
            .or_insert_with(|| (Vec::new(), Instant::now()));
        entry.0.push(upd);
    }

    /// True if the global possible-gap deadline has elapsed.
    pub fn global_deadline_elapsed(&self) -> bool {
        self.global
            .as_ref()
            .map(|(_, t)| t.elapsed().as_millis() as u64 >= POSSIBLE_GAP_DEADLINE_MS)
            .unwrap_or(false)
    }

    /// True if a channel's possible-gap deadline has elapsed.
    pub fn channel_deadline_elapsed(&self, channel_id: i64) -> bool {
        self.channel
            .get(&channel_id)
            .map(|(_, t)| t.elapsed().as_millis() as u64 >= POSSIBLE_GAP_DEADLINE_MS)
            .unwrap_or(false)
    }

    /// True if the global buffer has any pending updates.
    pub fn has_global(&self) -> bool {
        self.global.is_some()
    }

    /// True if a channel buffer has pending updates.
    pub fn has_channel(&self, channel_id: i64) -> bool {
        self.channel.contains_key(&channel_id)
    }

    /// Start the global deadline timer without buffering an update.
    ///
    /// Called when a gap is detected but the triggering update carries no
    /// high-level `Update` value (e.g. `updateShortSentMessage` with `upd=None`).
    /// Without this, `global` stays `None` → `global_deadline_elapsed()` always
    /// returns `false` → `gap_tick` never fires getDifference for such gaps.
    pub fn touch_global_timer(&mut self) {
        self.global
            .get_or_insert_with(|| (Vec::new(), Instant::now()));
    }

    /// Drain global buffered updates.
    pub fn drain_global(&mut self) -> Vec<update::Update> {
        self.global.take().map(|(v, _)| v).unwrap_or_default()
    }

    /// Drain channel buffered updates.
    pub fn drain_channel(&mut self, channel_id: i64) -> Vec<update::Update> {
        self.channel
            .remove(&channel_id)
            .map(|(v, _)| v)
            .unwrap_or_default()
    }
}

/// Full MTProto sequence-number state, including per-channel counters.
///
/// All fields are `pub` so that `connect()` can restore them from the
/// persisted session without going through an artificial constructor.
#[derive(Debug, Clone, Default)]
pub struct PtsState {
    /// Main pts counter (messages, non-channel updates).
    pub pts: i32,
    /// Secondary counter for secret-chat updates.
    pub qts: i32,
    /// Date of the last received update (Unix timestamp).
    pub date: i32,
    /// Combined-container sequence number.
    pub seq: i32,
    /// Per-channel pts counters.  `channel_id → pts`.
    pub channel_pts: HashMap<i64, i32>,
    /// How many times getChannelDifference has been called per channel.
    /// tDesktop starts at limit=100, then raises to 1000 after the first
    /// successful response.  We track call count to implement the same ramp-up.
    pub channel_diff_calls: HashMap<i64, u32>,
    /// Timestamp of last received update for deadline-based gap detection.
    pub last_update_at: Option<Instant>,
    /// Channels currently awaiting a getChannelDifference response.
    /// If a channel is in this set, no new gap-fill task is spawned for it.
    pub getting_diff_for: HashSet<i64>,
    /// Guard against concurrent global getDifference calls.
    /// Without this, two simultaneous gap detections both spawn get_difference(),
    /// which double-processes updates and corrupts pts state.
    pub getting_global_diff: bool,
    /// When getting_global_diff was set to true.  Used by the stuck-diff watchdog
    /// in check_update_deadline: if the flag has been set for >30 s the RPC is
    /// assumed hung and the guard is reset so the next gap_tick can retry.
    pub getting_global_diff_since: Option<Instant>,
}

impl PtsState {
    pub fn from_server_state(s: &tl::types::updates::State) -> Self {
        Self {
            pts: s.pts,
            qts: s.qts,
            date: s.date,
            seq: s.seq,
            channel_pts: HashMap::new(),
            channel_diff_calls: HashMap::new(),
            last_update_at: Some(Instant::now()),
            getting_diff_for: HashSet::new(),
            getting_global_diff: false,
            getting_global_diff_since: None,
        }
    }

    /// Record that an update was received now (resets the deadline timer).
    pub fn touch(&mut self) {
        self.last_update_at = Some(Instant::now());
    }

    /// Returns true if no update has been received for > 15 minutes.
    pub fn deadline_exceeded(&self) -> bool {
        self.last_update_at
            .as_ref()
            .map(|t| t.elapsed().as_secs() > 15 * 60)
            .unwrap_or(false)
    }

    /// Check whether `new_pts` is in order given `pts_count` new updates.
    pub fn check_pts(&self, new_pts: i32, pts_count: i32) -> PtsCheckResult {
        let expected = self.pts + pts_count;
        if new_pts == expected {
            PtsCheckResult::Ok
        } else if new_pts > expected {
            PtsCheckResult::Gap {
                expected,
                got: new_pts,
            }
        } else {
            PtsCheckResult::Duplicate
        }
    }

    /// Check a qts value (secret chat updates).
    pub fn check_qts(&self, new_qts: i32, qts_count: i32) -> PtsCheckResult {
        let expected = self.qts + qts_count;
        if new_qts == expected {
            PtsCheckResult::Ok
        } else if new_qts > expected {
            PtsCheckResult::Gap {
                expected,
                got: new_qts,
            }
        } else {
            PtsCheckResult::Duplicate
        }
    }

    /// Check top-level seq for UpdatesCombined containers.
    pub fn check_seq(&self, _new_seq: i32, seq_start: i32) -> PtsCheckResult {
        if self.seq == 0 {
            return PtsCheckResult::Ok;
        } // uninitialised: accept
        let expected = self.seq + 1;
        if seq_start == expected {
            PtsCheckResult::Ok
        } else if seq_start > expected {
            PtsCheckResult::Gap {
                expected,
                got: seq_start,
            }
        } else {
            PtsCheckResult::Duplicate
        }
    }

    /// Check a per-channel pts value.
    pub fn check_channel_pts(
        &self,
        channel_id: i64,
        new_pts: i32,
        pts_count: i32,
    ) -> PtsCheckResult {
        let local = self.channel_pts.get(&channel_id).copied().unwrap_or(0);
        if local == 0 {
            return PtsCheckResult::Ok;
        }
        let expected = local + pts_count;
        if new_pts == expected {
            PtsCheckResult::Ok
        } else if new_pts > expected {
            PtsCheckResult::Gap {
                expected,
                got: new_pts,
            }
        } else {
            PtsCheckResult::Duplicate
        }
    }

    /// Advance the global pts.
    pub fn advance(&mut self, new_pts: i32) {
        if new_pts > self.pts {
            self.pts = new_pts;
        }
        self.touch();
    }

    /// Advance the qts.
    pub fn advance_qts(&mut self, new_qts: i32) {
        if new_qts > self.qts {
            self.qts = new_qts;
        }
        self.touch();
    }

    /// Advance seq.
    pub fn advance_seq(&mut self, new_seq: i32) {
        if new_seq > self.seq {
            self.seq = new_seq;
        }
    }

    /// Advance a per-channel pts.
    pub fn advance_channel(&mut self, channel_id: i64, new_pts: i32) {
        let entry = self.channel_pts.entry(channel_id).or_insert(0);
        if new_pts > *entry {
            *entry = new_pts;
        }
        self.touch();
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum PtsCheckResult {
    Ok,
    Gap { expected: i32, got: i32 },
    Duplicate,
}

impl Client {
    /// Fetch and replay any updates missed since the persisted pts.
    ///
    /// loops on `Difference::Slice` (partial response) until the server
    /// returns a final `Difference` or `Empty`
    /// never dropping a partial batch.  Previous code returned after one slice,
    /// silently losing all updates in subsequent slices.
    pub async fn get_difference(&self) -> Result<Vec<update::Update>, InvocationError> {
        // Atomically claim the in-flight slot.
        //
        // TOCTOU fix: the old code set getting_global_diff=true inside get_difference
        // but the external guard in check_and_fill_gap read the flag in a SEPARATE lock
        // acquisition.  With multi-threaded Tokio, N tasks could all read false, all
        // pass the external check, then all race into this function and all set the flag
        // to true.  Each concurrent call resets pts_state to the server state it received;
        // the last STALE write rolls pts back below where it actually is, which immediately
        // triggers a new gap → another burst of concurrent getDifference calls → cascade.
        //
        // Fix: check-and-set inside a SINGLE lock acquisition.  Only the first caller
        // proceeds; all others see true and return Ok(vec![]) immediately.
        {
            let mut s = self.inner.pts_state.lock();
            if s.getting_global_diff {
                return Ok(vec![]);
            }
            s.getting_global_diff = true;
            s.getting_global_diff_since = Some(Instant::now());
        }

        // Drain the initial-gap buffer before the RPC.
        //
        // possible_gap.global contains updates buffered during the possible-gap window
        // (before we decided to call getDiff).  The server response covers exactly
        // these pts values → discard the snapshot on success.
        // On RPC error, restore them so the next gap_tick can retry.
        //
        // Note: updates arriving DURING the RPC flight are now force-dispatched by
        // check_and_fill_gap and never accumulate in possible_gap,
        // so there is nothing extra to drain after the call returns.
        let pre_diff = self.inner.possible_gap.lock().drain_global();

        // Wrap the RPC in a hard 30-second timeout so a hung TCP connection
        // (half-open socket, unresponsive DC) cannot hold getting_global_diff=true
        // forever and freeze the bot indefinitely.
        let result = tokio::time::timeout(
            std::time::Duration::from_secs(30),
            self.get_difference_inner(),
        )
        .await
        .unwrap_or_else(|_| {
            tracing::warn!("[ferogram] getDifference RPC timed out after 30 s: will retry");
            Err(InvocationError::Io(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "getDifference timed out",
            )))
        });

        // Always clear the guard, even on error.
        {
            let mut s = self.inner.pts_state.lock();
            s.getting_global_diff = false;
            s.getting_global_diff_since = None;
        }

        match &result {
            Ok(_) => {
                // pre_diff is covered by the server response; discard it.
                // (Flight-time updates are force-dispatched, not buffered into possible_gap.)
            }
            Err(_) => {
                // Restore pre-existing items so the next gap_tick retry sees them.
                let mut gap = self.inner.possible_gap.lock();
                for u in pre_diff {
                    gap.push_global(u);
                }
            }
        }

        result
    }

    async fn get_difference_inner(&self) -> Result<Vec<update::Update>, InvocationError> {
        use ferogram_tl_types::{Cursor, Deserializable};

        let mut all_updates: Vec<update::Update> = Vec::new();

        // loop until the server sends a final (non-Slice) response.
        loop {
            let (pts, qts, date) = {
                let s = self.inner.pts_state.lock();
                (s.pts, s.qts, s.date)
            };

            if pts == 0 {
                self.sync_pts_state().await?;
                return Ok(all_updates);
            }

            tracing::debug!("[ferogram] getDifference (pts={pts}, qts={qts}, date={date}) …");

            let req = tl::functions::updates::GetDifference {
                pts,
                pts_limit: None,
                pts_total_limit: None,
                date,
                qts,
                qts_limit: None,
            };

            let body = self.rpc_call_raw_pub(&req).await?;
            let body = crate::maybe_gz_decompress(body)?;
            let mut cur = Cursor::from_slice(&body);
            let diff = match tl::enums::updates::Difference::deserialize(&mut cur) {
                Ok(d) => d,
                Err(e) => {
                    let cid = body
                        .get(..4)
                        .map(|b| u32::from_le_bytes(b.try_into().unwrap()))
                        .unwrap_or(0);
                    tracing::debug!(
                        "[ferogram] getDifference: unrecognised response {cid:#010x}, \
                         returning {} buffered updates ({e})",
                        all_updates.len()
                    );
                    return Ok(all_updates);
                }
            };

            match diff {
                tl::enums::updates::Difference::Empty(e) => {
                    let mut s = self.inner.pts_state.lock();
                    s.date = e.date;
                    s.seq = e.seq;
                    s.touch();
                    tracing::debug!("[ferogram] getDifference: empty (seq={})", e.seq);
                    return Ok(all_updates);
                }

                tl::enums::updates::Difference::Difference(d) => {
                    tracing::debug!(
                        "[ferogram] getDifference: {} messages, {} updates (final)",
                        d.new_messages.len(),
                        d.other_updates.len()
                    );
                    self.cache_users_slice_pub(&d.users).await;
                    self.cache_chats_slice_pub(&d.chats).await;
                    for msg in d.new_messages {
                        all_updates.push(update::Update::NewMessage(
                            update::IncomingMessage::from_raw(msg).with_client(self.clone()),
                        ));
                    }
                    for upd in d.other_updates {
                        all_updates.extend(update::from_single_update_pub(upd));
                    }
                    let tl::enums::updates::State::State(ns) = d.state;
                    let saved_channel_pts = {
                        let s = self.inner.pts_state.lock();
                        s.channel_pts.clone()
                    };
                    let mut new_state = PtsState::from_server_state(&ns);
                    // Preserve per-channel pts across the global reset.
                    for (cid, cpts) in saved_channel_pts {
                        new_state.channel_pts.entry(cid).or_insert(cpts);
                    }
                    // Preserve in-flight sets: we clear getting_global_diff ourselves.
                    new_state.getting_global_diff = true; // will be cleared by caller
                    {
                        let mut s = self.inner.pts_state.lock();
                        let getting_diff_for = std::mem::take(&mut s.getting_diff_for);
                        let since = s.getting_global_diff_since; // preserve watchdog timestamp
                        *s = new_state;
                        s.getting_diff_for = getting_diff_for;
                        s.getting_global_diff_since = since;
                    }
                    // Final response: stop looping.
                    return Ok(all_updates);
                }

                tl::enums::updates::Difference::Slice(d) => {
                    // server has more data: apply intermediate_state and
                    // continue looping.  Old code returned here, losing all updates
                    // in subsequent slices.
                    tracing::debug!(
                        "[ferogram] getDifference slice: {} messages, {} updates: continuing",
                        d.new_messages.len(),
                        d.other_updates.len()
                    );
                    self.cache_users_slice_pub(&d.users).await;
                    self.cache_chats_slice_pub(&d.chats).await;
                    for msg in d.new_messages {
                        all_updates.push(update::Update::NewMessage(
                            update::IncomingMessage::from_raw(msg).with_client(self.clone()),
                        ));
                    }
                    for upd in d.other_updates {
                        all_updates.extend(update::from_single_update_pub(upd));
                    }
                    let tl::enums::updates::State::State(ns) = d.intermediate_state;
                    let saved_channel_pts = {
                        let s = self.inner.pts_state.lock();
                        s.channel_pts.clone()
                    };
                    let mut new_state = PtsState::from_server_state(&ns);
                    for (cid, cpts) in saved_channel_pts {
                        new_state.channel_pts.entry(cid).or_insert(cpts);
                    }
                    new_state.getting_global_diff = true;
                    {
                        let mut s = self.inner.pts_state.lock();
                        let getting_diff_for = std::mem::take(&mut s.getting_diff_for);
                        let since = s.getting_global_diff_since; // preserve watchdog timestamp
                        *s = new_state;
                        s.getting_diff_for = getting_diff_for;
                        s.getting_global_diff_since = since;
                    }
                    // Loop: fetch the next slice.
                    continue;
                }

                tl::enums::updates::Difference::TooLong(d) => {
                    tracing::warn!(
                        "[ferogram] getDifference: TooLong (pts={}): re-syncing",
                        d.pts
                    );
                    self.inner.pts_state.lock().pts = d.pts;
                    self.sync_pts_state().await?;
                    return Ok(all_updates);
                }
            }
        }
    }

    // Per-channel getChannelDifference

    /// Fetch missed updates for a single channel.
    pub async fn get_channel_difference(
        &self,
        channel_id: i64,
    ) -> Result<Vec<update::Update>, InvocationError> {
        let local_pts = self
            .inner
            .pts_state
            .lock()
            .channel_pts
            .get(&channel_id)
            .copied()
            .unwrap_or(0);

        let access_hash = self.inner.peer_cache.channels.get(&channel_id).unwrap_or(0);

        // No access hash in cache → we can't call getChannelDifference.
        // Attempting GetChannels with access_hash=0 also returns CHANNEL_INVALID,
        // so skip the call entirely and let the caller handle it.
        if access_hash == 0 {
            tracing::debug!(
                "[ferogram] channel {channel_id}: access_hash not cached, \
                 cannot call getChannelDifference: caller will remove from tracking"
            );
            return Err(InvocationError::Rpc(RpcError {
                code: 400,
                name: "CHANNEL_INVALID".into(),
                value: None,
            }));
        }

        tracing::debug!("[ferogram] getChannelDifference channel_id={channel_id} pts={local_pts}");

        let channel = tl::enums::InputChannel::InputChannel(tl::types::InputChannel {
            channel_id,
            access_hash,
        });

        // tDesktop ramp-up: limit=100 on first call, 1000 on subsequent ones.
        // Bots always use the server-side maximum (100_000).
        let diff_limit = if self.inner.is_bot.load(std::sync::atomic::Ordering::Relaxed) {
            CHANNEL_DIFF_LIMIT_BOT
        } else {
            let call_count = self
                .inner
                .pts_state
                .lock()
                .channel_diff_calls
                .get(&channel_id)
                .copied()
                .unwrap_or(0);
            if call_count == 0 { 100 } else { 1000 }
        };

        let req = tl::functions::updates::GetChannelDifference {
            force: false,
            channel,
            filter: tl::enums::ChannelMessagesFilter::Empty,
            pts: local_pts.max(1),
            limit: diff_limit,
        };

        let body = match self.rpc_call_raw_pub(&req).await {
            Ok(b) => {
                // Successful call bump the per-channel counter so next call uses 1000.
                self.inner
                    .pts_state
                    .lock()
                    .channel_diff_calls
                    .entry(channel_id)
                    .and_modify(|c| *c = c.saturating_add(1))
                    .or_insert(1);
                b
            }
            Err(InvocationError::Rpc(ref e)) if e.name == "PERSISTENT_TIMESTAMP_OUTDATED" => {
                // treat as empty diff: retry next gap
                tracing::debug!("[ferogram] PERSISTENT_TIMESTAMP_OUTDATED: skipping diff");
                return Ok(vec![]);
            }
            Err(e) => return Err(e),
        };
        let body = crate::maybe_gz_decompress(body)?;
        let mut cur = Cursor::from_slice(&body);
        let diff = tl::enums::updates::ChannelDifference::deserialize(&mut cur)?;

        let mut updates = Vec::new();

        match diff {
            tl::enums::updates::ChannelDifference::Empty(e) => {
                tracing::debug!("[ferogram] getChannelDifference: empty (pts={})", e.pts);
                self.inner
                    .pts_state
                    .lock()
                    .advance_channel(channel_id, e.pts);
            }
            tl::enums::updates::ChannelDifference::ChannelDifference(d) => {
                tracing::debug!(
                    "[ferogram] getChannelDifference: {} messages, {} updates",
                    d.new_messages.len(),
                    d.other_updates.len()
                );
                self.cache_users_slice_pub(&d.users).await;
                self.cache_chats_slice_pub(&d.chats).await;
                for msg in d.new_messages {
                    updates.push(update::Update::NewMessage(
                        update::IncomingMessage::from_raw(msg).with_client(self.clone()),
                    ));
                }
                for upd in d.other_updates {
                    updates.extend(update::from_single_update_pub(upd));
                }
                self.inner
                    .pts_state
                    .lock()
                    .advance_channel(channel_id, d.pts);
            }
            tl::enums::updates::ChannelDifference::TooLong(d) => {
                tracing::warn!(
                    "[ferogram] getChannelDifference TooLong: replaying messages, resetting pts"
                );
                self.cache_users_slice_pub(&d.users).await;
                self.cache_chats_slice_pub(&d.chats).await;
                for msg in d.messages {
                    updates.push(update::Update::NewMessage(
                        update::IncomingMessage::from_raw(msg).with_client(self.clone()),
                    ));
                }
                self.inner.pts_state.lock().advance_channel(channel_id, 0);
            }
        }

        Ok(updates)
    }

    // Sync from server

    pub async fn sync_pts_state(&self) -> Result<(), InvocationError> {
        let body = self
            .rpc_call_raw_pub(&tl::functions::updates::GetState {})
            .await?;
        let mut cur = Cursor::from_slice(&body);
        let tl::enums::updates::State::State(s) = tl::enums::updates::State::deserialize(&mut cur)?;
        let mut state = self.inner.pts_state.lock();
        state.pts = s.pts;
        state.qts = s.qts;
        state.date = s.date;
        state.seq = s.seq;
        state.touch();
        tracing::debug!(
            "[ferogram] pts synced: pts={}, qts={}, seq={}",
            s.pts,
            s.qts,
            s.seq
        );
        Ok(())
    }
    /// Check global pts, buffer during possible-gap window, fetch diff if real gap.
    ///
    /// When a global getDifference is already in-flight (`getting_global_diff == true`),
    /// updates are **force-dispatched** immediately without pts tracking.
    /// This prevents the cascade freeze that buffering caused:
    ///   1. getDiff runs; flight-buffered updates pile up in `possible_gap`.
    ///   2. getDiff returns; `gap_tick` sees `has_global()=true` → another getDiff.
    ///   3. Each getDiff spawns another → bot freezes under a burst of messages.
    pub async fn check_and_fill_gap(
        &self,
        new_pts: i32,
        pts_count: i32,
        upd: Option<update::Update>,
    ) -> Result<Vec<update::Update>, InvocationError> {
        // getDiff in flight: force updates through without pts tracking.
        //
        // Force-dispatch: socket updates are sent through
        // when getting_diff_for contains the key; no buffering, no pts check.
        // Buffering caused a cascade of getDiff calls and a bot freeze under bursts.
        // Force-dispatch means these may duplicate what getDiff returns (same pts
        // range), which is acceptable: Telegram's spec explicitly states that socket
        // updates received during getDiff "should also have been retrieved through
        // getDifference". Application-layer deduplication by message_id handles doubles.
        // pts is NOT advanced here; getDiff sets it authoritatively when it returns.
        if self.inner.pts_state.lock().getting_global_diff {
            tracing::debug!("[ferogram] global diff in flight: force-applying pts={new_pts}");
            return Ok(upd.into_iter().collect());
        }

        let result = self.inner.pts_state.lock().check_pts(new_pts, pts_count);
        match result {
            PtsCheckResult::Ok => {
                // Advance pts and dispatch only this update.
                //
                // Do NOT blindly drain possible_gap here.
                //
                // Old behaviour: drain all buffered updates and return them together
                // with the Ok update.  This caused a second freeze:
                //
                //   1. pts=1021.  Burst arrives: 1024-1030 all gap → buffered.
                //   2. Update 1022 arrives → Ok → drain dispatches 1024-1030.
                //   3. pts advances only to 1022 (the Ok value), NOT to 1030.
                //   4. Bot sends replies → updateShortSentMessage pts=1031 →
                //      check_and_fill_gap: expected=1023, got=1031 → GAP.
                //   5. Cascade getDiff → duplicates → flood-wait → freeze.
                //
                // The fix is to re-check each buffered update in
                // order and advancing pts for each one (process_updates inner loop
                // over entry.possible_gap).  Layer's Update enum carries no pts
                // metadata, so we cannot do that ordered sequence check here.
                //
                // Correct equivalent: leave possible_gap alone.  The buffered
                // updates will be recovered by gap_tick → getDiff(new_pts), which
                // drains possible_gap into pre_diff, lets the server fill the
                // gap, and advances pts to the true server state; no stale pts,
                // no secondary cascade, no duplicates.
                self.inner.pts_state.lock().advance(new_pts);
                Ok(upd.into_iter().collect())
            }
            PtsCheckResult::Gap { expected, got } => {
                // Buffer the update; start the deadline timer regardless.
                //
                // Bug fix (touch_global_timer): when upd=None (e.g. the gap is
                // triggered by an updateShortSentMessage RPC response), nothing was
                // ever pushed to possible_gap.global, so global stayed None.
                // global_deadline_elapsed() returned false forever, gap_tick never
                // saw has_global()=true, and the gap was never resolved unless a
                // subsequent user message arrived.  touch_global_timer() starts the
                // 1-second deadline clock even without a buffered update.
                {
                    let mut gap = self.inner.possible_gap.lock();
                    if let Some(u) = upd {
                        gap.push_global(u);
                    } else {
                        gap.touch_global_timer();
                    }
                }
                let deadline_elapsed = self.inner.possible_gap.lock().global_deadline_elapsed();
                if deadline_elapsed {
                    tracing::warn!(
                        "[ferogram] global pts gap: expected {expected}, got {got}: getDifference"
                    );
                    // get_difference() is now atomic (check-and-set) and drains the
                    // possible_gap buffer internally on success, so callers must NOT
                    // drain before calling or splice the old buffer onto the results.
                    // Doing so caused every gap update to be dispatched twice, which
                    // triggered FLOOD_WAIT, blocked the handler, and froze the bot.
                    self.get_difference().await
                } else {
                    tracing::debug!(
                        "[ferogram] global pts gap: expected {expected}, got {got}: buffering (possible gap)"
                    );
                    Ok(vec![])
                }
            }
            PtsCheckResult::Duplicate => {
                tracing::debug!("[ferogram] global pts duplicate, discarding");
                Ok(vec![])
            }
        }
    }

    /// Check qts (secret chat updates) and fill gap if needed.
    pub async fn check_and_fill_qts_gap(
        &self,
        new_qts: i32,
        qts_count: i32,
    ) -> Result<Vec<update::Update>, InvocationError> {
        let result = self.inner.pts_state.lock().check_qts(new_qts, qts_count);
        match result {
            PtsCheckResult::Ok => {
                self.inner.pts_state.lock().advance_qts(new_qts);
                Ok(vec![])
            }
            PtsCheckResult::Gap { expected, got } => {
                tracing::warn!("[ferogram] qts gap: expected {expected}, got {got}: getDifference");
                self.get_difference().await
            }
            PtsCheckResult::Duplicate => Ok(vec![]),
        }
    }

    /// Check top-level seq and fill gap if needed.
    pub async fn check_and_fill_seq_gap(
        &self,
        new_seq: i32,
        seq_start: i32,
    ) -> Result<Vec<update::Update>, InvocationError> {
        let result = self.inner.pts_state.lock().check_seq(new_seq, seq_start);
        match result {
            PtsCheckResult::Ok => {
                self.inner.pts_state.lock().advance_seq(new_seq);
                Ok(vec![])
            }
            PtsCheckResult::Gap { expected, got } => {
                tracing::warn!("[ferogram] seq gap: expected {expected}, got {got}: getDifference");
                self.get_difference().await
            }
            PtsCheckResult::Duplicate => Ok(vec![]),
        }
    }

    /// Check a per-channel pts, fetch getChannelDifference if there is a gap.
    pub async fn check_and_fill_channel_gap(
        &self,
        channel_id: i64,
        new_pts: i32,
        pts_count: i32,
        upd: Option<update::Update>,
    ) -> Result<Vec<update::Update>, InvocationError> {
        // if a diff is already in flight for this channel, skip: prevents
        // 1 gap from spawning N concurrent getChannelDifference tasks.
        if self
            .inner
            .pts_state
            .lock()
            .getting_diff_for
            .contains(&channel_id)
        {
            tracing::debug!("[ferogram] channel {channel_id} diff already in flight, skipping");
            if let Some(u) = upd {
                self.inner.possible_gap.lock().push_channel(channel_id, u);
            }
            return Ok(vec![]);
        }

        let result = self
            .inner
            .pts_state
            .lock()
            .check_channel_pts(channel_id, new_pts, pts_count);
        match result {
            PtsCheckResult::Ok => {
                let mut buffered = self.inner.possible_gap.lock().drain_channel(channel_id);
                self.inner
                    .pts_state
                    .lock()
                    .advance_channel(channel_id, new_pts);
                if let Some(u) = upd {
                    buffered.push(u);
                }
                Ok(buffered)
            }
            PtsCheckResult::Gap { expected, got } => {
                if let Some(u) = upd {
                    self.inner.possible_gap.lock().push_channel(channel_id, u);
                }
                let deadline_elapsed = self
                    .inner
                    .possible_gap
                    .lock()
                    .channel_deadline_elapsed(channel_id);
                if deadline_elapsed {
                    tracing::warn!(
                        "[ferogram] channel {channel_id} pts gap: expected {expected}, got {got}: getChannelDifference"
                    );
                    // mark this channel as having a diff in flight.
                    self.inner
                        .pts_state
                        .lock()
                        .getting_diff_for
                        .insert(channel_id);
                    let buffered = self.inner.possible_gap.lock().drain_channel(channel_id);
                    match self.get_channel_difference(channel_id).await {
                        Ok(mut diff_updates) => {
                            // diff complete, allow future gaps to be handled.
                            self.inner
                                .pts_state
                                .lock()
                                .getting_diff_for
                                .remove(&channel_id);
                            diff_updates.splice(0..0, buffered);
                            Ok(diff_updates)
                        }
                        // Permanent access errors: remove the channel from pts tracking
                        // entirely (. The next update for this
                        // channel will have local=0 → PtsCheckResult::Ok, advancing pts
                        // without any gap fill. This breaks the infinite gap→CHANNEL_INVALID
                        // loop that happened when advance_channel kept the stale pts alive.
                        //
                        // Common causes:
                        // - access_hash not in peer cache (update arrived via updateShort
                        // which carries no chats list)
                        // - bot was kicked / channel deleted
                        Err(InvocationError::Rpc(ref e))
                            if e.name == "CHANNEL_INVALID"
                                || e.name == "CHANNEL_PRIVATE"
                                || e.name == "CHANNEL_NOT_MODIFIED" =>
                        {
                            tracing::debug!(
                                "[ferogram] channel {channel_id}: {}: removing from pts tracking \
                                 (next update treated as first-seen, no gap fill)",
                                e.name
                            );
                            {
                                let mut s = self.inner.pts_state.lock();
                                s.getting_diff_for.remove(&channel_id);
                                s.channel_pts.remove(&channel_id); // ←  fix: delete, not advance
                            }
                            Ok(buffered)
                        }
                        Err(InvocationError::Deserialize(ref msg)) => {
                            // Unrecognised constructor or parse failure: treat same as
                            // CHANNEL_INVALID: remove from tracking so we don't loop.
                            tracing::debug!(
                                "[ferogram] channel {channel_id}: deserialize error ({msg}): \
                                 removing from pts tracking"
                            );
                            {
                                let mut s = self.inner.pts_state.lock();
                                s.getting_diff_for.remove(&channel_id);
                                s.channel_pts.remove(&channel_id);
                            }
                            Ok(buffered)
                        }
                        Err(e) => {
                            // also clear on unexpected errors so we don't get stuck.
                            self.inner
                                .pts_state
                                .lock()
                                .getting_diff_for
                                .remove(&channel_id);
                            Err(e)
                        }
                    }
                } else {
                    tracing::debug!(
                        "[ferogram] channel {channel_id} pts gap: expected {expected}, got {got}: buffering"
                    );
                    Ok(vec![])
                }
            }
            PtsCheckResult::Duplicate => {
                tracing::debug!("[ferogram] channel {channel_id} pts duplicate, discarding");
                Ok(vec![])
            }
        }
    }

    /// Called periodically (e.g. from keepalive) to fire getDifference
    /// if no update has been received for > 15 minutes.
    ///
    /// also drives per-entry possible-gap deadlines independently of
    /// incoming updates.  Previously the POSSIBLE_GAP_DEADLINE_MS window was
    /// only evaluated when a new incoming update called check_and_fill_gap
    /// meaning a quiet channel with a real gap would never fire getDifference
    /// until another update arrived.  This
    /// which scans all LiveEntry.effective_deadline() on every keepalive tick.
    pub async fn check_update_deadline(&self) -> Result<(), InvocationError> {
        // Stuck-diff watchdog: if getting_global_diff has been true for more than
        // 30 s the in-flight getDifference RPC is assumed hung (e.g. half-open TCP
        // that the OS keepalive hasn't killed yet).  Reset the guard so the next
        // gap_tick cycle can issue a fresh getDifference.  The 30-second timeout
        // in get_difference() will concurrently return an error and also clear the
        // flag; this watchdog is a belt-and-suspenders safety net for edge cases
        // where that timeout itself is somehow delayed.
        {
            let stuck = {
                let s = self.inner.pts_state.lock();
                s.getting_global_diff
                    && s.getting_global_diff_since
                        .map(|t| t.elapsed().as_secs() > 30)
                        .unwrap_or(false)
            };
            if stuck {
                tracing::warn!(
                    "[ferogram] getDifference in-flight for >30 s: \
                     resetting guard so gap_tick can retry"
                );
                let mut s = self.inner.pts_state.lock();
                s.getting_global_diff = false;
                s.getting_global_diff_since = None;
            }
        }

        // existing 5-minute global timeout
        let exceeded = self.inner.pts_state.lock().deadline_exceeded();
        if exceeded {
            tracing::info!("[ferogram] update deadline exceeded: fetching getDifference");
            let updates = self.get_difference().await?;
            for u in updates {
                if self.inner.update_tx.try_send(u).is_err() {
                    tracing::warn!("[ferogram] update channel full: dropping diff update");
                }
            }
        }

        // drive global possible-gap deadline
        // If the possible-gap window has expired but no new update has arrived
        // to trigger check_and_fill_gap, fire getDifference from here.
        {
            let gap_expired = self.inner.possible_gap.lock().global_deadline_elapsed();
            // Note: get_difference() is now atomic (check-and-set), so the
            // `already` guard is advisory only; get_difference() will bail
            // safely if another call is already in flight.
            if gap_expired {
                tracing::debug!(
                    "[ferogram] B3 global possible-gap deadline expired: getDifference"
                );
                // get_difference() snapshots and drains the pre-existing buffer at its
                // start (before the RPC), so updates that arrive DURING the RPC flight
                // remain in possible_gap for the next cycle.  Never drain here.
                match self.get_difference().await {
                    Ok(updates) => {
                        for u in updates {
                            if self.inner.update_tx.try_send(u).is_err() {
                                tracing::warn!(
                                    "[ferogram] update channel full: dropping gap update"
                                );
                            }
                        }
                    }
                    Err(e) => {
                        tracing::warn!("[ferogram] B3 global gap diff failed: {e}");
                        return Err(e);
                    }
                }
            }
        }

        // drive per-channel possible-gap deadlines
        // Collect expired channel IDs up-front to avoid holding the lock across awaits.
        let expired_channels: Vec<i64> = {
            let gap = self.inner.possible_gap.lock();
            gap.channel
                .keys()
                .copied()
                .filter(|&id| gap.channel_deadline_elapsed(id))
                .collect()
        };
        for channel_id in expired_channels {
            let already = self
                .inner
                .pts_state
                .lock()
                .getting_diff_for
                .contains(&channel_id);
            if already {
                continue;
            }
            tracing::debug!(
                "[ferogram] B3 channel {channel_id} possible-gap deadline expired: getChannelDifference"
            );
            // Mark in-flight before spawning so a racing incoming update can't
            // also spawn a diff for the same channel.
            self.inner
                .pts_state
                .lock()
                .getting_diff_for
                .insert(channel_id);
            let buffered = self.inner.possible_gap.lock().drain_channel(channel_id);
            let c = self.clone();
            let utx = self.inner.update_tx.clone();
            tokio::spawn(async move {
                match c.get_channel_difference(channel_id).await {
                    Ok(mut updates) => {
                        c.inner
                            .pts_state
                            .lock()
                            .getting_diff_for
                            .remove(&channel_id);
                        updates.splice(0..0, buffered);
                        for u in updates {
                            if utx.try_send(attach_client_to_update(u, &c)).is_err() {
                                tracing::warn!(
                                    "[ferogram] update channel full: dropping ch gap update"
                                );
                            }
                        }
                    }
                    Err(e) => {
                        tracing::warn!("[ferogram] B3 channel {channel_id} gap diff failed: {e}");
                        c.inner
                            .pts_state
                            .lock()
                            .getting_diff_for
                            .remove(&channel_id);
                    }
                }
            });
        }

        Ok(())
    }
}
