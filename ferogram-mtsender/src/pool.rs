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

use crate::errors::InvocationError;
use crate::sender::DcConnection;
use crate::sender_task::{FrameEvent, RpcEnqueue, spawn_sender_task};
use ferogram_connect::util::maybe_gz_pack;
use ferogram_connect::{Socks5Config, TransportKind};
use ferogram_session::DcEntry;
use ferogram_tl_types::{RemoteCall, Serializable};
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use tokio::sync::{mpsc, oneshot};

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

/// Pool of per-DC authenticated connections.
/// Each DC holds up to MAX_CONNS_PER_DC slots. The pool lock is dropped
/// before any network I/O so concurrent callers don't serialize on it.
pub struct DcPool {
    /// Per-DC connection slots; inner Vec holds slot Arcs.
    pub conns: HashMap<i32, Vec<Arc<ConnSlot>>>,
    addrs: HashMap<i32, String>,
    #[allow(dead_code)]
    home_dc_id: i32,
    /// Proxy config forwarded to auto-reconnect.
    socks5: Option<Socks5Config>,
    /// Transport kind reused for secondary DC connections.
    transport: TransportKind,
    /// DCs that have already received `invokeWithLayer(initConnection(...))`.
    init_done: std::collections::HashSet<i32>,
}

impl DcPool {
    /// Build an empty pool for `home_dc_id`, seeded with addresses for every
    /// DC in `dc_entries`. No connections are opened yet; slots get created
    /// lazily on first use of each DC.
    pub fn new(
        home_dc_id: i32,
        dc_entries: &[DcEntry],
        socks5: Option<Socks5Config>,
        transport: TransportKind,
    ) -> Self {
        let addrs = dc_entries
            .iter()
            .map(|e| (e.dc_id, e.addr.clone()))
            .collect();
        Self {
            conns: HashMap::new(),
            addrs,
            home_dc_id,
            socks5,
            transport,
            init_done: std::collections::HashSet::new(),
        }
    }

    /// Returns true if at least one connection slot exists for `dc_id`.
    pub fn has_connection(&self, dc_id: i32) -> bool {
        self.conns.get(&dc_id).is_some_and(|v| !v.is_empty())
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
        self.conns.entry(dc_id).or_default().push(slot);
        let total: usize = self.conns.values().map(|v| v.len()).sum();
        metrics::gauge!("ferogram.connections_active").set(total as f64);
    }

    /// Returns the least-loaded slot for `dc_id`, creating one if needed.
    /// Creates a new slot if all existing ones are busy and count < MAX_CONNS_PER_DC.
    /// Drop the DcPool guard before locking the returned slot.
    pub(crate) async fn get_or_create_slot(
        &mut self,
        dc_id: i32,
        pfs: bool,
        auth_key: Option<([u8; 256], i64, i32)>,
    ) -> Result<Arc<ConnSlot>, InvocationError> {
        let addr = self.addrs.get(&dc_id).cloned().ok_or_else(|| {
            InvocationError::Deserialize(format!("dc_pool: no address for DC{dc_id}"))
        })?;

        // Ensure at least one slot exists.
        if !self.conns.contains_key(&dc_id) || self.conns[&dc_id].is_empty() {
            tracing::debug!("[ferogram::pool] opening first connection to DC{dc_id} at {addr}");
            let conn = if let Some((key, salt, offset)) = auth_key {
                DcConnection::connect_with_key(
                    &addr,
                    key,
                    salt,
                    offset,
                    self.socks5.as_ref(),
                    None,
                    &self.transport,
                    dc_id as i16,
                    pfs,
                )
                .await?
            } else {
                DcConnection::connect_raw(
                    &addr,
                    self.socks5.as_ref(),
                    &self.transport,
                    dc_id as i16,
                )
                .await?
            };
            let slot = Self::spawn_slot(conn);
            self.conns.entry(dc_id).or_default().push(slot);
            self.init_done.remove(&dc_id);
            let total: usize = self.conns.values().map(|v| v.len()).sum();
            metrics::gauge!("ferogram.connections_active").set(total as f64);
        }

        let slots = self
            .conns
            .get(&dc_id)
            .expect("dc_id must be registered before use");

        // pick least-busy slot
        let best = slots
            .iter()
            .min_by_key(|s| s.in_flight.load(Ordering::Relaxed))
            .expect("slots vec is non-empty")
            .clone();
        let min_inflight = best.in_flight.load(Ordering::Relaxed);

        // Spawn a new slot if: all are busy AND we have room for more.
        //
        // With pipelined slots this matters less than it used to (a single
        // slot can now happily carry many in-flight requests at once), but
        // it's still worth spreading load across a few real TCP connections
        // for very heavy transfers.
        if min_inflight > 0 && slots.len() < MAX_CONNS_PER_DC {
            tracing::debug!(
                "[ferogram::pool] DC{dc_id}: all {} slots busy (min_inflight={min_inflight}), opening extra connection",
                slots.len()
            );
            let conn = if let Some((key, salt, offset)) = auth_key {
                DcConnection::connect_with_key(
                    &addr,
                    key,
                    salt,
                    offset,
                    self.socks5.as_ref(),
                    None,
                    &self.transport,
                    dc_id as i16,
                    pfs,
                )
                .await?
            } else {
                DcConnection::connect_raw(
                    &addr,
                    self.socks5.as_ref(),
                    &self.transport,
                    dc_id as i16,
                )
                .await?
            };
            let new_slot = Self::spawn_slot(conn);
            let arc = new_slot.clone();
            self.conns
                .get_mut(&dc_id)
                .expect("dc_id must be registered")
                .push(new_slot);
            let total: usize = self.conns.values().map(|v| v.len()).sum();
            metrics::gauge!("ferogram.connections_active").set(total as f64);
            return Ok(arc);
        }

        Ok(best)
    }

