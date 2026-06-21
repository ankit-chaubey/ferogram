// Copyright (c) Ankit Chaubey <ankitchaubey.dev@gmail.com>
//
// Licensed under either the MIT License or the Apache License 2.0.

//! Single-task MTProto sender.
//!
//! All TCP I/O (read + write + ping) happens inside ONE task that owns the
//! unsplit `TcpStream`.  Callers enqueue request bodies via [`MtpSender::enqueue`]
//! and get results via a oneshot channel.  The task loops calling
//! [`MtpSender::step`] which does ONE I/O event per call (read | write | ping).
//!
//! This eliminates every problem caused by the old split-reader/writer model:
//!   - No `Mutex<OwnedWriteHalf>` contention.
//!   - ACKs always flushed on every outgoing frame.
//!   - No `diff_in_flight` gate: diffs are tasks that enqueue like any RPC.
//!   - No self-deadlock: the same task that sends a request also reads the reply.
//!   - On connection error `fail_all()` immediately resolves every pending oneshot.

use std::collections::VecDeque;
use std::time::Duration;

use ferogram_connect::util::{
    build_container_body, build_msgs_ack_body, crc32_ieee, random_i64, tl_read_string,
};
use ferogram_connect::{FrameKind, FutureSalt};
use ferogram_mtproto::EncryptedSession;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::oneshot;
use tokio::time::Instant;

use crate::errors::InvocationError;
use crate::pool::build_msgs_ack_ping_body;

const PING_DELAY: Duration = Duration::from_secs(60);
const READ_BUF_CAP: usize = (1024 * 1024) + (8 * 1024);

#[derive(Debug)]
enum MsgState {
    /// Not yet serialised into the write buffer.
    Pending,
    /// Serialised: msg_id known, write not yet complete.
    Serialised { msg_id: i64, container_msg_id: i64 },
    /// Write complete: waiting for rpc_result from the server.
    Sent { msg_id: i64, container_msg_id: i64 },
}

struct Request {
    body: Vec<u8>,
    state: MsgState,
    tx: oneshot::Sender<Result<Vec<u8>, InvocationError>>,
}

enum StepOutcome {
    Frames,
    Wrote(usize),
    Ping,
}

pub struct MtpSender {
    stream: TcpStream,
    pub enc: EncryptedSession,
    pub frame_kind: FrameKind,
    pub perm_auth_key: Option<[u8; 256]>,

    requests: VecDeque<Request>,

    /// Received msg_ids waiting to be ACKed on the next outgoing frame.
    pending_ack: Vec<i64>,

    /// Bodies queued for resend (bad_msg / bad_server_salt).
    resend_queue: Vec<Vec<u8>>,

    /// Read buffer.  We append into `[..tail]` and peel frames from the front.
    read_buf: Box<[u8]>,
    read_tail: usize,

    /// Write buffer + cursor (bytes before `write_head` have been sent).
    write_buf: Vec<u8>,
    write_head: usize,

    next_ping: Instant,

    pub salts: Vec<FutureSalt>,
    pub start_salt_time: Option<(i32, std::time::Instant)>,
}

impl MtpSender {
    pub fn new(
        stream: TcpStream,
        enc: EncryptedSession,
        frame_kind: FrameKind,
        perm_auth_key: Option<[u8; 256]>,
    ) -> Self {
        Self {
            stream,
            enc,
            frame_kind,
            perm_auth_key,
            requests: VecDeque::new(),
            pending_ack: Vec::new(),
            resend_queue: Vec::new(),
            read_buf: vec![0u8; READ_BUF_CAP].into_boxed_slice(),
            read_tail: 0,
            write_buf: Vec::with_capacity(64 * 1024),
            write_head: 0,
            next_ping: Instant::now() + PING_DELAY,
            salts: Vec::new(),
            start_salt_time: None,
        }
    }

