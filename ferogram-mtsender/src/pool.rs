/*
 * Copyright (c) 2026 Ankit Chaubey <ankitchaubey.dev@gmail.com>
 * https://github.com/ankit-chaubey
 *
 * Project: ferogram
 * Website: https://ferogram.dev
 *
 * Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
 * https://www.apache.org/licenses/LICENSE-2.0> or the MIT license
 * <LICENSE-MIT or https://opensource.org/licenses/MIT>, at your option.
 * This file may not be copied, modified, or distributed except according
 * to those terms.
 */

use crate::errors::InvocationError;
use crate::sender::DcConnection;
use crate::sender_task::{FrameEvent, RpcEnqueue, spawn_sender_task};
use ferogram_connect::util::maybe_gz_pack;
use ferogram_connect::{Socks5Config, TransportKind};
use ferogram_session::{DcEntry, DcFlags};
use ferogram_tl_types::{RemoteCall, Serializable};
use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use tokio::sync::{Mutex, mpsc, oneshot};

// Max simultaneous connections per DC.
const MAX_CONNS_PER_DC: usize = 3;

/// One slot in the per-DC connection pool.
///
/// Each slot is backed by a background sender task (see
/// [`crate::sender_task::spawn_sender_task`]), not a locked `DcConnection`.
/// Enqueueing a request just posts it to the task's mpsc channel and waits
/// on a oneshot for the result: no lock is held across the network round
/// trip, so any number of callers can have requests in flight on the same
/// slot at once. The task itself batches whatever is pending into as few
/// frames as possible and matches replies back to callers by msg_id,
/// regardless of the order responses arrive in.
///
/// `in_flight` still lets the pool pick the least-busy slot without needing
/// to touch the connection itself.
pub struct ConnSlot {
    rpc_tx: mpsc::Sender<RpcEnqueue>,
    pub in_flight: AtomicUsize,
    /// Set to `false` by the drain task below once the connection's sender
    /// task reports an error. Callers check this after a failed call to
    /// decide whether to evict the slot and retry on a fresh one, instead of
    /// matching on the specific `InvocationError` variant (the sender task
    /// always reports connection failures as `Deserialize`, since it has no
    /// way to know whether a given caller still cares about the original
    /// `Io`/etc. error kind once `fail_all` has fanned it out to everyone
    /// waiting on this connection).
    alive: Arc<AtomicBool>,
    /// Snapshot of the auth key / salt / time offset taken when the slot was
    /// created. Used by `collect_keys` to persist session info. The auth key
    /// never changes for a slot's lifetime; salt and time offset can drift a
    /// little as the connection runs (FutureSalts rotation), but a stale
    /// value here only costs one bad_server_salt round trip the next time
    /// this DC is reconnected, since the sender task self-corrects from
    /// server-supplied corrections either way.
    auth_key: [u8; 256],
    first_salt: i64,
    time_offset: i32,
}

/// Counts a request until its future is completed or dropped.
struct InFlightGuard<'a>(&'a AtomicUsize);

impl<'a> InFlightGuard<'a> {
    fn new(count: &'a AtomicUsize) -> Self {
        count.fetch_add(1, Ordering::Relaxed);
        Self(count)
    }
}

impl Drop for InFlightGuard<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::Relaxed);
    }
}

/// A selected ready slot, counted as busy from selection until the caller
/// finishes or cancels. Keeping this lease across an RPC lets the pool mutex
/// be released during network I/O without losing load accounting.
pub struct SlotLease {
    slot: Arc<ConnSlot>,
}

impl SlotLease {
    pub async fn invoke<R: RemoteCall>(&self, req: &R) -> Result<Vec<u8>, InvocationError> {
        DcPool::send_untracked(&self.slot, maybe_gz_pack(&req.to_bytes())).await
    }

    pub fn is_alive(&self) -> bool {
        self.slot.alive.load(Ordering::Acquire)
    }
}