    /// Evict all slots for a DC (called on connection failure to force
    /// reconnection on the next call).
    pub fn evict(&mut self, dc_id: i32) {
        self.conns.remove(&dc_id);
        self.init_done.remove(&dc_id);
        let total: usize = self.conns.values().map(|v| v.len()).sum();
        metrics::gauge!("ferogram.connections_active").set(total as f64);
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
        slot.in_flight.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = oneshot::channel();
        let send_result = slot.rpc_tx.send(RpcEnqueue { body, tx }).await;
        let result = if send_result.is_err() {
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
        };
        slot.in_flight.fetch_sub(1, Ordering::Relaxed);
        result
    }

    /// Invoke a raw RPC call on the given DC.
    /// Pool lock is released before the network round-trip begins.
    pub async fn invoke_on_dc<R: RemoteCall>(
        &mut self,
        dc_id: i32,
        _dc_entries: &[DcEntry],
        req: &R,
    ) -> Result<Vec<u8>, InvocationError> {
        let slot = self.get_or_create_slot(dc_id, false, None).await?;
        let body = maybe_gz_pack(&req.to_bytes());
        let result = Self::send_via_slot(&slot, body.clone()).await;

        if let Err(ref e) = result {
            let kind = match e {
                InvocationError::Rpc(_) => "rpc",
                InvocationError::Io(_) => "io",
                _ => "other",
            };
            metrics::counter!("ferogram.rpc_errors_total", "kind" => kind).increment(1);
        }

        if let Err(InvocationError::Rpc(ref e)) = result
            && e.code == -404
        {
            // Telegram dropped the auth key (e.g. AndroidTV killed the socket during sleep).
            // Evict and redo a full DH exchange; the login session is still valid server-side.
            tracing::warn!(
                "[ferogram::pool] DC{dc_id} returned -404 (auth key gone); evicting and redoing DH"
            );
            self.evict(dc_id);
            let retry_slot = self.get_or_create_slot(dc_id, false, None).await?;
            return Self::send_via_slot(&retry_slot, body).await;
        }

        if result.is_err() && !slot.alive.load(Ordering::Acquire) {
            tracing::warn!(
                "[ferogram::pool] DC{dc_id} connection died mid-request; evicting and retrying on a fresh connection"
            );
            self.evict(dc_id);
            let retry_slot = self.get_or_create_slot(dc_id, false, None).await?;
            return Self::send_via_slot(&retry_slot, body).await;
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
    pub async fn invoke_on_dc_serializable<S: Serializable>(
        &mut self,
        dc_id: i32,
        req: &S,
    ) -> Result<Vec<u8>, InvocationError> {
        let slot = self
            .get_or_create_slot(dc_id, false, None)
            .await
            .map_err(|_| InvocationError::Deserialize(format!("no connection for DC{dc_id}")))?;
        let body = maybe_gz_pack(&req.to_bytes());
        let result = Self::send_via_slot(&slot, body.clone()).await;

        if let Err(InvocationError::Rpc(ref e)) = result
            && e.code == -404
        {
            tracing::warn!(
                "[ferogram::pool] DC{dc_id} returned -404 (serializable path); evicting and redoing DH"
            );
            self.evict(dc_id);
            let retry_slot = self.get_or_create_slot(dc_id, false, None).await?;
            return Self::send_via_slot(&retry_slot, body).await;
        }

        if result.is_err() && !slot.alive.load(Ordering::Acquire) {
            tracing::warn!(
                "[ferogram::pool] DC{dc_id} connection died mid-request (serializable path); evicting and retrying"
            );
            self.evict(dc_id);
            let retry_slot = self.get_or_create_slot(dc_id, false, None).await?;
            return Self::send_via_slot(&retry_slot, body).await;
        }
        result
    }

    /// Update the address table (called after `initConnection`).
    pub fn update_addrs(&mut self, entries: &[DcEntry]) {
        for e in entries {
            self.addrs.insert(e.dc_id, e.addr.clone());
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