    pub fn auth_key_bytes(&self) -> [u8; 256] {
        self.perm_auth_key
            .unwrap_or_else(|| self.enc.auth_key_bytes())
    }
    pub fn first_salt(&self) -> i64 {
        self.enc.salt
    }
    pub fn time_offset(&self) -> i32 {
        self.enc.time_offset
    }
    pub fn session_id(&self) -> i64 {
        self.enc.session_id()
    }

    /// Enqueue a pre-serialised TL body.  The caller awaits `rx` for the result.
    pub fn enqueue(
        &mut self,
        body: Vec<u8>,
        tx: oneshot::Sender<Result<Vec<u8>, InvocationError>>,
    ) {
        self.requests.push_back(Request {
            body,
            state: MsgState::Pending,
            tx,
        });
    }

    /// Replace the TCP stream after a reconnect.
    ///
    /// All pending requests are reset to `Pending` so they are re-sent on the
    /// new connection.  I/O buffers and ACK queue are cleared.
    pub fn set_stream(
        &mut self,
        stream: TcpStream,
        enc: EncryptedSession,
        frame_kind: FrameKind,
        perm_auth_key: Option<[u8; 256]>,
    ) {
        self.stream = stream;
        self.enc = enc;
        self.frame_kind = frame_kind;
        self.perm_auth_key = perm_auth_key;
        self.read_tail = 0;
        self.write_buf.clear();
        self.write_head = 0;
        self.pending_ack.clear();
        self.resend_queue.clear();
        self.next_ping = Instant::now() + PING_DELAY;
        self.salts.clear();
        self.start_salt_time = None;
        for req in self.requests.iter_mut() {
            req.state = MsgState::Pending;
        }
    }

    /// Fail every pending request with `err` and clear I/O state.
    /// Call this immediately after a connection error before reconnecting.
    pub fn fail_all(&mut self, err: &InvocationError) {
        let msg = format!("{err:?}");
        for req in self.requests.drain(..) {
            let _ = req.tx.send(Err(InvocationError::Deserialize(msg.clone())));
        }
        self.write_buf.clear();
        self.write_head = 0;
        self.pending_ack.clear();
        self.resend_queue.clear();
    }

    /// Drive one network event: read | write | ping.
    ///
    /// Returns a list of raw TL bodies that should be dispatched as updates
    /// (anything that is not an rpc_result, bad_msg, new_session, etc.).
    ///
    /// Returns `Err` on I/O / transport / crypto failure.  The caller must call
    /// `fail_all()` then reconnect.
    pub async fn step(&mut self) -> Result<Vec<Vec<u8>>, InvocationError> {
        self.try_fill_write();

        let has_write = self.write_head < self.write_buf.len();

        // Split the stream borrows for the select! (avoids double &mut self.stream).
        let (mut rh, mut wh) = self.stream.split();

        let result = tokio::select! {
            biased;

            result = rh.read(&mut self.read_buf[self.read_tail..]) => {
                let n = result.map_err(InvocationError::Io)?;
                if n == 0 {
                    return Err(InvocationError::Io(std::io::Error::new(
                        std::io::ErrorKind::ConnectionReset,
                        "server closed connection",
                    )));
                }
                self.read_tail += n;
                Ok::<_, InvocationError>(StepOutcome::Frames)
            }

            result = wh.write(&self.write_buf[self.write_head..]),
                if has_write =>
            {
                let n = result.map_err(InvocationError::Io)?;
                Ok(StepOutcome::Wrote(n))
            }

            _ = tokio::time::sleep_until(self.next_ping) => {
                Ok(StepOutcome::Ping)
            }
        }?;

        // rh/wh borrows end here; self is fully accessible again.
        match result {
            StepOutcome::Frames => self.drain_frames(),
            StepOutcome::Wrote(n) => {
                self.write_head += n;
                if self.write_head >= self.write_buf.len() {
                    self.write_buf.clear();
                    self.write_head = 0;
                    for req in self.requests.iter_mut() {
                        if let MsgState::Serialised {
                            msg_id,
                            container_msg_id,
                        } = req.state
                        {
                            req.state = MsgState::Sent {
                                msg_id,
                                container_msg_id,
                            };
                        }
                    }
                }
                Ok(vec![])
            }
            StepOutcome::Ping => {
                let body = build_msgs_ack_ping_body(random_i64());
                let (tx, _rx) = oneshot::channel();
                self.enqueue(body, tx);
                self.next_ping = Instant::now() + PING_DELAY;
                Ok(vec![])
            }
        }
    }