impl Drop for SlotLease {
    fn drop(&mut self) {
        self.slot.in_flight.fetch_sub(1, Ordering::Relaxed);
    }
}

/// Pool of per-DC authenticated connections.
/// Media DCs may hold up to MAX_CONNS_PER_DC slots. Callers using a shared
/// mutex can reserve a slot and release that mutex before network I/O.
pub struct DcPool {
    /// Per-DC connection slots; inner Vec holds slot Arcs.
    pub conns: HashMap<i32, Vec<Arc<ConnSlot>>>,
    addrs: HashMap<i32, String>,
    /// Media-only DCs may use parallel file sessions.
    media_dcs: HashSet<i32>,
    /// DCs that have already received `invokeWithLayer(initConnection(...))`.
    init_done: std::collections::HashSet<i32>,
}

impl DcPool {
    /// Build an empty pool seeded with DC addresses and media capability.
    /// The client opens and initializes connections before inserting them.
    pub fn new(
        _home_dc_id: i32,
        dc_entries: &[DcEntry],
        _socks5: Option<Socks5Config>,
        _transport: TransportKind,
    ) -> Self {
        let addrs = dc_entries
            .iter()
            .map(|e| (e.dc_id, e.addr.clone()))
            .collect();
        let media_dcs = dc_entries
            .iter()
            .filter(|e| e.flags.contains(DcFlags::MEDIA_ONLY))
            .map(|e| e.dc_id)
            .collect();
        Self {
            conns: HashMap::new(),
            addrs,
            media_dcs,
            init_done: std::collections::HashSet::new(),
        }
    }

    /// Returns true if at least one live slot exists for `dc_id`.
    pub fn has_connection(&self, dc_id: i32) -> bool {
        self.conns
            .get(&dc_id)
            .is_some_and(|v| v.iter().any(|slot| slot.alive.load(Ordering::Acquire)))
    }

    /// Graduate an already set-up `DcConnection` into a pipelined slot.
    ///
    /// The connection has already done its DH / PFS bind / initConnection as
    /// a plain `DcConnection`. From here on its socket is owned by a single
    /// background task; this function just spawns that task and wraps the
    /// resulting handle in a `ConnSlot`.
    fn spawn_slot(conn: DcConnection) -> Arc<ConnSlot> {
        let auth_key = conn.auth_key_bytes();
        let first_salt = conn.first_salt();
        let time_offset = conn.time_offset();
        let (stream, frame_kind, enc) = conn.into_parts();

        let (handle, mut frame_rx) = spawn_sender_task(stream, enc, frame_kind, None);

        // Pool slots don't support reconnect: on failure the pool just
        // evicts the whole DC and a fresh slot is opened from scratch on the
        // next call. Dropping reconnect_tx here means the sender task's
        // error branch sees its reconnect channel closed and shuts itself
        // down cleanly instead of waiting for a reconnect that will never
        // come.
        drop(handle.reconnect_tx);

        let alive = Arc::new(AtomicBool::new(true));
        let alive_for_drain = alive.clone();
        tokio::spawn(async move {
            while let Some(event) = frame_rx.recv().await {
                if let FrameEvent::Error(e) = event {
                    tracing::warn!("[ferogram::pool] worker connection dropped: {e}");
                    alive_for_drain.store(false, Ordering::Release);
                    break;
                }
                // FrameEvent::Update / Connected: pool connections don't
                // dispatch updates, nothing to do.
            }
        });

        Arc::new(ConnSlot {
            rpc_tx: handle.rpc_tx,
            in_flight: AtomicUsize::new(0),
            alive,
            auth_key,
            first_salt,
            time_offset,
        })
    }

    /// Insert a pre-built, already initialized connection into the pool as a
    /// new slot.
    pub fn insert(&mut self, dc_id: i32, conn: DcConnection) {
        let slot = Self::spawn_slot(conn);
        let slots = self.conns.entry(dc_id).or_default();
        slots.retain(|old| old.alive.load(Ordering::Acquire));
        slots.push(slot);
        self.init_done.insert(dc_id);
        let total: usize = self.conns.values().map(|v| v.len()).sum();
        crate::metrics_shim::gauge!("ferogram.connections_active").set(total as f64);
    }

