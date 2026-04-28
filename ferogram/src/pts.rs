// Copyright (c) Ankit Chaubey <ankitchaubey.dev@gmail.com>
// SPDX-License-Identifier: MIT OR Apache-2.0
//
// ferogram: async Telegram MTProto client in Rust
// https://github.com/ankit-chaubey/ferogram
//
// If you use or modify this code, keep this notice at the top of your file
// and include the LICENSE-MIT or LICENSE-APACHE file from this repository:
// https://github.com/ankit-chaubey/ferogram

use std::collections::{HashMap, HashSet};
use std::time::Instant;

use ferogram_tl_types as tl;

use crate::session_backend::UpdateStateChange;
use crate::util::decode_checked;
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
    /// Limit starts at 100, then rises to 1000 after the first successful response.
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
    /// True once sync_pts_state() has succeeded post-auth (or state was restored
    /// from a saved session).  While false, gap detection is disabled.
    pub state_ready: bool,
    /// Channels that permanently returned CHANNEL_INVALID or CHANNEL_PRIVATE.
    /// Updates for these channels are force-dispatched without pts tracking.
    /// Prevents the infinite getChannelDifference → CHANNEL_INVALID loop.
    pub permanently_invalid_channels: HashSet<i64>,
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
            state_ready: true,
            permanently_invalid_channels: HashSet::new(),
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
            .map(|t: &std::time::Instant| t.elapsed().as_secs() > 15 * 60)
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

    /// Check top-level seq for Updates / UpdatesCombined containers.
    ///
    /// Rejects seq_start > new_seq as malformed. Without this check a server
    /// could set new_seq = i32::MAX and poison self.seq, dropping all future updates.
    pub fn check_seq(&self, new_seq: i32, seq_start: i32) -> PtsCheckResult {
        // Reject malformed containers where seq_start > new_seq.
        if seq_start > new_seq {
            return PtsCheckResult::Duplicate;
        }
        if self.seq == 0 {
            return PtsCheckResult::Ok;
        } // uninitialised: accept any
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

/// Extract a `(canonical_peer_id, msg_id)` key for deduplication from a
/// `NewMessage` or `MessageEdited` update.  Returns `None` for all other
/// update types.
fn msg_dedup_key(upd: &update::Update) -> Option<(i64, i32)> {
    let msg = match upd {
        update::Update::NewMessage(m) | update::Update::MessageEdited(m) => &m.raw,
        _ => return None,
    };
    let (peer, id) = match msg {
        tl::enums::Message::Message(m) => (&m.peer_id, m.id),
        tl::enums::Message::Service(m) => (&m.peer_id, m.id),
        _ => return None,
    };
    let peer_id = match peer {
        tl::enums::Peer::Channel(c) => c.channel_id,
        tl::enums::Peer::Chat(c) => c.chat_id,
        tl::enums::Peer::User(u) => u.user_id,
    };
    Some((peer_id, id))
}

impl Client {
    // Write a state change directly to the session backend.
    // Called after confirmed protocol checkpoints (getDifference, getChannelDifference,
    // sync_pts_state). Blocking I/O but the session file is tiny so it's fast.
    #[inline]
    fn persist_state(&self, change: UpdateStateChange) {
        if let Err(e) = self.inner.session_backend.apply_update_state(change) {
            tracing::warn!("[ferogram/persist] state write failed: {e}");
        }
    }

    // Filter a batch of updates through the bounded dedup cache.
    // Only NewMessage and MessageEdited are checked; everything else passes through.
    fn filter_dedupes(&self, updates: Vec<update::Update>) -> Vec<update::Update> {
        if updates.is_empty() {
            return updates;
        }
        let mut cache = self.inner.dedupe_cache.lock();
        updates
            .into_iter()
            .filter(|u| {
                if let Some((peer_id, msg_id)) = msg_dedup_key(u) {
                    !cache.check_and_insert(peer_id, msg_id)
                } else {
                    true
                }
            })
            .collect()
    }

    /// Fetch and replay any updates missed since the persisted pts.
    ///
    /// Loops on `Difference::Slice` until the server returns a final
    /// `Difference` or `Empty`, accumulating all batches.
    pub async fn get_difference(&self) -> Result<Vec<update::Update>, InvocationError> {
        // Atomically claim the in-flight slot.  Only the first caller proceeds;
        // all others wait for the in-flight diff to complete instead of returning
        // empty immediately.
        //
        // returning Ok(vec![]) right away caused a race where
        // an updateShortMessage "unknown sender" getDifference call overlapped
        // with the B3 gap-timer getDifference. The spawned task got back an
        // empty result, dispatched nothing, and the DM was silently dropped.
        // With the wait-and-return approach, the in-flight diff dispatches the
        // DM itself; the waiting caller then returns Ok(vec![]) safely (the
        // updates were already sent via update_tx by the owner of the in-flight diff).
        loop {
            {
                let mut s = self.inner.pts_state.lock().await;
                if !s.getting_global_diff {
                    // Slot is free; claim it atomically.
                    s.getting_global_diff = true;
                    s.getting_global_diff_since = Some(Instant::now());
                    break; // fall through to the actual RPC
                }
                // Another task is running the diff; release the lock before sleeping.
            }
            // Poll at 50 ms intervals; timeout is 5 s longer than the inner
            // 30 s RPC timeout so the guard is always cleared before we give
            // up (either by the RPC completing or by the watchdog resetting it).
            static WAIT_DEADLINE_SECS: u64 = 35;
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            if !self.inner.pts_state.lock().await.getting_global_diff {
                // In-flight diff has finished and already dispatched its updates.
                return Ok(vec![]);
            }
            // Check the absolute deadline by inspecting getting_global_diff_since.
            let elapsed = self
                .inner
                .pts_state
                .lock()
                .await
                .getting_global_diff_since
                .map(|t| t.elapsed())
                .unwrap_or_default();
            if elapsed.as_secs() >= WAIT_DEADLINE_SECS {
                tracing::warn!(
                    "[ferogram] get_difference: waited {WAIT_DEADLINE_SECS} s for \
                     in-flight getDifference to finish; giving up (will retry on next gap)"
                );
                return Ok(vec![]);
            }
        }

        // Drain the possible-gap buffer before the RPC.
        // On success, the server response covers these pts values (discard).
        // On error, restore them so the next gap_tick can retry.
        let pre_diff = self.inner.possible_gap.lock().await.drain_global();

        // Hard 30-second timeout: prevents a hung connection from blocking the guard.
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
            let mut s = self.inner.pts_state.lock().await;
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
                let mut gap = self.inner.possible_gap.lock().await;
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
                let s = self.inner.pts_state.lock().await;
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
                    // Extract the unknown constructor ID directly from the error
                    // type, rather than from body[..4] which always shows the
                    // outer `updates.difference` constructor (misleading since
                    // the actual failure is inside a field, e.g. new_messages).
                    let unknown_cid = match &e {
                        ferogram_tl_types::deserialize::Error::UnexpectedConstructor { id } => {
                            format!("{id:#010x}")
                        }
                        ferogram_tl_types::deserialize::Error::UnexpectedEof => {
                            "unexpected-eof".to_owned()
                        }
                    };
                    // This most commonly means a new Telegram Message or Update
                    // constructor was added that ferogram's api.tl doesn't know
                    // about yet.  The TL cursor misaligns on the unknown item,
                    // reads into zero-padding, and surfaces as 0x00000000.
                    // Fix: regenerate ferogram-tl-types from a newer api.tl that
                    // includes the missing constructor.
                    tracing::warn!(
                        "[ferogram] getDifference: TL schema mismatch - \
                         unknown constructor {unknown_cid} encountered while parsing \
                         getDifference response ({} updates already accumulated). \
                         This means api.tl is missing a new constructor. \
                         Resyncing pts to server state to prevent infinite retry.",
                        all_updates.len()
                    );
                    // Resync pts to the server's current state so the next
                    // gap_tick does not immediately re-enter getDifference
                    // with the same stale pts, which would repeat this error
                    // on the same response indefinitely.
                    // If sync fails here, the watchdog in check_update_deadline
                    // resets the in-flight guard after 30 s and gap_tick retries.
                    let _ = self.sync_pts_state().await;
                    return Ok(all_updates);
                }
            };

            match diff {
                tl::enums::updates::Difference::Empty(e) => {
                    let mut s = self.inner.pts_state.lock().await;
                    s.date = e.date;
                    s.seq = e.seq;
                    s.touch();
                    let (pts, date, seq) = (s.pts, s.date, s.seq);
                    drop(s); // release lock before I/O
                    // Checkpoint: persist this diff boundary immediately.
                    self.persist_state(UpdateStateChange::Primary { pts, date, seq });
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
                        let s = self.inner.pts_state.lock().await;
                        s.channel_pts.clone()
                    };
                    let mut new_state = PtsState::from_server_state(&ns);
                    // Preserve per-channel pts across the global reset.
                    for (cid, cpts) in saved_channel_pts {
                        new_state.channel_pts.entry(cid).or_insert(cpts);
                    }
                    // Preserve in-flight sets: we clear getting_global_diff ourselves.
                    new_state.getting_global_diff = true; // will be cleared by caller
                    let (new_pts, new_qts, new_date, new_seq) =
                        (new_state.pts, new_state.qts, new_state.date, new_state.seq);
                    {
                        let mut s = self.inner.pts_state.lock().await;
                        let getting_diff_for = std::mem::take(&mut s.getting_diff_for);
                        let since = s.getting_global_diff_since; // preserve watchdog timestamp
                        *s = new_state;
                        s.getting_diff_for = getting_diff_for;
                        s.getting_global_diff_since = since;
                    }
                    // Protocol checkpoint: flush primary + secondary immediately.
                    self.persist_state(UpdateStateChange::Primary {
                        pts: new_pts,
                        date: new_date,
                        seq: new_seq,
                    });
                    self.persist_state(UpdateStateChange::Secondary { qts: new_qts });
                    // Safety-net dedup for this diff batch before returning.
                    all_updates = self.filter_dedupes(all_updates);
                    // Final response: stop looping.
                    return Ok(all_updates);
                }

                tl::enums::updates::Difference::Slice(d) => {
                    // Partial response: apply intermediate_state and continue.
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
                        let s = self.inner.pts_state.lock().await;
                        s.channel_pts.clone()
                    };
                    let mut new_state = PtsState::from_server_state(&ns);
                    for (cid, cpts) in saved_channel_pts {
                        new_state.channel_pts.entry(cid).or_insert(cpts);
                    }
                    new_state.getting_global_diff = true;
                    let (slice_pts, slice_qts, slice_date, slice_seq) =
                        (new_state.pts, new_state.qts, new_state.date, new_state.seq);
                    {
                        let mut s = self.inner.pts_state.lock().await;
                        let getting_diff_for = std::mem::take(&mut s.getting_diff_for);
                        let since = s.getting_global_diff_since; // preserve watchdog timestamp
                        *s = new_state;
                        s.getting_diff_for = getting_diff_for;
                        s.getting_global_diff_since = since;
                    }
                    // Checkpoint each slice so a crash mid-loop doesn't lose the
                    // progress already made.
                    self.persist_state(UpdateStateChange::Primary {
                        pts: slice_pts,
                        date: slice_date,
                        seq: slice_seq,
                    });
                    self.persist_state(UpdateStateChange::Secondary { qts: slice_qts });
                    continue;
                }

                tl::enums::updates::Difference::TooLong(d) => {
                    tracing::warn!(
                        "[ferogram] getDifference: TooLong (pts={}): re-syncing",
                        d.pts
                    );
                    self.inner.pts_state.lock().await.pts = d.pts;
                    self.sync_pts_state().await?;
                    // Discard any partially-accumulated updates from prior slice
                    // iterations. These come from an intermediate state that is no
                    // longer coherent relative to the new pts (d.pts). Returning them
                    // would give the caller a stale partial set, and pts would then
                    // jump forward past those updates, causing false gap detection on
                    // the next getDifference call.
                    all_updates.clear();
                    return Ok(all_updates);
                }
            }
        }
    }

    /// Fetch missed updates for a single channel.
    pub async fn get_channel_difference(
        &self,
        channel_id: i64,
    ) -> Result<Vec<update::Update>, InvocationError> {
        let local_pts = self
            .inner
            .pts_state
            .lock()
            .await
            .channel_pts
            .get(&channel_id)
            .copied()
            .unwrap_or(0);

        let mut access_hash = self
            .inner
            .peer_cache
            .read()
            .await
            .channels
            .get(&channel_id)
            .copied()
            .unwrap_or(0);

        // No access hash in cache → try to refresh the peer cache via GetDialogs
        // before giving up.  This handles the common case where the access_hash
        // was evicted after a reconnect (peer_cache is in-memory only) but the
        // channel is still accessible.
        if access_hash == 0 {
            tracing::warn!(
                "[ferogram] channel {channel_id}: access_hash not cached; \
                 attempting recovery via GetDialogs refresh"
            );
            if let Err(e) = self.prefetch_channel_access_hashes().await {
                tracing::warn!("[ferogram] channel {channel_id}: GetDialogs refresh failed: {e}");
            }
            let fresh_hash = self
                .inner
                .peer_cache
                .read()
                .await
                .channels
                .get(&channel_id)
                .copied()
                .unwrap_or(0);
            if fresh_hash == 0 {
                // Still missing after refresh; channel is genuinely inaccessible.
                tracing::warn!(
                    "[ferogram] channel {channel_id}: access_hash still missing after \
                     GetDialogs refresh; removing from pts tracking to prevent \
                     the infinite getChannelDifference → CHANNEL_INVALID loop"
                );
                return Err(InvocationError::Rpc(RpcError {
                    code: 400,
                    name: "CHANNEL_INVALID".into(),
                    value: None,
                }));
            }
            tracing::debug!(
                "[ferogram] channel {channel_id}: access_hash recovered via GetDialogs"
            );
            // Use the freshly fetched hash for the InputChannel below.
            access_hash = fresh_hash;
        }

        tracing::debug!("[ferogram] getChannelDifference channel_id={channel_id} pts={local_pts}");

        let channel = tl::enums::InputChannel::InputChannel(tl::types::InputChannel {
            channel_id,
            access_hash,
        });

        // Limit 100 on first call, 1000 on subsequent; bots use the server maximum.
        let diff_limit = if self.inner.is_bot.load(std::sync::atomic::Ordering::Relaxed) {
            CHANNEL_DIFF_LIMIT_BOT
        } else {
            let call_count = self
                .inner
                .pts_state
                .lock()
                .await
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
                // Bump per-channel counter so subsequent calls use higher limit.
                self.inner
                    .pts_state
                    .lock()
                    .await
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
        let diff = decode_checked::<tl::enums::updates::ChannelDifference>(
            "updates.getChannelDifference",
            &body,
        )?;

        let mut updates = Vec::new();

        match diff {
            tl::enums::updates::ChannelDifference::Empty(e) => {
                tracing::debug!("[ferogram] getChannelDifference: empty (pts={})", e.pts);
                self.inner
                    .pts_state
                    .lock()
                    .await
                    .advance_channel(channel_id, e.pts);
                // Checkpoint immediately.
                self.persist_state(UpdateStateChange::Channel {
                    id: channel_id,
                    pts: e.pts,
                });
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
                    .await
                    .advance_channel(channel_id, d.pts);
                // Checkpoint immediately.
                self.persist_state(UpdateStateChange::Channel {
                    id: channel_id,
                    pts: d.pts,
                });
                // Safety-net dedup for this channel diff batch.
                updates = self.filter_dedupes(updates);
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
                // Setting pts=0 here caused an infinite loop: the next call used
                // local_pts.max(1)=1, still far behind the server -> TooLong again.
                // Extract the server's current pts from d.dialog instead and advance
                // to that value. If dialog.pts is absent, remove channel from tracking
                // so gap detection re-bootstraps on the next real update.
                let server_channel_pts: Option<i32> = match &d.dialog {
                    tl::enums::Dialog::Dialog(dlg) => dlg.pts,
                    _ => None,
                };
                let recovery_pts = server_channel_pts.unwrap_or(0);
                {
                    let mut s = self.inner.pts_state.lock().await;
                    if recovery_pts > 0 {
                        s.advance_channel(channel_id, recovery_pts);
                    } else {
                        s.channel_pts.remove(&channel_id);
                    }
                }
                self.persist_state(UpdateStateChange::Channel {
                    id: channel_id,
                    pts: recovery_pts,
                });
                tracing::debug!(
                    "[ferogram] getChannelDifference TooLong: channel {channel_id}                      reset to server pts={recovery_pts}"
                );
            }
        }

        Ok(updates)
    }

    pub async fn sync_pts_state(&self) -> Result<(), InvocationError> {
        let body = self
            .rpc_call_raw_pub(&tl::functions::updates::GetState {})
            .await?;
        let tl::enums::updates::State::State(s) =
            decode_checked::<tl::enums::updates::State>("updates.getState", &body)?;
        let mut state = self.inner.pts_state.lock().await;
        state.pts = s.pts;
        state.qts = s.qts;
        state.date = s.date;
        state.seq = s.seq;
        state.touch();
        state.state_ready = true;
        let (pts, qts, date, seq) = (state.pts, state.qts, state.date, state.seq);
        drop(state); // release lock before I/O
        tracing::debug!(
            "[ferogram] pts synced: pts={}, qts={}, seq={}",
            pts,
            qts,
            seq
        );
        // Hard checkpoint: session is authoritative after GetState.
        self.persist_state(UpdateStateChange::Primary { pts, date, seq });
        self.persist_state(UpdateStateChange::Secondary { qts });
        Ok(())
    }
    /// Check global pts, buffer during possible-gap window, fetch diff if real gap.
    ///
    /// While a global getDifference is in-flight, incoming socket updates are
    /// buffered into `possible_gap` (not dispatched). After getDifference advances
    /// pts, those buffered updates are re-checked by gap_tick: updates whose pts
    /// falls within the diff range are Duplicate (discarded); genuinely new ones
    /// are applied normally. This prevents the force-dispatch + diff-replay double
    /// emission that caused duplicate command handling.
    pub async fn check_and_fill_gap(
        &self,
        new_pts: i32,
        pts_count: i32,
        upd: Option<update::Update>,
    ) -> Result<Vec<update::Update>, InvocationError> {
        // State not yet initialised: force-dispatch without pts tracking.
        if !self.inner.pts_state.lock().await.state_ready {
            tracing::debug!("[ferogram] state not ready: force-dispatching pts={new_pts}");
            return Ok(upd.into_iter().collect());
        }

        // getDiff in flight: buffer this update so it is NOT emitted now.
        //
        // Previously this path force-dispatched the update immediately, which
        // caused a duplicate when getDifference returned the same update ~150 ms
        // later (the server includes all updates up to its snapshot time, which
        // overlaps the force-dispatch window in an async client).
        //
        // After getDifference completes and advances pts, gap_tick re-evaluates
        // the possible_gap buffer.  Updates already covered by the diff response
        // will have pts ≤ new server pts → PtsCheckResult::Duplicate → discarded.
        // Genuinely post-diff updates will be Ok → dispatched once.
        if self.inner.pts_state.lock().await.getting_global_diff {
            tracing::debug!(
                "[ferogram] global diff in flight: buffering pts={new_pts} (suppressing force-dispatch to prevent duplicate)"
            );
            let mut gap = self.inner.possible_gap.lock().await;
            if let Some(u) = upd {
                gap.push_global(u);
            } else {
                gap.touch_global_timer();
            }
            return Ok(vec![]);
        }

        // FIX: check + advance in a single lock acquisition to eliminate the
        // TOCTOU window where two concurrent tasks both see stale pts and both
        // trigger spurious getDifference calls.
        let result = {
            let mut s = self.inner.pts_state.lock().await;
            let r = s.check_pts(new_pts, pts_count);
            if r == PtsCheckResult::Ok {
                s.advance(new_pts);
            }
            r
        };
        match result {
            PtsCheckResult::Ok => {
                // pts already advanced in the atomic check+advance above.
                // Debounced persist: live-update advances are coalesced by the worker.
                let s = self.inner.pts_state.lock().await;
                self.persist_state(UpdateStateChange::Primary {
                    pts: s.pts,
                    date: s.date,
                    seq: s.seq,
                });
                drop(s);
                Ok(upd.into_iter().collect())
            }
            PtsCheckResult::Gap { expected, got } => {
                // Buffer the update; start the deadline timer even when upd=None
                // so the gap is resolved regardless of subsequent incoming traffic.
                {
                    let mut gap = self.inner.possible_gap.lock().await;
                    if let Some(u) = upd {
                        gap.push_global(u);
                    } else {
                        gap.touch_global_timer();
                    }
                }
                let deadline_elapsed = self
                    .inner
                    .possible_gap
                    .lock()
                    .await
                    .global_deadline_elapsed();
                if deadline_elapsed {
                    tracing::warn!(
                        "[ferogram] global pts gap: expected {expected}, got {got}: getDifference"
                    );
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
        let result = self
            .inner
            .pts_state
            .lock()
            .await
            .check_qts(new_qts, qts_count);
        match result {
            PtsCheckResult::Ok => {
                self.inner.pts_state.lock().await.advance_qts(new_qts);
                // Debounced persist for secret-chat counter.
                self.persist_state(UpdateStateChange::Secondary { qts: new_qts });
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
        let result = self
            .inner
            .pts_state
            .lock()
            .await
            .check_seq(new_seq, seq_start);
        match result {
            PtsCheckResult::Ok => {
                self.inner.pts_state.lock().await.advance_seq(new_seq);
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
        // Permanently inaccessible channel: force-dispatch without pts tracking.
        if self
            .inner
            .pts_state
            .lock()
            .await
            .permanently_invalid_channels
            .contains(&channel_id)
        {
            return Ok(upd.into_iter().collect());
        }

        // Skip if a diff is already in flight to prevent concurrent tasks.
        if self
            .inner
            .pts_state
            .lock()
            .await
            .getting_diff_for
            .contains(&channel_id)
        {
            tracing::debug!("[ferogram] channel {channel_id} diff already in flight, skipping");
            if let Some(u) = upd {
                self.inner
                    .possible_gap
                    .lock()
                    .await
                    .push_channel(channel_id, u);
            }
            return Ok(vec![]);
        }

        let result = self
            .inner
            .pts_state
            .lock()
            .await
            .check_channel_pts(channel_id, new_pts, pts_count);
        match result {
            PtsCheckResult::Ok => {
                let mut buffered = self
                    .inner
                    .possible_gap
                    .lock()
                    .await
                    .drain_channel(channel_id);
                self.inner
                    .pts_state
                    .lock()
                    .await
                    .advance_channel(channel_id, new_pts);
                // Debounced persist for live channel updates.
                self.persist_state(UpdateStateChange::Channel {
                    id: channel_id,
                    pts: new_pts,
                });
                if let Some(u) = upd {
                    buffered.push(u);
                }
                Ok(buffered)
            }
            PtsCheckResult::Gap { expected, got } => {
                if let Some(u) = upd {
                    self.inner
                        .possible_gap
                        .lock()
                        .await
                        .push_channel(channel_id, u);
                }
                let deadline_elapsed = self
                    .inner
                    .possible_gap
                    .lock()
                    .await
                    .channel_deadline_elapsed(channel_id);
                if deadline_elapsed {
                    tracing::warn!(
                        "[ferogram] channel {channel_id} pts gap: expected {expected}, got {got}: getChannelDifference"
                    );
                    // mark this channel as having a diff in flight.
                    self.inner
                        .pts_state
                        .lock()
                        .await
                        .getting_diff_for
                        .insert(channel_id);
                    let buffered = self
                        .inner
                        .possible_gap
                        .lock()
                        .await
                        .drain_channel(channel_id);
                    match self.get_channel_difference(channel_id).await {
                        Ok(mut diff_updates) => {
                            // diff complete, allow future gaps to be handled.
                            self.inner
                                .pts_state
                                .lock()
                                .await
                                .getting_diff_for
                                .remove(&channel_id);
                            diff_updates.splice(0..0, buffered);
                            Ok(diff_updates)
                        }
                        // Permanent access errors: remove the channel from pts tracking.
                        // The next update will be treated as first-seen, breaking the
                        // infinite gap → CHANNEL_INVALID loop.
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
                                let mut s = self.inner.pts_state.lock().await;
                                s.getting_diff_for.remove(&channel_id);
                                s.channel_pts.remove(&channel_id); // delete, not advance
                                s.permanently_invalid_channels.insert(channel_id); // ← never retry
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
                                let mut s = self.inner.pts_state.lock().await;
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
                                .await
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
    /// if no update has been received for > 15 minutes, and to drive
    /// per-channel possible-gap deadline checks.
    pub async fn check_update_deadline(&self) -> Result<(), InvocationError> {
        // Stuck-diff watchdog: reset the in-flight guard if getDifference has been
        // running for >30 s, allowing the next gap_tick to retry.
        {
            let stuck = {
                let s = self.inner.pts_state.lock().await;
                s.getting_global_diff
                    && s.getting_global_diff_since
                        .map(|t: std::time::Instant| t.elapsed().as_secs() > 30)
                        .unwrap_or(false)
            };
            if stuck {
                tracing::warn!(
                    "[ferogram] getDifference in-flight for >30 s: \
                     resetting guard so gap_tick can retry"
                );
                let mut s = self.inner.pts_state.lock().await;
                s.getting_global_diff = false;
                s.getting_global_diff_since = None;
            }
        }

        let exceeded = self.inner.pts_state.lock().await.deadline_exceeded();
        if exceeded {
            tracing::info!("[ferogram] update deadline exceeded: fetching getDifference");
            let updates = self.get_difference().await?;
            for u in updates {
                if self.inner.update_tx.try_send(u).is_err() {
                    tracing::warn!("[ferogram] update channel full: dropping diff update");
                }
            }
        }

        // Fire getDifference if the possible-gap deadline expired without a new update.
        {
            let gap_expired = self
                .inner
                .possible_gap
                .lock()
                .await
                .global_deadline_elapsed();
            if gap_expired {
                tracing::debug!(
                    "[ferogram] B3 global possible-gap deadline expired: getDifference"
                );
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

        // Collect expired channel IDs before awaiting to avoid holding the lock.
        let expired_channels: Vec<i64> = {
            let gap = self.inner.possible_gap.lock().await;
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
                .await
                .getting_diff_for
                .contains(&channel_id);
            if already {
                continue;
            }
            tracing::debug!(
                "[ferogram] B3 channel {channel_id} possible-gap deadline expired: getChannelDifference"
            );
            // Mark in-flight before spawning to prevent concurrent diff tasks.
            self.inner
                .pts_state
                .lock()
                .await
                .getting_diff_for
                .insert(channel_id);
            let buffered = self
                .inner
                .possible_gap
                .lock()
                .await
                .drain_channel(channel_id);
            let c = self.clone();
            let utx = self.inner.update_tx.clone();
            tokio::spawn(async move {
                match c.get_channel_difference(channel_id).await {
                    Ok(mut updates) => {
                        c.inner
                            .pts_state
                            .lock()
                            .await
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
                            .await
                            .getting_diff_for
                            .remove(&channel_id);
                    }
                }
            });
        }

        Ok(())
    }

    /// Sync pts state after a fresh DH exchange, retrying with backoff on 401.
    ///
    /// After a new auth key is created, Telegram's backend needs a moment to
    /// propagate it across all app servers.  Instead of a fixed 2-second sleep
    /// (which is too long when propagation is fast and too short when it's slow),
    /// this method fires GetState immediately and retries on AUTH_KEY_UNREGISTERED
    /// with exponential backoff.
    pub async fn sync_state_after_dh(&self) {
        // Guard: never call GetState before the client is authorised.
        if !self
            .inner
            .signed_in
            .load(std::sync::atomic::Ordering::SeqCst)
        {
            tracing::debug!("[ferogram] sync_state_after_dh: not signed in yet  - skipping");
            return;
        }
        for delay_ms in [0u64, 100, 300, 700, 1500] {
            if delay_ms > 0 {
                tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;
            }
            match self.sync_pts_state().await {
                Ok(()) => return,
                Err(ref e) if matches!(e, InvocationError::Rpc(r) if r.code == 401) => {
                    tracing::debug!(
                        "[ferogram] sync_state_after_dh: AUTH_KEY_UNREGISTERED \
                         (delay={delay_ms}ms), retrying"
                    );
                    continue;
                }
                Err(e) => {
                    tracing::warn!("[ferogram] sync_state_after_dh failed: {e}");
                    return;
                }
            }
        }
        tracing::warn!("[ferogram] sync_state_after_dh: all retries exhausted");
    }
}