    /// Pack pending requests (+ ACKs + resends) into one encrypted frame.
    fn try_fill_write(&mut self) {
        // Only fill when the previous frame has been fully sent.
        if self.write_head < self.write_buf.len() {
            return;
        }
        self.write_buf.clear();
        self.write_head = 0;

        // Collect (body, content_related) pairs for this frame.
        let mut msgs: Vec<(Vec<u8>, bool)> = Vec::new();

        // 1. Pending ACKs (highest priority, non-content-related).
        if !self.pending_ack.is_empty() {
            msgs.push((build_msgs_ack_body(&self.pending_ack), false));
            self.pending_ack.clear();
        }

        // 2. Resend queue.
        for body in self.resend_queue.drain(..) {
            msgs.push((body, true));
        }

        // 3. New pending requests.
        for req in self.requests.iter_mut() {
            if matches!(req.state, MsgState::Pending) {
                msgs.push((req.body.clone(), true));
            }
        }

        if msgs.is_empty() {
            return;
        }

        let wire = if msgs.len() == 1 {
            // Single message, no container needed.
            let (body, content_related) = &msgs[0];
            let (wire, msg_id) = self.enc.pack_body_with_msg_id(body, *content_related);
            self.mark_serialised(body, msg_id, msg_id);
            wire
        } else {
            // Build msg_container.
            // alloc_msg_seqno advances the counter for each message in the container.
            let mut inner: Vec<(i64, i32, Vec<u8>)> = Vec::with_capacity(msgs.len());
            for (body, content_related) in &msgs {
                let (msg_id, seqno) = self.enc.alloc_msg_seqno(*content_related);
                inner.push((msg_id, seqno, body.clone()));
            }
            let container_body: Vec<(i64, i32, &[u8])> = inner
                .iter()
                .map(|(id, seq, b)| (*id, *seq, b.as_slice()))
                .collect();
            let raw_container = build_container_body(&container_body);
            let (wire, container_msg_id) = self.enc.pack_container(&raw_container);
            // Mark each pending request with its msg_id.
            for (msg_id, _, body) in &inner {
                self.mark_serialised(body, *msg_id, container_msg_id);
            }
            wire
        };

        self.write_buf = self.frame_encode(&wire);
    }

    /// Find the Pending request with matching body and advance to Serialised.
    fn mark_serialised(&mut self, body: &[u8], msg_id: i64, container_msg_id: i64) {
        for req in self.requests.iter_mut() {
            if matches!(req.state, MsgState::Pending) && req.body == *body {
                req.state = MsgState::Serialised {
                    msg_id,
                    container_msg_id,
                };
                return;
            }
        }
    }

    /// Peel and process all complete frames from the read buffer.
    fn drain_frames(&mut self) -> Result<Vec<Vec<u8>>, InvocationError> {
        let mut updates = Vec::new();
        let mut offset = 0usize;

        loop {
            match self.peel_one(offset) {
                Peel::Complete { payload, end } => {
                    offset = end;
                    match self.process_payload(payload) {
                        Ok(mut u) => updates.append(&mut u),
                        Err(e) => {
                            self.consume_read(offset);
                            return Err(e);
                        }
                    }
                }
                Peel::Incomplete => break,
                Peel::Err(e) => {
                    self.consume_read(offset);
                    return Err(e);
                }
            }
        }
        self.consume_read(offset);
        Ok(updates)
    }

    /// Shift consumed bytes out of the read buffer.
    fn consume_read(&mut self, consumed: usize) {
        if consumed > 0 && consumed <= self.read_tail {
            self.read_buf.copy_within(consumed..self.read_tail, 0);
            self.read_tail -= consumed;
        }
    }