    /// Publish a connection only after its full client-level setup succeeds.
    /// Dropping or failing the setup future leaves the pool unchanged.
    pub async fn insert_after_setup<F>(
        pool: &Mutex<Self>,
        dc_id: i32,
        setup: F,
    ) -> Result<(), InvocationError>
    where
        F: Future<Output = Result<DcConnection, InvocationError>>,
    {
        let conn = setup.await?;
        pool.lock().await.insert(dc_id, conn);
        Ok(())
    }

    /// Select only a slot that the client has fully set up and inserted.
    /// Opening a connection here would bypass initConnection and foreign-DC auth.
    fn select_slot(&self, dc_id: i32) -> Result<Arc<ConnSlot>, InvocationError> {
        self.conns
            .get(&dc_id)
            .and_then(|slots| {
                slots
                    .iter()
                    .filter(|slot| slot.alive.load(Ordering::Acquire))
                    .min_by_key(|slot| slot.in_flight.load(Ordering::Relaxed))
            })
            .cloned()
            .ok_or_else(|| {
                InvocationError::Deserialize(format!("no ready connection for DC{dc_id}"))
            })
    }

    pub fn reserve_slot(&self, dc_id: i32) -> Result<SlotLease, InvocationError> {
        let slot = self.select_slot(dc_id)?;
        slot.in_flight.fetch_add(1, Ordering::Relaxed);
        Ok(SlotLease { slot })
    }

    /// Apply a call's failure to the pool only when its slot still belongs to
    /// this DC. A late result must not evict a replacement connection.
    pub fn finish_call(
        &mut self,
        dc_id: i32,
        lease: &SlotLease,
        result: &Result<Vec<u8>, InvocationError>,
    ) {
        if let Err(e) = result {
            let _kind = match e {
                InvocationError::Rpc(_) => "rpc",
                InvocationError::Io(_) => "io",
                _ => "other",
            };
            crate::metrics_shim::counter!("ferogram.rpc_errors_total", "kind" => _kind)
                .increment(1);
        }
        let fatal = matches!(result, Err(InvocationError::Rpc(e)) if e.code == -404)
            || (result.is_err() && !lease.is_alive());
        if fatal
            && self
                .conns
                .get(&dc_id)
                .is_some_and(|slots| slots.iter().any(|slot| Arc::ptr_eq(slot, &lease.slot)))
        {
            self.evict(dc_id);
        }
    }

    /// Whether a ready media-only DC can benefit from another connection.
    /// The media flag describes the endpoint, even when its DC id is home.
    /// Main RPC sessions use non-media endpoints and stay on one connection
    /// unless the server's tmp_sessions limit is explicitly supported.
    pub fn should_expand(&self, dc_id: i32) -> bool {
        if !self.media_dcs.contains(&dc_id) {
            return false;
        }
        let Some(slots) = self.conns.get(&dc_id) else {
            return false;
        };
        !slots.is_empty()
            && slots.len() < MAX_CONNS_PER_DC
            && slots.iter().all(|slot| {
                slot.alive.load(Ordering::Acquire) && slot.in_flight.load(Ordering::Relaxed) > 0
            })
    }

    /// Evict all slots for a DC (called on connection failure to force
    /// reconnection on the next call).
    pub fn evict(&mut self, dc_id: i32) {
        self.conns.remove(&dc_id);
        self.init_done.remove(&dc_id);
        let total: usize = self.conns.values().map(|v| v.len()).sum();
        crate::metrics_shim::gauge!("ferogram.connections_active").set(total as f64);
        tracing::debug!("[ferogram::pool] evicted all connections for DC{dc_id}");
    }

