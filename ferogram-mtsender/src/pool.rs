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
use ferogram_connect::{Socks5Config, TransportKind};
use ferogram_session::DcEntry;
use ferogram_tl_types::RemoteCall;
use std::collections::HashMap;

// Max simultaneous connections per DC.
const MAX_CONNS_PER_DC: usize = 3;

/// One slot in the per-DC connection pool.
/// `in_flight` lets the pool pick the least-busy slot without locking it.
pub struct ConnSlot {
    pub conn: tokio::sync::Mutex<DcConnection>,
    pub in_flight: std::sync::atomic::AtomicUsize,
}

/// Pool of per-DC authenticated connections.
/// Each DC holds up to MAX_CONNS_PER_DC slots. The pool lock is dropped
/// before any network I/O so concurrent callers don't serialize on it.
pub struct DcPool {
    /// Per-DC connection slots; inner Vec holds slot Arcs.
    pub conns: HashMap<i32, Vec<std::sync::Arc<ConnSlot>>>,
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

    /// Insert a pre-built connection into the pool as a new slot.
    pub fn insert(&mut self, dc_id: i32, conn: DcConnection) {
        let slot = std::sync::Arc::new(ConnSlot {
            conn: tokio::sync::Mutex::new(conn),
            in_flight: std::sync::atomic::AtomicUsize::new(0),
        });
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
    ) -> Result<std::sync::Arc<ConnSlot>, InvocationError> {
        use std::sync::atomic::Ordering;

        let addr = self.addrs.get(&dc_id).cloned().ok_or_else(|| {
            InvocationError::Deserialize(format!("dc_pool: no address for DC{dc_id}"))
        })?;

        // Ensure at least one slot exists.
        if !self.conns.contains_key(&dc_id) || self.conns[&dc_id].is_empty() {
            tracing::info!("[dc_pool] auto-connecting DC{dc_id} ({addr})");
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
            let slot = std::sync::Arc::new(ConnSlot {
                conn: tokio::sync::Mutex::new(conn),
                in_flight: std::sync::atomic::AtomicUsize::new(0),
            });
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
        if min_inflight > 0 && slots.len() < MAX_CONNS_PER_DC {
            tracing::debug!(
                "[dc_pool] DC{dc_id}: all {} slots busy (min_inflight={}), opening new slot",
                slots.len(),
                min_inflight
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
            let new_slot = std::sync::Arc::new(ConnSlot {
                conn: tokio::sync::Mutex::new(conn),
                in_flight: std::sync::atomic::AtomicUsize::new(0),
            });
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

    /// Evict all slots for a DC (called on IO error to force reconnection).
    pub fn evict(&mut self, dc_id: i32) {
        self.conns.remove(&dc_id);
        self.init_done.remove(&dc_id);
        let total: usize = self.conns.values().map(|v| v.len()).sum();
        metrics::gauge!("ferogram.connections_active").set(total as f64);
        tracing::debug!("[dc_pool] evicted all slots for DC{dc_id}");
    }

    /// Invoke a raw RPC call on the given DC.
    /// Pool lock is released before the network round-trip begins.
    pub async fn invoke_on_dc<R: RemoteCall>(
        &mut self,
        dc_id: i32,
        _dc_entries: &[DcEntry],
        req: &R,
    ) -> Result<Vec<u8>, InvocationError> {
        use std::sync::atomic::Ordering;
        let slot = self.get_or_create_slot(dc_id, false, None).await?;
        slot.in_flight.fetch_add(1, Ordering::Relaxed);
        let result = slot.conn.lock().await.rpc_call(req).await;
        slot.in_flight.fetch_sub(1, Ordering::Relaxed);
        if let Err(ref e) = result {
            let kind = match e {
                InvocationError::Rpc(_) => "rpc",
                InvocationError::Io(_) => "io",
                _ => "other",
            };
            metrics::counter!("ferogram.rpc_errors_total", "kind" => kind).increment(1);
        }
        if matches!(result, Err(InvocationError::Io(_))) {
            tracing::warn!("[dc_pool] IO error on DC{dc_id}, evicting all slots and retrying");
            self.evict(dc_id);
            let retry_slot = self.get_or_create_slot(dc_id, false, None).await?;
            retry_slot.in_flight.fetch_add(1, Ordering::Relaxed);
            let r = retry_slot.conn.lock().await.rpc_call(req).await;
            retry_slot.in_flight.fetch_sub(1, Ordering::Relaxed);
            return r;
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
    pub async fn invoke_on_dc_serializable<S: ferogram_tl_types::Serializable>(
        &mut self,
        dc_id: i32,
        req: &S,
    ) -> Result<Vec<u8>, InvocationError> {
        use std::sync::atomic::Ordering;
        let slot = self
            .get_or_create_slot(dc_id, false, None)
            .await
            .map_err(|_| InvocationError::Deserialize(format!("no connection for DC{dc_id}")))?;
        slot.in_flight.fetch_add(1, Ordering::Relaxed);
        let result = slot.conn.lock().await.rpc_call_serializable(req).await;
        slot.in_flight.fetch_sub(1, Ordering::Relaxed);
        if matches!(result, Err(InvocationError::Io(_))) {
            tracing::warn!("[dc_pool] serializable IO error on DC{dc_id}, evicting and retrying");
            self.evict(dc_id);
            let retry_slot = self.get_or_create_slot(dc_id, false, None).await?;
            retry_slot.in_flight.fetch_add(1, Ordering::Relaxed);
            let r = retry_slot
                .conn
                .lock()
                .await
                .rpc_call_serializable(req)
                .await;
            retry_slot.in_flight.fetch_sub(1, Ordering::Relaxed);
            return r;
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
                && let Ok(conn) = slot.conn.try_lock()
            {
                e.auth_key = Some(conn.auth_key_bytes());
                e.first_salt = conn.first_salt();
                e.time_offset = conn.time_offset();
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