    /// Decrypt one complete payload and dispatch its body.
    fn process_payload(&mut self, mut payload: Vec<u8>) -> Result<Vec<Vec<u8>>, InvocationError> {
        let msg = self
            .enc
            .unpack(&mut payload)
            .map_err(|e| InvocationError::Deserialize(format!("decrypt: {e:?}")))?;

        // Every received content-related message must be ACKed.
        if msg.msg_id & 1 == 1 {
            self.pending_ack.push(msg.msg_id);
        }

        self.dispatch(&msg.body, msg.msg_id)
    }

    /// Route one decrypted message body. Returns raw update bodies.
    fn dispatch(&mut self, body: &[u8], msg_id: i64) -> Result<Vec<Vec<u8>>, InvocationError> {
        if body.len() < 4 {
            return Ok(vec![]);
        }
        let cid = u32::from_le_bytes(body[..4].try_into().unwrap());
        tracing::trace!(
            "[mtp_sender] dispatch: ctor={cid:#010x} msg_id={msg_id:#x} len={}",
            body.len()
        );

        match cid {
            // rpc_result#f35c6d01
            0xf35c6d01 => {
                if body.len() < 12 {
                    return Ok(vec![]);
                }
                let req_msg_id = i64::from_le_bytes(body[4..12].try_into().unwrap());
                let mut result = body[12..].to_vec();
                // The RPC result payload itself may be gzip_packed#3072cfa1-wrapped
                // (Telegram does this for large responses such as
                // getDifference/getChannelDifference). Unlike updates pushed
                // through `dispatch`, this inner payload is the final answer
                // handed to the RPC caller, not something we recurse into - so
                // it must be unwrapped here, otherwise the caller tries to
                // deserialize the still-compressed bytes and fails with
                // "unexpected constructor id: 0x3072cfa1".
                if result.len() >= 4
                    && u32::from_le_bytes(result[..4].try_into().unwrap()) == 0x3072cfa1
                {
                    use ferogram_connect::util::{gz_inflate, tl_read_bytes};
                    if let Some(compressed) = tl_read_bytes(&result[4..])
                        && let Ok(decompressed) = gz_inflate(&compressed)
                    {
                        result = decompressed;
                    }
                }
                // rpc_error#2144ca19 error_code:int error_message:string
                // Telegram wraps RPC errors inside rpc_result just like any other
                // reply, so they must be unwrapped here. Otherwise the raw
                // error_code/error_message bytes get handed to the caller as if
                // they were the expected response type, which then fails with a
                // confusing "unexpected constructor id" instead of the real error.
                let outcome = if result.len() >= 8
                    && u32::from_le_bytes(result[..4].try_into().unwrap()) == 0x2144ca19
                {
                    let code = i32::from_le_bytes(result[4..8].try_into().unwrap());
                    let message = tl_read_string(&result[8..]).unwrap_or_default();
                    Err(InvocationError::Rpc(
                        crate::errors::RpcError::from_telegram(code, &message),
                    ))
                } else {
                    Ok(result)
                };
                self.resolve(req_msg_id, outcome);
                Ok(vec![])
            }

            // msg_container#73f1f8dc
            0x73f1f8dc => {
                if body.len() < 8 {
                    return Ok(vec![]);
                }
                let count = u32::from_le_bytes(body[4..8].try_into().unwrap()) as usize;
                let mut updates = Vec::new();
                let mut pos = 8usize;
                for _ in 0..count {
                    if pos + 16 > body.len() {
                        break;
                    }
                    let inner_msg_id = i64::from_le_bytes(body[pos..pos + 8].try_into().unwrap());
                    let _seqno = i32::from_le_bytes(body[pos + 8..pos + 12].try_into().unwrap());
                    let bytes =
                        u32::from_le_bytes(body[pos + 12..pos + 16].try_into().unwrap()) as usize;
                    pos += 16;
                    if pos + bytes > body.len() {
                        break;
                    }
                    let inner = body[pos..pos + bytes].to_vec();
                    pos += bytes;
                    if inner_msg_id & 1 == 1 {
                        self.pending_ack.push(inner_msg_id);
                    }
                    let mut u = self.dispatch(&inner, inner_msg_id)?;
                    updates.append(&mut u);
                }
                Ok(updates)
            }

            // gzip_packed#3072cfa1
            0x3072cfa1 => {
                use ferogram_connect::util::{gz_inflate, tl_read_bytes};
                if let Some(compressed) = tl_read_bytes(&body[4..])
                    && let Ok(decompressed) = gz_inflate(&compressed)
                {
                    return self.dispatch(&decompressed, msg_id);
                }
                Ok(vec![])
            }

            // bad_server_salt#edab447b
            0xedab447b => {
                if body.len() >= 28 {
                    let bad_msg_id = i64::from_le_bytes(body[4..12].try_into().unwrap());
                    let new_salt = i64::from_le_bytes(body[20..28].try_into().unwrap());
                    tracing::debug!(
                        "[mtp_sender] bad_server_salt: bad={bad_msg_id:#018x} new_salt={new_salt:#018x}"
                    );
                    self.enc.salt = new_salt;
                    self.queue_resend(bad_msg_id);
                }
                Ok(vec![])
            }

            // bad_msg_notification#a7eff811
            0xa7eff811 => {
                if body.len() >= 20 {
                    let bad_msg_id = i64::from_le_bytes(body[4..12].try_into().unwrap());
                    let error_code = u32::from_le_bytes(body[16..20].try_into().unwrap());
                    tracing::debug!(
                        "[mtp_sender] bad_msg_notification: bad={bad_msg_id:#018x} code={error_code}"
                    );
                    match error_code {
                        16 | 17 => {
                            self.enc.correct_time_offset(msg_id);
                            self.queue_resend(bad_msg_id);
                        }
                        32 | 33 => {
                            self.enc.correct_seq_no(error_code);
                            self.queue_resend(bad_msg_id);
                        }
                        _ => {
                            self.queue_resend(bad_msg_id);
                        }
                    }
                }
                Ok(vec![])
            }

            // new_session_created#9ec20908
            0x9ec20908 => {
                if body.len() >= 28 {
                    let new_salt = i64::from_le_bytes(body[20..28].try_into().unwrap());
                    tracing::debug!("[mtp_sender] new_session_created salt={new_salt:#018x}");
                    self.enc.salt = new_salt;
                    // Server lost our session: re-queue all sent requests.
                    for req in self.requests.iter_mut() {
                        if matches!(
                            req.state,
                            MsgState::Sent { .. } | MsgState::Serialised { .. }
                        ) {
                            req.state = MsgState::Pending;
                        }
                    }
                }
                Ok(vec![])
            }

            // msgs_ack#62d6b459: server ACKing our messages; nothing to do.
            0x62d6b459 => Ok(vec![]),

            // pong#347773c5: response to PingDelayDisconnect.
            0x347773c5 => {
                if body.len() >= 12 {
                    let pong_req_id = i64::from_le_bytes(body[4..12].try_into().unwrap());
                    self.resolve(pong_req_id, Ok(body.to_vec()));
                }
                Ok(vec![])
            }

            // Everything else (Updates, UpdateShort, etc.) → return as update body.
            _ => Ok(vec![body.to_vec()]),
        }
    }