    /// Enqueue `body` on `slot` and await the result.
    ///
    /// This is the only place that touches `rpc_tx`/the oneshot: no mutex,
    /// no blocking for the duration of the round trip. Multiple callers can
    /// call this against the same slot concurrently and their requests will
    /// pipeline on the wire instead of queueing behind each other.
    async fn send_via_slot(
        slot: &Arc<ConnSlot>,
        body: Vec<u8>,
    ) -> Result<Vec<u8>, InvocationError> {
        let _in_flight = InFlightGuard::new(&slot.in_flight);
        Self::send_untracked(slot, body).await
    }

    async fn send_untracked(
        slot: &Arc<ConnSlot>,
        body: Vec<u8>,
    ) -> Result<Vec<u8>, InvocationError> {
        let (tx, rx) = oneshot::channel();
        let send_result = slot.rpc_tx.send(RpcEnqueue { body, tx }).await;
        if send_result.is_err() {
            slot.alive.store(false, Ordering::Release);
            Err(InvocationError::Deserialize(
                "worker sender task shut down".into(),
            ))
        } else {
            match rx.await {
                Ok(r) => r,
                Err(_) => {
                    slot.alive.store(false, Ordering::Release);
                    Err(InvocationError::Deserialize(
                        "worker rpc channel closed".into(),
                    ))
                }
            }
        }
    }

    /// Invoke a raw RPC call on an already initialized slot of the given DC.
    /// Shared-pool callers should use `reserve_slot` and `SlotLease::invoke`
    /// so their mutex is released before the network round trip.
    ///
    /// On connection death or a `-404` (auth key gone), this evicts the
    /// dead slot and returns the error as-is -- it does not reconnect and
    /// resend itself. `DcPool` has no `api_id`/device info to build
    /// `invokeWithLayer(initConnection(...))`, so it can't safely redo
    /// setup on its own. The caller sees `!pool.has_connection(dc_id)`
    /// after this returns and is expected to redo full setup -- cached
    /// auth key, `InitConnection`, and (for foreign DCs)
    /// `auth.importAuthorization` -- before retrying.
    pub async fn invoke_on_dc<R: RemoteCall>(
        &mut self,
        dc_id: i32,
        _dc_entries: &[DcEntry],
        req: &R,
    ) -> Result<Vec<u8>, InvocationError> {
        let slot = self.select_slot(dc_id)?;
        let body = maybe_gz_pack(&req.to_bytes());
        let result = Self::send_via_slot(&slot, body.clone()).await;

        if let Err(ref e) = result {
            let _kind = match e {
                InvocationError::Rpc(_) => "rpc",
                InvocationError::Io(_) => "io",
                _ => "other",
            };
            crate::metrics_shim::counter!("ferogram.rpc_errors_total", "kind" => _kind)
                .increment(1);
        }

        if let Err(InvocationError::Rpc(ref e)) = result
            && e.code == -404
        {
            // Telegram dropped the auth key (e.g. AndroidTV killed the socket during sleep).
            // Evict; the caller redoes DH + auth import + InitConnection and retries.
            tracing::warn!(
                "[ferogram::pool] DC{dc_id} returned -404 (auth key gone); evicting for caller to redo setup"
            );
            self.evict(dc_id);
            return result;
        }

        if result.is_err() && !slot.alive.load(Ordering::Acquire) {
            tracing::warn!(
                "[ferogram::pool] DC{dc_id} connection died mid-request; evicting for caller to redo setup"
            );
            self.evict(dc_id);
        }
        result
    }

    /// Mark a DC as having completed initConnection.
    pub fn mark_init_done(&mut self, dc_id: i32) {
        self.init_done.insert(dc_id);
    }

    /// Returns true if this DC has already received initConnection this session.
    pub fn is_init_done(&self, dc_id: i32) -> bool {
        self.init_done.contains(&dc_id)
    }