    /// Fulfill the oneshot for the request with `req_msg_id`.
    fn resolve(&mut self, req_msg_id: i64, result: Result<Vec<u8>, InvocationError>) {
        // Check by msg_id first.
        if let Some(i) = self.requests.iter().position(|r| match &r.state {
            MsgState::Sent { msg_id, .. } => *msg_id == req_msg_id,
            _ => false,
        }) {
            let req = self.requests.remove(i).unwrap();
            let _ = req.tx.send(result);
            return;
        }
        // Fall back to container_msg_id.
        if let Some(i) = self.requests.iter().position(|r| match &r.state {
            MsgState::Sent {
                container_msg_id, ..
            } => *container_msg_id == req_msg_id,
            _ => false,
        }) {
            let req = self.requests.remove(i).unwrap();
            let _ = req.tx.send(result);
        }
    }

    /// Re-queue the request with `bad_msg_id` for resend.
    fn queue_resend(&mut self, bad_msg_id: i64) {
        if let Some(req) = self.requests.iter_mut().find(|r| match &r.state {
            MsgState::Sent { msg_id, .. } | MsgState::Serialised { msg_id, .. } => {
                *msg_id == bad_msg_id
            }
            _ => false,
        }) {
            tracing::debug!("[mtp_sender] queuing resend for {bad_msg_id:#018x}");
            req.state = MsgState::Pending;
        }
    }