    /// Like `invoke_on_dc` but accepts any `Serializable` type.
    /// Same evict-and-propagate behavior as `invoke_on_dc` -- see its doc comment.
    pub async fn invoke_on_dc_serializable<S: Serializable>(
        &mut self,
        dc_id: i32,
        req: &S,
    ) -> Result<Vec<u8>, InvocationError> {
        let slot = self.select_slot(dc_id)?;
        let body = maybe_gz_pack(&req.to_bytes());
        let result = Self::send_via_slot(&slot, body.clone()).await;

        if let Err(InvocationError::Rpc(ref e)) = result
            && e.code == -404
        {
            tracing::warn!(
                "[ferogram::pool] DC{dc_id} returned -404 (serializable path); evicting for caller to redo setup"
            );
            self.evict(dc_id);
            return result;
        }

        if result.is_err() && !slot.alive.load(Ordering::Acquire) {
            tracing::warn!(
                "[ferogram::pool] DC{dc_id} connection died mid-request (serializable path); evicting for caller to redo setup"
            );
            self.evict(dc_id);
        }
        result
    }

    /// Update the address table (called after `initConnection`).
    pub fn update_addrs(&mut self, entries: &[DcEntry]) {
        for e in entries {
            self.addrs.insert(e.dc_id, e.addr.clone());
            if e.flags.contains(DcFlags::MEDIA_ONLY) {
                self.media_dcs.insert(e.dc_id);
            } else {
                self.media_dcs.remove(&e.dc_id);
            }
        }
    }

    /// Save the auth keys from pool connections back into the DC entry list.
    /// Uses the first slot per DC (all slots share the same auth key).
    pub fn collect_keys(&self, entries: &mut [DcEntry]) {
        for e in entries.iter_mut() {
            if let Some(slots) = self.conns.get(&e.dc_id)
                && let Some(slot) = slots.first()
            {
                e.auth_key = Some(slot.auth_key);
                e.first_salt = slot.first_salt;
                e.time_offset = slot.time_offset;
            }
        }
    }
}

/// Serialize a `msgs_ack#62d6b459 { msg_ids: Vector<long> }` TL body.
///
/// This is sent as a non-content-related encrypted frame (even seq_no)
/// to acknowledge received server messages and prevent Telegram from
/// closing the connection due to un-acked messages.
pub(crate) fn build_msgs_ack_body(msg_ids: &[i64]) -> Vec<u8> {
    let mut out = Vec::with_capacity(4 + 4 + 4 + msg_ids.len() * 8);
    out.extend_from_slice(&0x62d6b459_u32.to_le_bytes()); // msgs_ack constructor
    out.extend_from_slice(&0x1cb5c415_u32.to_le_bytes()); // Vector constructor
    out.extend_from_slice(&(msg_ids.len() as u32).to_le_bytes());
    for &id in msg_ids {
        out.extend_from_slice(&id.to_le_bytes());
    }
    out
}

/// Serialize a `ping_delay_disconnect#f3427b8c { ping_id, disconnect_delay: 75 }` body.
///
/// Tells Telegram to close the connection after 75 seconds of silence.
pub(crate) fn build_msgs_ack_ping_body(ping_id: i64) -> Vec<u8> {
    // ping_delay_disconnect#f3427b8c ping_id:long disconnect_delay:int = Pong
    let mut out = Vec::with_capacity(4 + 8 + 4);
    out.extend_from_slice(&0xf3427b8c_u32.to_le_bytes()); // constructor
    out.extend_from_slice(&ping_id.to_le_bytes());
    out.extend_from_slice(&75_i32.to_le_bytes()); // disconnect_delay = 75 s
    out
}

#[cfg(test)]
mod pool_regressions {
    use super::*;

    fn fake_slot() -> (Arc<ConnSlot>, mpsc::Receiver<RpcEnqueue>) {
        let (rpc_tx, rpc_rx) = mpsc::channel(1);
        (
            Arc::new(ConnSlot {
                rpc_tx,
                in_flight: AtomicUsize::new(0),
                alive: Arc::new(AtomicBool::new(true)),
                auth_key: [0; 256],
                first_salt: 0,
                time_offset: 0,
            }),
            rpc_rx,
        )
    }

    #[tokio::test]
    async fn cancelled_rpc_releases_in_flight_slot() {
        let (slot, mut worker) = fake_slot();
        let caller_slot = slot.clone();
        let call = tokio::spawn(async move { DcPool::send_via_slot(&caller_slot, vec![1]).await });
        let _pending = worker.recv().await.expect("request enqueued");
        assert_eq!(slot.in_flight.load(Ordering::Relaxed), 1);
        call.abort();
        call.await.expect_err("call aborted");
        assert_eq!(slot.in_flight.load(Ordering::Relaxed), 0);
    }

    #[tokio::test]
    async fn completed_and_failed_rpcs_release_in_flight_slot_once() {
        let (slot, mut worker) = fake_slot();
        for outcome in [Ok(vec![2]), Err(InvocationError::Dropped)] {
            let caller_slot = slot.clone();
            let call =
                tokio::spawn(async move { DcPool::send_via_slot(&caller_slot, vec![1]).await });
            let pending = worker.recv().await.expect("request enqueued");
            assert_eq!(slot.in_flight.load(Ordering::Relaxed), 1);
            pending.tx.send(outcome).expect("caller waiting");
            let _ = call.await.expect("task finished");
            assert_eq!(slot.in_flight.load(Ordering::Relaxed), 0);
        }
        drop(worker);
        assert!(DcPool::send_via_slot(&slot, vec![1]).await.is_err());
        assert_eq!(slot.in_flight.load(Ordering::Relaxed), 0);
    }

    #[tokio::test]
    async fn busy_foreign_media_slot_never_routes_to_an_unprepared_connection() {
        use ferogram_session::DcFlags;
        use ferogram_tl_types::functions::help::GetConfig;

        let entry = DcEntry {
            dc_id: 2,
            addr: "127.0.0.1:0".into(),
            auth_key: None,
            first_salt: 0,
            time_offset: 0,
            flags: DcFlags::MEDIA_ONLY,
        };
        let mut pool = DcPool::new(1, &[entry.clone()], None, TransportKind::Abridged);
        let (ready_slot, mut worker) = fake_slot();
        ready_slot.in_flight.store(1, Ordering::Relaxed);
        pool.conns.insert(2, vec![ready_slot.clone()]);
        pool.mark_init_done(2);

        // The existing slot is ready, though busy. An extra connection cannot
        // become selectable until client-level setup has succeeded.
        {
            let entries = [entry];
            let call = pool.invoke_on_dc(2, &entries, &GetConfig {});
            tokio::pin!(call);
            let pending = tokio::select! {
                req = worker.recv() => req.expect("ready slot receives the RPC"),
                result = &mut call => panic!("RPC bypassed the ready slot: {result:?}"),
            };
            pending.tx.send(Ok(vec![42])).expect("caller waiting");
            assert_eq!(call.await.expect("ready slot succeeds"), vec![42]);
        }
        assert_eq!(pool.conns[&2].len(), 1);
    }