    /// Encode encrypted bytes into wire-ready transport frames.
    fn frame_encode(&self, data: &[u8]) -> Vec<u8> {
        match &self.frame_kind {
            FrameKind::Full { send_seqno, .. } => {
                let seq = send_seqno.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                let total_len = (data.len() as u32) + 12;
                let mut pkt = Vec::with_capacity(total_len as usize);
                pkt.extend_from_slice(&total_len.to_le_bytes());
                pkt.extend_from_slice(&seq.to_le_bytes());
                pkt.extend_from_slice(data);
                let crc = crc32_ieee(&pkt);
                pkt.extend_from_slice(&crc.to_le_bytes());
                pkt
            }
            FrameKind::Abridged => abridged_frame(data),
            FrameKind::Intermediate => {
                let mut f = Vec::with_capacity(4 + data.len());
                f.extend_from_slice(&(data.len() as u32).to_le_bytes());
                f.extend_from_slice(data);
                f
            }
            FrameKind::Obfuscated { cipher } => {
                let mut f = abridged_frame(data);
                // Synchronous lock is fine: we are the only task touching this.
                if let Ok(mut c) = cipher.try_lock() {
                    c.encrypt(&mut f);
                }
                f
            }
            FrameKind::PaddedIntermediate { cipher } => {
                let mut pad_buf = [0u8; 1];
                ferogram_crypto::fill_random(&mut pad_buf);
                let pad_len = (pad_buf[0] & 0x0f) as usize;
                let total = data.len() + pad_len;
                let mut f = Vec::with_capacity(4 + total);
                f.extend_from_slice(&(total as u32).to_le_bytes());
                f.extend_from_slice(data);
                let mut pad = vec![0u8; pad_len];
                ferogram_crypto::fill_random(&mut pad);
                f.extend_from_slice(&pad);
                if let Ok(mut c) = cipher.try_lock() {
                    c.encrypt(&mut f);
                }
                f
            }
            FrameKind::FakeTls { cipher } => {
                const TLS_APP_DATA: u8 = 0x17;
                const CHUNK: usize = 2878;
                let mut out = Vec::new();
                if let Ok(mut c) = cipher.try_lock() {
                    for chunk in data.chunks(CHUNK) {
                        let len = chunk.len() as u16;
                        let mut rec = Vec::with_capacity(5 + chunk.len());
                        rec.push(TLS_APP_DATA);
                        rec.extend_from_slice(&[0x03, 0x03]);
                        rec.extend_from_slice(&len.to_be_bytes());
                        rec.extend_from_slice(chunk);
                        c.encrypt(&mut rec[5..]);
                        out.extend_from_slice(&rec);
                    }
                }
                out
            }
        }
    }

    /// Try to extract one complete transport frame starting at `offset`.
    fn peel_one(&self, offset: usize) -> Peel {
        let buf = &self.read_buf[offset..self.read_tail];
        match &self.frame_kind {
            FrameKind::Full { recv_seqno, .. } => peel_full(buf, offset, recv_seqno),
            FrameKind::Abridged | FrameKind::Obfuscated { .. } => peel_abridged(buf, offset),
            _ => peel_intermediate(buf, offset),
        }
    }
}

enum Peel {
    Complete { payload: Vec<u8>, end: usize },
    Incomplete,
    Err(InvocationError),
}

fn abridged_frame(data: &[u8]) -> Vec<u8> {
    let words = data.len() / 4;
    let mut f = if words < 0x7f {
        let mut v = Vec::with_capacity(1 + data.len());
        v.push(words as u8);
        v
    } else {
        let mut v = Vec::with_capacity(4 + data.len());
        v.extend_from_slice(&[
            0x7f,
            (words & 0xff) as u8,
            ((words >> 8) & 0xff) as u8,
            ((words >> 16) & 0xff) as u8,
        ]);
        v
    };
    f.extend_from_slice(data);
    f
}

fn peel_abridged(buf: &[u8], base: usize) -> Peel {
    if buf.is_empty() {
        return Peel::Incomplete;
    }
    let (hdr, words) = if buf[0] < 0x7f {
        (1, buf[0] as usize)
    } else if buf[0] == 0x7f {
        if buf.len() < 4 {
            return Peel::Incomplete;
        }
        let w = buf[1] as usize | (buf[2] as usize) << 8 | (buf[3] as usize) << 16;
        (4, w)
    } else {
        if buf.len() < 4 {
            return Peel::Incomplete;
        }
        let code = i32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]);
        return Peel::Err(io_err(format!("transport code {code}")));
    };
    let payload_len = words * 4;
    if buf.len() < hdr + payload_len {
        return Peel::Incomplete;
    }
    Peel::Complete {
        payload: buf[hdr..hdr + payload_len].to_vec(),
        end: base + hdr + payload_len,
    }
}

fn peel_intermediate(buf: &[u8], base: usize) -> Peel {
    if buf.len() < 4 {
        return Peel::Incomplete;
    }
    let li = i32::from_le_bytes(buf[..4].try_into().unwrap());
    if li < 0 {
        return Peel::Err(io_err(format!("transport code {li}")));
    }
    let len = li as usize;
    if buf.len() < 4 + len {
        return Peel::Incomplete;
    }
    Peel::Complete {
        payload: buf[4..4 + len].to_vec(),
        end: base + 4 + len,
    }
}

fn peel_full(
    buf: &[u8],
    base: usize,
    recv_seqno: &std::sync::Arc<std::sync::atomic::AtomicU32>,
) -> Peel {
    if buf.len() < 4 {
        return Peel::Incomplete;
    }
    let li = i32::from_le_bytes(buf[..4].try_into().unwrap());
    if li < 0 {
        return Peel::Err(io_err(format!("Full transport code {li}")));
    }
    let total = li as usize;
    if total < 12 {
        return Peel::Err(InvocationError::Deserialize(format!(
            "Full: packet too short ({total})"
        )));
    }
    if buf.len() < total {
        return Peel::Incomplete;
    }

    // CRC check.
    let (body_and_seq, crc_bytes) = buf[..total].split_at(total - 4);
    let expected_crc = u32::from_le_bytes(crc_bytes.try_into().unwrap());
    let actual_crc = crc32_ieee(body_and_seq);
    if actual_crc != expected_crc {
        return Peel::Err(InvocationError::Deserialize(format!(
            "Full: CRC mismatch (got {actual_crc:#010x}, expected {expected_crc:#010x})"
        )));
    }

    // Seqno check (body_and_seq = [len(4)][seq(4)][payload...]).
    let recv_seq = i32::from_le_bytes(buf[4..8].try_into().unwrap());
    let expected_seq = recv_seqno.load(std::sync::atomic::Ordering::Relaxed) as i32;
    if recv_seq != expected_seq {
        return Peel::Err(InvocationError::Deserialize(format!(
            "Full: bad seq (got {recv_seq}, expected {expected_seq})"
        )));
    }
    recv_seqno.store(
        expected_seq.wrapping_add(1) as u32,
        std::sync::atomic::Ordering::Relaxed,
    );

    Peel::Complete {
        payload: buf[8..total - 4].to_vec(),
        end: base + total,
    }
}

fn io_err(msg: String) -> InvocationError {
    InvocationError::Io(std::io::Error::other(msg))
}