    #[tokio::test]
    async fn reserved_slot_cancellation_and_connection_failure_clear_pool_state() {
        use ferogram_session::DcFlags;
        use ferogram_tl_types::functions::help::GetConfig;

        let entry = DcEntry {
            dc_id: 2,
            addr: "127.0.0.1:0".into(),
            auth_key: None,
            first_salt: 0,
            time_offset: 0,
            flags: DcFlags::MEDIA_ONLY,
        };
        let mut pool = DcPool::new(1, &[entry], None, TransportKind::Abridged);
        let (slot, mut worker) = fake_slot();
        pool.conns.insert(2, vec![slot.clone()]);
        pool.mark_init_done(2);

        let lease = pool.reserve_slot(2).expect("ready slot");
        assert!(pool.should_expand(2));
        let call = tokio::spawn(async move { lease.invoke(&GetConfig {}).await });
        let _pending = worker.recv().await.expect("request enqueued");
        call.abort();
        call.await.expect_err("call cancelled");
        assert_eq!(slot.in_flight.load(Ordering::Relaxed), 0);
        assert!(!pool.should_expand(2));

        let lease = pool.reserve_slot(2).expect("slot reusable");
        let call = tokio::spawn(async move {
            let result = lease.invoke(&GetConfig {}).await;
            (lease, result)
        });
        let pending = worker.recv().await.expect("request enqueued");
        slot.alive.store(false, Ordering::Release);
        pending
            .tx
            .send(Err(InvocationError::Dropped))
            .expect("caller waiting");
        let (lease, result) = call.await.expect("call finished");
        pool.finish_call(2, &lease, &result);
        drop(lease);
        assert!(!pool.has_connection(2));
        assert_eq!(slot.in_flight.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn media_pool_can_expand_to_three_ready_slots_but_regular_pool_cannot() {
        use ferogram_session::DcFlags;
        let entries = [
            DcEntry {
                dc_id: 1,
                addr: "home".into(),
                auth_key: None,
                first_salt: 0,
                time_offset: 0,
                flags: DcFlags::NONE,
            },
            DcEntry {
                dc_id: 2,
                addr: "media".into(),
                auth_key: None,
                first_salt: 0,
                time_offset: 0,
                flags: DcFlags::MEDIA_ONLY,
            },
        ];
        let mut pool = DcPool::new(1, &entries, None, TransportKind::Abridged);
        let (home, _home_rx) = fake_slot();
        home.in_flight.store(1, Ordering::Relaxed);
        pool.conns.insert(1, vec![home]);
        assert!(!pool.should_expand(1));

        let (first, _first_rx) = fake_slot();
        first.in_flight.store(1, Ordering::Relaxed);
        pool.conns.insert(2, vec![first]);
        assert!(pool.should_expand(2));
        let (second, _second_rx) = fake_slot();
        pool.conns.get_mut(&2).unwrap().push(second.clone());
        assert!(!pool.should_expand(2));
        second.in_flight.store(1, Ordering::Relaxed);
        assert!(pool.should_expand(2));
        let (third, _third_rx) = fake_slot();
        third.in_flight.store(1, Ordering::Relaxed);
        pool.conns.get_mut(&2).unwrap().push(third);
        assert!(!pool.should_expand(2));

        // A media endpoint can share the home DC id without becoming a main
        // RPC session; file sessions are still allowed to grow.
        let mut home_media = entries[0].clone();
        home_media.flags = DcFlags::MEDIA_ONLY;
        pool.update_addrs(&[home_media]);
        assert!(pool.should_expand(1));
    }

    #[tokio::test]
    async fn failed_or_cancelled_setup_never_publishes_a_slot() {
        use ferogram_session::DcFlags;
        let entry = DcEntry {
            dc_id: 2,
            addr: "127.0.0.1:0".into(),
            auth_key: None,
            first_salt: 0,
            time_offset: 0,
            flags: DcFlags::MEDIA_ONLY,
        };
        let pool = Arc::new(Mutex::new(DcPool::new(
            1,
            &[entry],
            None,
            TransportKind::Abridged,
        )));
        let failed =
            DcPool::insert_after_setup(&pool, 2, async { Err(InvocationError::Dropped) }).await;
        assert!(failed.is_err());
        assert!(pool.lock().await.reserve_slot(2).is_err());

        let pending_pool = pool.clone();
        let (entered_tx, entered_rx) = oneshot::channel();
        let setup = tokio::spawn(async move {
            DcPool::insert_after_setup(&pending_pool, 2, async {
                let _ = entered_tx.send(());
                std::future::pending::<Result<DcConnection, InvocationError>>().await
            })
            .await
        });
        entered_rx.await.expect("setup started");
        setup.abort();
        setup.await.expect_err("setup cancelled");
        assert!(pool.lock().await.reserve_slot(2).is_err());
        assert!(!pool.lock().await.has_connection(2));
    }
}
