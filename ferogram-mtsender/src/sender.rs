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

use ferogram_connect::FrameKind;
use ferogram_mtproto::{
    EncryptedSession, SeenMsgIds, Session, authentication as auth, new_seen_msg_ids, step2_temp,
};
use ferogram_tl_types as tl;
use ferogram_tl_types::{Cursor, Deserializable, RemoteCall};
use tokio::io::AsyncReadExt;
use tokio::net::TcpStream;

use crate::errors::InvocationError;
use crate::pool::{build_msgs_ack_body, build_msgs_ack_ping_body};
use ferogram_connect::TransportKind;
// metrics and tracing
#[allow(unused_imports)]
use metrics::{counter, histogram};

/// A single encrypted connection to one Telegram DC.
/// Un-acked server msg_ids to accumulate before eagerly flushing a `msgs_ack` frame.
const PENDING_ACKS_THRESHOLD: usize = 10;

/// `PingDelayDisconnect` interval for worker connections (in GetFile chunks).
/// Keeps the socket alive within Telegram's 75-second idle-disconnect window.
const PING_EVERY_N_CHUNKS: u32 = 5;

pub struct DcConnection {
    stream: TcpStream,
    enc: EncryptedSession,
    pending_acks: Vec<i64>,
    call_count: u32,
    /// Active framing kind for this connection.
    frame_kind: FrameKind,
    /// Persistent dedup ring that outlives individual EncryptedSessions.
    #[allow(dead_code)]
    seen_msg_ids: SeenMsgIds,
}

impl DcConnection {
    /// Races the default transport set (see `default_transport_race`).
    /// Use `connect_fastest_with` to pass a custom race.
    #[tracing::instrument(skip(socks5), fields(addr = %addr, dc_id = dc_id))]
    pub async fn connect_fastest(
        addr: &str,
        socks5: Option<&ferogram_connect::Socks5Config>,
        dc_id: i16,
    ) -> Result<(Self, String), InvocationError> {
        let race = ferogram_connect::default_transport_race();
        Self::connect_fastest_with(addr, socks5, dc_id, &race).await
    }

    /// Races the given transports in parallel, each after its stagger
    /// delay, and returns whichever finishes DH first. Others are cancelled.
    #[tracing::instrument(skip(socks5, race), fields(addr = %addr, dc_id = dc_id))]
    pub async fn connect_fastest_with(
        addr: &str,
        socks5: Option<&ferogram_connect::Socks5Config>,
        dc_id: i16,
        race: &[ferogram_connect::RaceLeg],
    ) -> Result<(Self, String), InvocationError> {
        use tokio::task::JoinSet;
        let addr = addr.to_owned();
        let socks5 = socks5.cloned();
        tracing::debug!(
            "[ferogram::sender] probing {addr} with {} transports in parallel: {:?}",
            race.len(),
            race.iter().map(|l| &l.transport).collect::<Vec<_>>()
        );
        let mut set: JoinSet<Result<(DcConnection, String), InvocationError>> = JoinSet::new();

        for leg in race {
            let a = addr.clone();
            let s = socks5.clone();
            let transport = leg.transport.clone();
            let stagger = leg.stagger;
            let label = format!("{transport:?}");
            set.spawn(async move {
                if !stagger.is_zero() {
                    tokio::time::sleep(stagger).await;
                }
                Ok((
                    DcConnection::connect_raw(&a, s.as_ref(), None, &transport, dc_id).await?,
                    label,
                ))
            });
        }

        let mut last_err = InvocationError::Deserialize("connect_fastest: no candidates".into());
        while let Some(outcome) = set.join_next().await {
            match outcome {
                Ok(Ok((conn, label))) => {
                    set.abort_all();
                    return Ok((conn, label));
                }
                Ok(Err(e)) => {
                    last_err = e;
                }
                Err(e) if e.is_cancelled() => {}
                Err(_) => {}
            }
        }
        Err(last_err)
    }

    /// Connect and perform full DH handshake, optionally via `mtproxy`.
    #[tracing::instrument(skip(socks5, mtproxy, transport), fields(addr = %addr, dc_id = dc_id))]
    pub async fn connect_raw(
        addr: &str,
        socks5: Option<&ferogram_connect::Socks5Config>,
        mtproxy: Option<&ferogram_connect::MtProxyConfig>,
        transport: &TransportKind,
        dc_id: i16,
    ) -> Result<Self, InvocationError> {
        tracing::debug!("[ferogram::sender] connecting to {addr} with known auth key");
        let (stream, frame_kind, enc) =
            ferogram_connect::connect_to_dc(addr, dc_id, transport, socks5, mtproxy).await?;

        tracing::debug!("[ferogram::sender] DH complete, auth key established for {addr}");
        let seen = new_seen_msg_ids();
        Ok(Self {
            stream,
            frame_kind,
            enc: EncryptedSession::with_seen(
                enc.auth_key_bytes(),
                enc.salt,
                enc.time_offset,
                seen.clone(),
            ),
            pending_acks: Vec::new(),
            call_count: 0,
            seen_msg_ids: seen,
        })
    }

    /// Connect with an already-known auth key (no DH needed).
    /// If `pfs` is true, performs a temp-key DH bind before any RPCs.
    #[allow(clippy::too_many_arguments)]
    pub async fn connect_with_key(
        addr: &str,
        auth_key: [u8; 256],
        first_salt: i64,
        time_offset: i32,
        socks5: Option<&ferogram_connect::Socks5Config>,
        mtproxy: Option<&ferogram_connect::MtProxyConfig>,
        transport: &TransportKind,
        dc_id: i16,
        pfs: bool,
    ) -> Result<Self, InvocationError> {
        // ferogram-connect owns TCP open + keepalive + transport init.
        let (mut stream, mut frame_kind) =
            ferogram_connect::Connection::open_stream_pub(addr, dc_id, transport, socks5, mtproxy)
                .await?;

        if pfs {
            tracing::debug!("[ferogram::sender] PFS: binding temporary key for DC{dc_id}");
            match Self::do_pool_pfs_bind(&mut stream, &mut frame_kind, &auth_key, dc_id).await {
                Ok(temp_enc) => {
                    tracing::debug!("[ferogram::sender] PFS: temporary key bound for DC{dc_id}");
                    return Ok(Self {
                        stream,
                        frame_kind,
                        enc: temp_enc,
                        pending_acks: Vec::new(),
                        call_count: 0,
                        seen_msg_ids: new_seen_msg_ids(),
                    });
                }
                Err(e) => {
                    tracing::warn!(
                        "[ferogram::sender] PFS bind failed for DC{dc_id} ({e}); using permanent key"
                    );
                    return Err(e);
                }
            }
        }

        let seen = new_seen_msg_ids();
        Ok(Self {
            stream,
            frame_kind,
            enc: EncryptedSession::with_seen(auth_key, first_salt, time_offset, seen.clone()),
            pending_acks: Vec::new(),
            call_count: 0,
            seen_msg_ids: seen,
        })
    }

    /// Temp-key DH handshake + auth.bindTempAuthKey on an existing stream.
    async fn do_pool_pfs_bind(
        stream: &mut tokio::net::TcpStream,
        kind: &mut FrameKind,
        perm_auth_key: &[u8; 256],
        dc_id: i16,
    ) -> Result<EncryptedSession, InvocationError> {
        use ferogram_mtproto::{
            auth_key_id_from_key, encrypt_bind_inner, gen_msg_id, new_seen_msg_ids,
            serialize_bind_temp_auth_key,
        };
        const TEMP_EXPIRES: i32 = 86_400; // 24 h

        // temp-key DH
        let mut plain = Session::new();

        let (req1, s1) = auth::step1().map_err(|e| InvocationError::Deserialize(e.to_string()))?;
        Self::send_plain_frame(stream, &plain.pack(&req1).to_plaintext_bytes(), kind).await?;
        let res_pq: tl::enums::ResPq = Self::recv_plain_frame(stream, kind).await?;

        let (req2, s2) = step2_temp(s1, res_pq, dc_id as i32, TEMP_EXPIRES)
            .map_err(|e| InvocationError::Deserialize(e.to_string()))?;
        Self::send_plain_frame(stream, &plain.pack(&req2).to_plaintext_bytes(), kind).await?;
        let dh: tl::enums::ServerDhParams = Self::recv_plain_frame(stream, kind).await?;

        let (req3, s3) =
            auth::step3(s2, dh).map_err(|e| InvocationError::Deserialize(e.to_string()))?;
        Self::send_plain_frame(stream, &plain.pack(&req3).to_plaintext_bytes(), kind).await?;
        let ans: tl::enums::SetClientDhParamsAnswer = Self::recv_plain_frame(stream, kind).await?;

        let done = {
            let mut result =
                auth::finish(s3, ans).map_err(|e| InvocationError::Deserialize(e.to_string()))?;
            let mut attempts = 0u8;
            loop {
                match result {
                    ferogram_mtproto::FinishResult::Done(d) => break d,
                    ferogram_mtproto::FinishResult::Retry {
                        retry_id,
                        dh_params,
                        nonce,
                        server_nonce,
                        new_nonce,
                    } => {
                        attempts += 1;
                        if attempts >= 5 {
                            return Err(InvocationError::Deserialize(
                                "PFS pool temp DH retry exceeded 5".into(),
                            ));
                        }
                        let (rr, s3r) = ferogram_mtproto::retry_step3(
                            &dh_params,
                            nonce,
                            server_nonce,
                            new_nonce,
                            retry_id,
                        )
                        .map_err(|e| InvocationError::Deserialize(e.to_string()))?;
                        Self::send_plain_frame(stream, &plain.pack(&rr).to_plaintext_bytes(), kind)
                            .await?;
                        let ar: tl::enums::SetClientDhParamsAnswer =
                            Self::recv_plain_frame(stream, kind).await?;
                        result = auth::finish(s3r, ar)
                            .map_err(|e| InvocationError::Deserialize(e.to_string()))?;
                    }
                }
            }
        };

        let temp_key = done.auth_key;
        let temp_salt = done.first_salt;
        let temp_offset = done.time_offset;

        // build bindTempAuthKey body
        let temp_key_id = auth_key_id_from_key(&temp_key);
        let perm_key_id = auth_key_id_from_key(perm_auth_key);

        let mut nonce_buf = [0u8; 8];
        ferogram_crypto::fill_random(&mut nonce_buf);
        let nonce = i64::from_le_bytes(nonce_buf);

        let server_now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system clock is before UNIX epoch")
            .as_secs() as i32
            + temp_offset;
        let expires_at = server_now + TEMP_EXPIRES;

        let seen = new_seen_msg_ids();
        let mut temp_enc = EncryptedSession::with_seen(temp_key, temp_salt, temp_offset, seen);
        let temp_session_id = temp_enc.session_id();

        let msg_id = gen_msg_id();
        let enc_msg = encrypt_bind_inner(
            perm_auth_key,
            msg_id,
            nonce,
            temp_key_id,
            perm_key_id,
            temp_session_id,
            expires_at,
        );
        let bind_body = serialize_bind_temp_auth_key(perm_key_id, nonce, expires_at, &enc_msg);

        // send encrypted bind request
        let wire = temp_enc.pack_body_at_msg_id(&bind_body, msg_id);
        Self::send_abridged(stream, &wire, kind).await?;

        // Receive and verify response.
        // The server may send informational frames first (msgs_ack, new_session_created)
        // before the actual rpc_result{boolTrue}, so we loop up to 5 frames.
        for attempt in 0u8..5 {
            let mut raw = Self::recv_abridged(stream, kind).await?;
            let decrypted = temp_enc.unpack(&mut raw).map_err(|e| {
                InvocationError::Deserialize(format!("PFS pool bind decrypt: {e:?}"))
            })?;
            match ferogram_connect::decode_bind_response(&decrypted.body) {
                Ok(()) => {
                    // bindTempAuthKey succeeds under the temp key; keep the session
                    // sequence as-is so subsequent RPCs continue from the same MTProto
                    // message stream.
                    return Ok(temp_enc);
                }
                Err(ref e) if e == "__need_more__" => {
                    tracing::debug!(
                        "[ferogram::sender] PFS (DC{dc_id}): got informational frame on attempt {attempt}, reading next"
                    );
                    continue;
                }
                Err(reason) => {
                    tracing::error!(
                        "[ferogram::sender] PFS bind rejected by server for DC{dc_id}: {reason}"
                    );
                    return Err(InvocationError::Deserialize(format!(
                        "auth.bindTempAuthKey (pool): {reason}"
                    )));
                }
            }
        }
        Err(InvocationError::Deserialize(
            "auth.bindTempAuthKey (pool): no boolTrue after 5 frames".into(),
        ))
    }

    /// The auth key this connection is currently encrypted with. Unlike
    /// [`crate::MtpSender::auth_key_bytes`], there's no separate permanent
    /// key tracked here, so under PFS this is the temporary key, not one
    /// safe to persist to the session.
    pub fn auth_key_bytes(&self) -> [u8; 256] {
        self.enc.auth_key_bytes()
    }
    /// The server salt this connection started with.
    pub fn first_salt(&self) -> i64 {
        self.enc.salt
    }
    /// Clock offset (seconds) between this client and the server.
    pub fn time_offset(&self) -> i32 {
        self.enc.time_offset
    }

    /// Decompose this connection into its raw parts so it can be handed off
    /// to [`crate::sender_task::spawn_sender_task`], graduating it from a
    /// single-request-at-a-time `DcConnection` into a pipelined background
    /// sender task that supports multiple concurrent in-flight requests.
    ///
    /// `DcPool` uses this once a connection has finished its setup (DH, PFS
    /// bind, initConnection) as a plain `DcConnection`. ferogram's transfer
    /// workers (`Client::open_worker_sender`) use the same pattern to enable
    /// request pipelining on upload/download connections.
    pub fn into_parts(self) -> (TcpStream, FrameKind, EncryptedSession) {
        (self.stream, self.frame_kind, self.enc)
    }

    /// Send `req` and block until its matching `rpc_result` comes back,
    /// discarding or handling anything else that arrives in between (server
    /// pushes, the periodic keepalive ping, salt/session-reset retries).
    /// One request at a time; `DcPool` graduates connections that need
    /// pipelined concurrent requests into a background sender task instead
    /// of using this directly.
    #[tracing::instrument(skip(self, req), fields(method = std::any::type_name::<R>()))]
    pub async fn rpc_call<R: RemoteCall>(&mut self, req: &R) -> Result<Vec<u8>, InvocationError> {
        let _t0 = std::time::Instant::now();
        // Periodic PingDelayDisconnect: sent before the request to piggyback on
        // the same TCP write window.  Keeps the socket alive across the download.
        self.call_count += 1;
        if self.call_count.is_multiple_of(PING_EVERY_N_CHUNKS) {
            let ping_id = self.call_count as i64;
            let ping_body = build_msgs_ack_ping_body(ping_id);
            // PingDelayDisconnect is content-related (returns Pong): must use odd seq_no.
            let (ping_wire, _) = self.enc.pack_body_with_msg_id(&ping_body, true);
            // This ping is fire-and-forget. The Pong response is a content-related
            // server message and must be acknowledged. If the RPC result arrives before
            // the Pong, the Pong's msg_id is never added to pending_acks. On idle
            // connections (no subsequent RPCs) the un-acked Pong will eventually cause
            // Telegram to close the connection. A dedicated always-running reader task
            // that drains and acks all server messages would fix this permanently; for
            // now the next rpc_call iteration receives and acks the Pong via pending_acks.
            let _ = Self::send_abridged(&mut self.stream, &ping_wire, &mut self.frame_kind).await;
        }

        // Flush pending acks.
        if !self.pending_acks.is_empty() {
            let ack_body = build_msgs_ack_body(&self.pending_acks);
            let (ack_wire, _) = self.enc.pack_body_with_msg_id(&ack_body, false);
            let _ = Self::send_abridged(&mut self.stream, &ack_wire, &mut self.frame_kind).await;
            self.pending_acks.clear();
        }

        // Track sent msg_id to verify rpc_result.req_msg_id and discard stale responses.
        let (wire, mut sent_msg_id) = self.enc.pack_with_msg_id(req);
        Self::send_abridged(&mut self.stream, &wire, &mut self.frame_kind).await?;
        let mut salt_retries = 0u8;
        let mut session_resets = 0u8;
        loop {
            let mut raw = Self::recv_abridged(&mut self.stream, &mut self.frame_kind).await?;
            let msg = self
                .enc
                .unpack(&mut raw)
                .map_err(|e| InvocationError::Deserialize(e.to_string()))?;
            // Track every received msg_id for acknowledgement.
            self.pending_acks.push(msg.msg_id);
            if self.pending_acks.len() >= PENDING_ACKS_THRESHOLD {
                // Eager flush: too many un-acked messages  - Telegram will close the
                // connection if we don't ack within its window.
                let ack_body = build_msgs_ack_body(&self.pending_acks);
                let (ack_wire, _) = self.enc.pack_body_with_msg_id(&ack_body, false);
                let _ =
                    Self::send_abridged(&mut self.stream, &ack_wire, &mut self.frame_kind).await;
                self.pending_acks.clear();
            }
            // Salt is updated only on explicit bad_server_salt, not on every message.
            if msg.body.len() < 4 {
                return Ok(msg.body);
            }
            let mut need_resend = false;
            let mut need_session_reset = false;
            let mut bad_msg_code: Option<u32> = None;
            let mut bad_msg_server_id: Option<i64> = None;
            // Process all flags before returning: containers may carry
            // new_session_created + rpc_result together.
            let scan_result = Self::scan_body(
                &msg.body,
                &mut self.enc.salt,
                &mut need_resend,
                &mut need_session_reset,
                &mut bad_msg_code,
                &mut bad_msg_server_id,
                Some(sent_msg_id),
                msg.msg_id,
            )?;
            // new_session_created requires seq_no reset to 0.
            if need_session_reset {
                session_resets += 1;
                if session_resets > 2 {
                    return Err(InvocationError::Deserialize(
                        "new_session_created: exceeded 2 resets".into(),
                    ));
                }
                if !self.pending_acks.is_empty() {
                    let ack_body = build_msgs_ack_body(&self.pending_acks);
                    let (ack_wire, _) = self.enc.pack_body_with_msg_id(&ack_body, false);
                    let _ = Self::send_abridged(&mut self.stream, &ack_wire, &mut self.frame_kind)
                        .await;
                    self.pending_acks.clear();
                }
                // Keep the current session sequence. new_session_created updates the
                // server salt and may require resending stale requests, but it does
                // not require zeroing the local MTProto seq counter.
                if scan_result.is_none() {
                    // No result yet; resend using the current MTProto sequence.
                    tracing::debug!(
                        "[ferogram::sender] new_session_created: resending request (attempt {session_resets}/2)"
                    );
                    let (wire, new_id) = self.enc.pack_with_msg_id(req);
                    sent_msg_id = new_id;
                    Self::send_abridged(&mut self.stream, &wire, &mut self.frame_kind).await?;
                }
                // If scan_result.is_some(), the result arrived in the same container
                // as new_session_created; session has been reset for future calls,
                // fall through to return the result.
            } else if need_resend {
                // Apply seq_no / time corrections from bad_msg_notification.
                match bad_msg_code {
                    Some(16) | Some(17) => {
                        if let Some(srv_id) = bad_msg_server_id {
                            self.enc.correct_time_offset(srv_id);
                        }
                        // Do not call undo_seq_no here. Reusing the same seq_no on a
                        // retry violates MTProto monotonicity; the server may reject
                        // with code 32. Let the next pack_with_msg_id assign the next
                        // available odd seq_no for the resent message.
                    }
                    Some(32) | Some(33) => {
                        // correct_seq_no does a full session reset (new session_id,
                        // seq_no=0) instead of magic +/- offsets.
                        self.enc
                            .correct_seq_no(bad_msg_code.expect("matched Some arm"));
                    }
                    _ => {
                        // bad_server_salt or bad_msg code 48
                        self.enc.undo_seq_no();
                    }
                }
                salt_retries += 1;
                if salt_retries >= 5 {
                    return Err(InvocationError::Deserialize(
                        "bad_server_salt/bad_msg: exceeded 5 retries".into(),
                    ));
                }
                tracing::debug!(
                    "[ferogram::sender] resending transfer request after bad_msg correction (code={bad_msg_code:?}, attempt {salt_retries}/5)"
                );
                if !self.pending_acks.is_empty() {
                    let ack_body = build_msgs_ack_body(&self.pending_acks);
                    let (ack_wire, _) = self.enc.pack_body_with_msg_id(&ack_body, false);
                    let _ = Self::send_abridged(&mut self.stream, &ack_wire, &mut self.frame_kind)
                        .await;
                    self.pending_acks.clear();
                }
                let (wire, new_id) = self.enc.pack_with_msg_id(req);
                sent_msg_id = new_id;
                Self::send_abridged(&mut self.stream, &wire, &mut self.frame_kind).await?;
            }
            if let Some(result) = scan_result {
                metrics::counter!("ferogram.rpc_calls_total", "result" => "ok").increment(1);
                metrics::histogram!("ferogram.rpc_latency_ms")
                    .record(_t0.elapsed().as_millis() as f64);
                return Ok(result);
            }
        }
    }
    ///
    /// Returns `Ok(Some(bytes))` when rpc_result is found.
    /// Returns `Ok(None)` for informational messages (continue reading).
    /// Returns `Err` for rpc_error or parse failures.
    ///
    /// Output flags:
    /// - `need_resend`: set for bad_server_salt / bad_msg_notification (codes 16/17/32/33/48)
    /// - `need_session_reset`: set for new_session_created (seq_no must reset to 0)
    /// - `bad_msg_code`: error_code from bad_msg_notification for caller to apply correction
    /// - `bad_msg_server_id`: server msg_id for time-offset correction (codes 16/17)
    /// - `server_msg_id`: outer frame msg_id for time-offset correction (codes 16/17).
    ///   Must be msg.msg_id from the caller, not bad_msg_id (client clock, not server's).
    #[allow(clippy::too_many_arguments)]
    fn scan_body(
        body: &[u8],
        salt: &mut i64,
        need_resend: &mut bool,
        need_session_reset: &mut bool,
        bad_msg_code: &mut Option<u32>,
        bad_msg_server_id: &mut Option<i64>,
        sent_msg_id: Option<i64>,
        server_msg_id: i64,
    ) -> Result<Option<Vec<u8>>, InvocationError> {
        if body.len() < 4 {
            return Ok(None);
        }
        let cid = u32::from_le_bytes(body[..4].try_into().expect("body.len() >= 4 checked above"));
        match cid {
            0xf35c6d01 /* rpc_result: CID(4) + req_msg_id(8) + result */ => {
                if body.len() >= 12
                    && let Some(expected) = sent_msg_id {
                        let resp_id = i64::from_le_bytes(body[4..12].try_into().expect("body.len() >= 12 checked above"));
                        if resp_id != expected {
                            tracing::debug!(
                                "[ferogram::sender] rpc_result msg_id mismatch (got {resp_id:#018x}, want {expected:#018x}); skipping this frame"
                            );
                            return Ok(None);
                        }
                    }
                let inner = if body.len() >= 12 { &body[12..] } else { body };
                // Inner body may itself be gzip_packed (e.g. help.Config inside rpc_result).
                if inner.len() >= 4
                    && u32::from_le_bytes(inner[..4].try_into().expect("inner.len() >= 4 checked above")) == 0x3072cfa1
                {
                    let mut dummy_salt = *salt;
                    let mut nr = false; let mut nsr = false;
                    let mut bc = None; let mut bsi = None;
                    if let Some(r) = Self::scan_body(inner, &mut dummy_salt, &mut nr, &mut nsr, &mut bc, &mut bsi, None, server_msg_id)? {
                        return Ok(Some(r));
                    }
                    // Unwrap the gzip directly and return the decompressed bytes.
                    if let Some(compressed) = ferogram_connect::tl_read_bytes(&inner[4..])
                        && let Ok(out) = ferogram_connect::gz_inflate(&compressed)
                    {
                        return Ok(Some(out));
                    }
                    return Ok(None);
                }
                if inner.len() >= 8
                    && u32::from_le_bytes(inner[..4].try_into().expect("inner.len() >= 8 checked above")) == 0x2144ca19
                {
                    let code = i32::from_le_bytes(inner[4..8].try_into().expect("inner.len() >= 8 checked above"));
                    let message = ferogram_connect::tl_read_string(&inner[8..]).unwrap_or_default();
                    return Err(InvocationError::Rpc(
                        crate::errors::RpcError::from_telegram(code, &message),
                    ));
                }
                Ok(Some(inner.to_vec()))
            }
            0x2144ca19 /* rpc_error */ => {
                if body.len() < 8 {
                    return Err(InvocationError::Deserialize("rpc_error short".into()));
                }
                let code = i32::from_le_bytes(body[4..8].try_into().expect("body.len() >= 8 checked above"));
                let message = ferogram_connect::tl_read_string(&body[8..]).unwrap_or_default();
                Err(InvocationError::Rpc(crate::errors::RpcError::from_telegram(code, &message)))
            }
            0xedab447b /* bad_server_salt */ => {
                // bad_server_salt#edab447b bad_msg_id:long bad_msg_seqno:int error_code:int new_server_salt:long
                if body.len() >= 28 {
                    let bad_msg_id = i64::from_le_bytes(body[4..12].try_into().expect("body.len() >= 28 checked above"));
                    let new_salt   = i64::from_le_bytes(body[20..28].try_into().expect("body.len() >= 28 checked above"));
                    // Only apply new salt when bad_msg_id matches our sent request;
                    // stale frames from prior requests must not corrupt the current salt.
                    if sent_msg_id.is_none_or(|id| id == bad_msg_id) {
                        *salt = new_salt;
                        *need_resend = true;
                    }
                }
                Ok(None)
            }
            0x9ec20908 /* new_session_created */ => {
                // new_session_created#9ec20908 first_msg_id:long unique_id:long server_salt:long
                // Signal need_session_reset so the caller resets seq_no before resending.
                if body.len() >= 28 {
                    let first_msg_id = i64::from_le_bytes(body[4..12].try_into().expect("body.len() >= 28 checked above"));
                    let unique_id    = i64::from_le_bytes(body[12..20].try_into().expect("body.len() >= 28 checked above"));
                    let server_salt  = i64::from_le_bytes(body[20..28].try_into().expect("body.len() >= 28 checked above"));
                    tracing::debug!(
                        unique_id = format_args!("{unique_id:#018x}"),
                        first_msg_id,
                        salt = server_salt,
                        "[ferogram::sender] new_session_created: server opened fresh session"
                    );
                    *salt = server_salt;
                    // Only reset if the pending request predates the server's new session.
                    // If sent_msg_id == first_msg_id (fresh worker conn on first send),
                    // the server will reply with our current session_id. Unconditionally
                    // calling reset_session() here changes the id, causing the response
                    // decrypt to fail with session_id mismatch.
                    if sent_msg_id.is_some_and(|id| id < first_msg_id) {
                        *need_session_reset = true;
                    }
                }
                Ok(None)
            }
            0xa7eff811 /* bad_msg_notification */ => {
                // bad_msg_notification#a7eff811 bad_msg_id:long bad_msg_seqno:int error_code:int
                //
                // TL layout: body[4..12]=bad_msg_id, body[12..16]=bad_msg_seqno,
                // body[16..20]=error_code. Previous code read [12..16] as error_code
                // (bad_msg_seqno), so error matching always compared the wrong field.
                if body.len() >= 20 {
                    let bad_msg_id  = i64::from_le_bytes(body[4..12].try_into().expect("body.len() >= 20 checked above"));
                    // body[12..16] = bad_msg_seqno, not used for recovery.
                    let error_code  = u32::from_le_bytes(body[16..20].try_into().expect("body.len() >= 20 checked above"));
                    tracing::debug!(
                        bad_msg_id = format_args!("{bad_msg_id:#018x}"),
                        error_code,
                        "[ferogram::sender] bad_msg_notification received"
                    );
                    match error_code {
                        16 | 17 => {
                            // msg_id too low/high: time-offset correction needed.
                            // server_msg_id upper 32 bits = server Unix timestamp.
                            // bad_msg_id carries the client's clock, not the server's.
                            *bad_msg_code = Some(error_code);
                            *bad_msg_server_id = Some(server_msg_id);
                            *need_resend = sent_msg_id.is_none_or(|id| id == bad_msg_id);
                        }
                        32 | 33 => {
                            // seq_no wrong.
                            *bad_msg_code = Some(error_code);
                            *need_resend = sent_msg_id.is_none_or(|id| id == bad_msg_id);
                        }
                        48 => {
                            // bad_msg code 48 = incorrect server salt. Per spec, this
                            // arrives together with a bad_server_salt frame in the same
                            // container that carries the new salt. If bad_server_salt was
                            // already processed, *salt is updated and the resend uses the
                            // correct value. If not (partial container), resend once
                            // conservatively; the retry loop's 5-attempt cap prevents a loop.
                            *need_resend = sent_msg_id.is_none_or(|id| id == bad_msg_id);
                            tracing::debug!(
                                "[ferogram::sender] bad_msg code 48 (wrong server salt): will resend with updated salt"
                            );
                        }
                        _ => {
                            // Unknown code; resend to avoid the loop stalling.
                            *need_resend = sent_msg_id.is_none_or(|id| id == bad_msg_id);
                        }
                    }
                }
                Ok(None)
            }
            0x347773c5 /* pong */ => {
                // Pong is returned for both internal PingDelayDisconnect (fire-and-forget)
                // and user-invoked Ping (which has a pending invoke future waiting).
                // pong layout: CID(4) + msg_id(8) + ping_id(8)
                // pong.msg_id is the msg_id of the original ping request.
                // Route back to the caller when it matches the pending sent_msg_id.
                if body.len() >= 12
                    && let Some(expected) = sent_msg_id
                {
                    let pong_req_id = i64::from_le_bytes(body[4..12].try_into().expect("body.len() >= 12 for pong"));
                    if pong_req_id == expected {
                        return Ok(Some(body.to_vec()));
                    }
                }
                // Internal keepalive pong - discard.
                Ok(None)
            }
            0x73f1f8dc /* msg_container */ => {
                if body.len() < 8 {
                    return Ok(None);
                }
                let count = u32::from_le_bytes(body[4..8].try_into().expect("body.len() >= 8 for msg_container")) as usize;
                let mut pos = 8usize;
                // Do not early-return: containers may bundle new_session_created + rpc_result
                // together; all items must be processed so session/salt flags are observed.
                let mut found: Option<Vec<u8>> = None;
                for _ in 0..count {
                    if pos + 16 > body.len() { break; }
                    let inner_bytes =
                        u32::from_le_bytes(body[pos + 12..pos + 16].try_into().expect("pos+16 <= body.len() checked above")) as usize;
                    pos += 16;
                    if pos + inner_bytes > body.len() { break; }
                    let inner = &body[pos..pos + inner_bytes];
                    pos += inner_bytes;
                    if found.is_none() {
                        if let Some(r) = Self::scan_body(inner, salt, need_resend,
                            need_session_reset, bad_msg_code, bad_msg_server_id, sent_msg_id,
                            server_msg_id)?
                        {
                            found = Some(r);
                            // Do NOT return  - continue processing remaining items so that
                            // session/salt flags from co-arriving messages are observed.
                        }
                    } else {
                        // Result already captured; still process remaining items for
                        // side-effect flags (salt, session reset, bad_msg). Pass
                        // sent_msg_id so the req_msg_id guard still filters stale
                        // rpc_results. Passing None would bypass the guard and allow
                        // a stale response to overwrite `found` on the next iteration.
                        let _ = Self::scan_body(inner, salt, need_resend, need_session_reset,
                                                bad_msg_code, bad_msg_server_id, sent_msg_id,
                                                server_msg_id)?;
                    }
                }
                Ok(found)
            }
            0x3072cfa1 /* gzip_packed */ => {
                // Decompress and recurse: server wraps large responses in gzip_packed.
                if let Some(compressed) = ferogram_connect::tl_read_bytes(&body[4..])
                    && let Ok(decompressed) = ferogram_connect::gz_inflate(&compressed)
                    && !decompressed.is_empty()
                {
                    return Self::scan_body(
                        &decompressed, salt,
                        need_resend, need_session_reset,
                        bad_msg_code, bad_msg_server_id,
                        sent_msg_id,
                        server_msg_id,
                    );
                }
                Ok(None)
            }
            _ => Ok(None),
        }
    }

    /// Like `rpc_call` but accepts any `Serializable` type (not just `RemoteCall`).
    pub async fn rpc_call_serializable<S: ferogram_tl_types::Serializable>(
        &mut self,
        req: &S,
    ) -> Result<Vec<u8>, InvocationError> {
        if !self.pending_acks.is_empty() {
            let ack_body = build_msgs_ack_body(&self.pending_acks);
            let (ack_wire, _) = self.enc.pack_body_with_msg_id(&ack_body, false);
            let _ = Self::send_abridged(&mut self.stream, &ack_wire, &mut self.frame_kind).await;
            self.pending_acks.clear();
        }
        let (wire, mut sent_msg_id) = self.enc.pack_serializable_with_msg_id(req);
        Self::send_abridged(&mut self.stream, &wire, &mut self.frame_kind).await?;
        let mut salt_retries = 0u8;
        let mut session_resets = 0u8;
        loop {
            let mut raw = Self::recv_abridged(&mut self.stream, &mut self.frame_kind).await?;
            let msg = self
                .enc
                .unpack(&mut raw)
                .map_err(|e| InvocationError::Deserialize(e.to_string()))?;
            self.pending_acks.push(msg.msg_id);
            if self.pending_acks.len() >= PENDING_ACKS_THRESHOLD {
                let ack_body = build_msgs_ack_body(&self.pending_acks);
                let (ack_wire, _) = self.enc.pack_body_with_msg_id(&ack_body, false);
                let _ =
                    Self::send_abridged(&mut self.stream, &ack_wire, &mut self.frame_kind).await;
                self.pending_acks.clear();
            }
            // Salt updated only on explicit bad_server_salt, not on every message.
            if msg.body.len() < 4 {
                return Ok(msg.body);
            }
            let mut need_resend = false;
            let mut need_session_reset = false;
            let mut bad_msg_code: Option<u32> = None;
            let mut bad_msg_server_id: Option<i64> = None;
            // Save result before handling flags; apply all before returning.
            let scan_result = Self::scan_body(
                &msg.body,
                &mut self.enc.salt,
                &mut need_resend,
                &mut need_session_reset,
                &mut bad_msg_code,
                &mut bad_msg_server_id,
                Some(sent_msg_id),
                msg.msg_id,
            )?;
            if need_session_reset {
                session_resets += 1;
                if session_resets > 2 {
                    return Err(InvocationError::Deserialize(
                        "new_session_created (serializable): exceeded 2 resets".into(),
                    ));
                }
                if !self.pending_acks.is_empty() {
                    let ack_body = build_msgs_ack_body(&self.pending_acks);
                    let (ack_wire, _) = self.enc.pack_body_with_msg_id(&ack_body, false);
                    let _ = Self::send_abridged(&mut self.stream, &ack_wire, &mut self.frame_kind)
                        .await;
                    self.pending_acks.clear();
                }
                if scan_result.is_none() {
                    let (wire, new_id) = self.enc.pack_serializable_with_msg_id(req);
                    sent_msg_id = new_id;
                    Self::send_abridged(&mut self.stream, &wire, &mut self.frame_kind).await?;
                }
            } else if need_resend {
                match bad_msg_code {
                    Some(16) | Some(17) => {
                        if let Some(srv_id) = bad_msg_server_id {
                            self.enc.correct_time_offset(srv_id);
                        }
                        // Do not call undo_seq_no (see rpc_call for explanation).
                    }
                    Some(32) | Some(33) => {
                        self.enc
                            .correct_seq_no(bad_msg_code.expect("matched Some arm"));
                    }
                    _ => {
                        self.enc.undo_seq_no();
                    }
                }
                salt_retries += 1;
                if salt_retries >= 5 {
                    return Err(InvocationError::Deserialize(
                        "bad_server_salt (serializable): exceeded 5 retries".into(),
                    ));
                }
                tracing::debug!(
                    "[ferogram::sender] resending serializable request after bad_msg correction (code={bad_msg_code:?}, attempt {salt_retries}/5)"
                );
                if !self.pending_acks.is_empty() {
                    let ack_body = build_msgs_ack_body(&self.pending_acks);
                    let (ack_wire, _) = self.enc.pack_body_with_msg_id(&ack_body, false);
                    let _ = Self::send_abridged(&mut self.stream, &ack_wire, &mut self.frame_kind)
                        .await;
                    self.pending_acks.clear();
                }
                let (wire, new_id) = self.enc.pack_serializable_with_msg_id(req);
                sent_msg_id = new_id;
                Self::send_abridged(&mut self.stream, &wire, &mut self.frame_kind).await?;
            }
            if let Some(result) = scan_result {
                return Ok(result);
            }
        }
    }

    /// Send pre-serialized raw bytes and receive the raw response.
    /// Used by CDN download connections (no MTProto encryption layer).
    pub async fn rpc_call_raw(&mut self, body: &[u8]) -> Result<Vec<u8>, InvocationError> {
        Self::send_abridged(&mut self.stream, body, &mut self.frame_kind).await?;
        Self::recv_abridged(&mut self.stream, &mut self.frame_kind).await
    }

    /// Send a framed message using the active FrameKind.
    /// All transport variants (Abridged, Intermediate, Full, Obfuscated, …) are handled.
    async fn send_abridged(
        stream: &mut TcpStream,
        data: &[u8],
        kind: &mut FrameKind,
    ) -> Result<(), InvocationError> {
        use tokio::io::AsyncWriteExt as _;
        match kind {
            FrameKind::Abridged => {
                let words = data.len() / 4;
                let mut frame = if words < 0x7f {
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
                frame.extend_from_slice(data);
                stream.write_all(&frame).await?;
            }
            FrameKind::Intermediate => {
                let mut frame = Vec::with_capacity(4 + data.len());
                frame.extend_from_slice(&(data.len() as u32).to_le_bytes());
                frame.extend_from_slice(data);
                stream.write_all(&frame).await?;
            }
            FrameKind::Full { send_seqno, .. } => {
                let seq = send_seqno.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                let total_len = (data.len() as u32) + 12;
                let mut packet = Vec::with_capacity(total_len as usize);
                packet.extend_from_slice(&total_len.to_le_bytes());
                packet.extend_from_slice(&seq.to_le_bytes());
                packet.extend_from_slice(data);
                let crc = ferogram_connect::crc32_ieee(&packet);
                packet.extend_from_slice(&crc.to_le_bytes());
                stream.write_all(&packet).await?;
            }
            FrameKind::Obfuscated { cipher } => {
                let words = data.len() / 4;
                let mut frame = if words < 0x7f {
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
                frame.extend_from_slice(data);
                cipher.lock().await.encrypt(&mut frame);
                stream.write_all(&frame).await?;
            }
            FrameKind::PaddedIntermediate { cipher } => {
                let mut pad_len_buf = [0u8; 1];
                ferogram_crypto::fill_random(&mut pad_len_buf);
                let pad_len = (pad_len_buf[0] & 0x0f) as usize;
                let total_payload = data.len() + pad_len;
                let mut frame = Vec::with_capacity(4 + total_payload);
                frame.extend_from_slice(&(total_payload as u32).to_le_bytes());
                frame.extend_from_slice(data);
                let mut pad = vec![0u8; pad_len];
                ferogram_crypto::fill_random(&mut pad);
                frame.extend_from_slice(&pad);
                cipher.lock().await.encrypt(&mut frame);
                stream.write_all(&frame).await?;
            }
            FrameKind::FakeTls { cipher, .. } => {
                // Same PaddedIntermediate framing as `dd`, then wrapped in
                // TLS Application Data records (see ferogram-connect's
                // frame.rs / mtp_sender.rs for the matching read side and
                // the handshake that establishes `cipher`). The leading
                // ChangeCipherSpec decoy is sent once, during the
                // handshake, not here.
                let mut pad_len_buf = [0u8; 1];
                ferogram_crypto::fill_random(&mut pad_len_buf);
                let pad_len = (pad_len_buf[0] & 0x0f) as usize;
                let total_payload = data.len() + pad_len;
                let mut frame = Vec::with_capacity(4 + total_payload);
                frame.extend_from_slice(&(total_payload as u32).to_le_bytes());
                frame.extend_from_slice(data);
                let mut pad = vec![0u8; pad_len];
                ferogram_crypto::fill_random(&mut pad);
                frame.extend_from_slice(&pad);
                cipher.lock().await.encrypt(&mut frame);

                let mut wire = Vec::new();
                ferogram_connect::tls_record::wrap_application_data(&frame, &mut wire);
                stream.write_all(&wire).await?;
            }
        }
        Ok(())
    }

    /// Receive a framed message using the active FrameKind (with 60-second timeout).
    async fn recv_abridged(
        stream: &mut TcpStream,
        kind: &mut FrameKind,
    ) -> Result<Vec<u8>, InvocationError> {
        use tokio::time::{Duration, timeout};
        const RECV_TIMEOUT: Duration = Duration::from_secs(60);

        macro_rules! tread {
            ($buf:expr) => {
                timeout(RECV_TIMEOUT, stream.read_exact($buf))
                    .await
                    .map_err(|_| {
                        InvocationError::Io(std::io::Error::new(
                            std::io::ErrorKind::TimedOut,
                            "transfer recv: timeout (60 s)",
                        ))
                    })??
            };
        }

        match kind {
            FrameKind::Abridged => {
                let mut h = [0u8; 1];
                tread!(&mut h);
                let words = if h[0] == 0x7f {
                    let mut b = [0u8; 3];
                    tread!(&mut b);
                    let w = b[0] as usize | (b[1] as usize) << 8 | (b[2] as usize) << 16;
                    if w == 1 {
                        let mut code_buf = [0u8; 4];
                        tread!(&mut code_buf);
                        let code = i32::from_le_bytes(code_buf);
                        return Err(InvocationError::Rpc(
                            crate::errors::RpcError::from_telegram(code, "transport error"),
                        ));
                    }
                    w
                } else {
                    h[0] as usize
                };
                let mut buf = vec![0u8; words * 4];
                tread!(&mut buf);
                if buf.len() == 4 {
                    let code = i32::from_le_bytes(buf[..4].try_into().unwrap());
                    if code < 0 {
                        return Err(InvocationError::Rpc(
                            crate::errors::RpcError::from_telegram(code, "transport error"),
                        ));
                    }
                }
                Ok(buf)
            }
            FrameKind::Intermediate => {
                let mut len_buf = [0u8; 4];
                tread!(&mut len_buf);
                let len_i32 = i32::from_le_bytes(len_buf);
                if len_i32 < 0 {
                    return Err(InvocationError::Rpc(
                        crate::errors::RpcError::from_telegram(len_i32, "transport error"),
                    ));
                }
                let mut buf = vec![0u8; len_i32 as usize];
                tread!(&mut buf);
                Ok(buf)
            }
            FrameKind::Full { recv_seqno, .. } => {
                let mut len_buf = [0u8; 4];
                tread!(&mut len_buf);
                let total_len_i32 = i32::from_le_bytes(len_buf);
                if total_len_i32 < 0 {
                    return Err(InvocationError::Rpc(
                        crate::errors::RpcError::from_telegram(total_len_i32, "transport error"),
                    ));
                }
                let total_len = total_len_i32 as usize;
                if total_len < 12 {
                    return Err(InvocationError::Deserialize(
                        "Full transport: packet too short".into(),
                    ));
                }
                let mut rest = vec![0u8; total_len - 4];
                tread!(&mut rest);
                let (body, crc_bytes) = rest.split_at(rest.len() - 4);
                let expected_crc = u32::from_le_bytes(crc_bytes.try_into().unwrap());
                let mut check_input = Vec::with_capacity(4 + body.len());
                check_input.extend_from_slice(&len_buf);
                check_input.extend_from_slice(body);
                let actual_crc = ferogram_connect::crc32_ieee(&check_input);
                if actual_crc != expected_crc {
                    return Err(InvocationError::Deserialize(format!(
                        "Full transport: CRC mismatch (got {actual_crc:#010x}, expected {expected_crc:#010x})"
                    )));
                }
                let recv_seq = u32::from_le_bytes(body[..4].try_into().unwrap());
                let expected_seq = recv_seqno.load(std::sync::atomic::Ordering::Relaxed);
                if recv_seq != expected_seq {
                    return Err(InvocationError::Deserialize(format!(
                        "Full transport: seqno mismatch (got {recv_seq}, expected {expected_seq})"
                    )));
                }
                recv_seqno.store(
                    expected_seq.wrapping_add(1),
                    std::sync::atomic::Ordering::Relaxed,
                );
                Ok(body[4..].to_vec())
            }
            FrameKind::Obfuscated { cipher } => {
                let mut h = [0u8; 1];
                tread!(&mut h);
                cipher.lock().await.decrypt(&mut h);
                let words = if h[0] == 0x7f {
                    let mut b = [0u8; 3];
                    tread!(&mut b);
                    cipher.lock().await.decrypt(&mut b);
                    let w = b[0] as usize | (b[1] as usize) << 8 | (b[2] as usize) << 16;
                    if w == 1 {
                        let mut code_buf = [0u8; 4];
                        tread!(&mut code_buf);
                        cipher.lock().await.decrypt(&mut code_buf);
                        let code = i32::from_le_bytes(code_buf);
                        return Err(InvocationError::Rpc(
                            crate::errors::RpcError::from_telegram(code, "transport error"),
                        ));
                    }
                    w
                } else {
                    h[0] as usize
                };
                let mut buf = vec![0u8; words * 4];
                tread!(&mut buf);
                cipher.lock().await.decrypt(&mut buf);
                if buf.len() == 4 {
                    let code = i32::from_le_bytes(buf[..4].try_into().unwrap());
                    if code < 0 {
                        return Err(InvocationError::Rpc(
                            crate::errors::RpcError::from_telegram(code, "transport error"),
                        ));
                    }
                }
                Ok(buf)
            }
            FrameKind::PaddedIntermediate { cipher } => {
                let mut len_buf = [0u8; 4];
                tread!(&mut len_buf);
                cipher.lock().await.decrypt(&mut len_buf);
                let total_len = i32::from_le_bytes(len_buf);
                if total_len < 0 {
                    return Err(InvocationError::Rpc(
                        crate::errors::RpcError::from_telegram(total_len, "transport error"),
                    ));
                }
                let mut buf = vec![0u8; total_len as usize];
                tread!(&mut buf);
                cipher.lock().await.decrypt(&mut buf);
                if buf.len() >= 24 {
                    let pad = (buf.len() - 24) % 16;
                    buf.truncate(buf.len() - pad);
                }
                Ok(buf)
            }
            FrameKind::FakeTls {
                cipher,
                decoded_pending,
                ..
            } => {
                async fn timed(
                    fut: impl std::future::Future<Output = Result<(), ferogram_connect::ConnectError>>,
                ) -> Result<(), InvocationError> {
                    timeout(RECV_TIMEOUT, fut)
                        .await
                        .map_err(|_| {
                            InvocationError::Io(std::io::Error::new(
                                std::io::ErrorKind::TimedOut,
                                "transfer recv: timeout (60 s)",
                            ))
                        })?
                        .map_err(InvocationError::from)
                }

                let mut len_buf = [0u8; 4];
                timed(ferogram_connect::faketls_read_exact(
                    stream,
                    cipher,
                    decoded_pending,
                    &mut len_buf,
                ))
                .await?;
                let total_len = i32::from_le_bytes(len_buf);
                if total_len < 0 {
                    return Err(InvocationError::Rpc(
                        crate::errors::RpcError::from_telegram(total_len, "transport error"),
                    ));
                }
                let mut buf = vec![0u8; total_len as usize];
                timed(ferogram_connect::faketls_read_exact(
                    stream,
                    cipher,
                    decoded_pending,
                    &mut buf,
                ))
                .await?;
                if buf.len() >= 24 {
                    let pad = (buf.len() - 24) % 16;
                    buf.truncate(buf.len() - pad);
                }
                Ok(buf)
            }
        }
    }

    /// Send a plaintext (DH handshake) frame, padding to 4-byte alignment for
    /// abridged-family transports. Full and Intermediate don't need padding.
    async fn send_plain_frame(
        stream: &mut TcpStream,
        data: &[u8],
        kind: &mut FrameKind,
    ) -> Result<(), InvocationError> {
        // Abridged/Obfuscated use word-count (len/4); must be 4-byte aligned.
        // Full and Intermediate carry the exact byte length so no padding needed.
        let needs_align = matches!(kind, FrameKind::Abridged | FrameKind::Obfuscated { .. });
        if needs_align && !data.len().is_multiple_of(4) {
            let mut padded = data.to_vec();
            let pad = 4 - (data.len() % 4);
            padded.resize(data.len() + pad, 0);
            Self::send_abridged(stream, &padded, kind).await
        } else {
            Self::send_abridged(stream, data, kind).await
        }
    }

    async fn recv_plain_frame<T: Deserializable>(
        stream: &mut TcpStream,
        kind: &mut FrameKind,
    ) -> Result<T, InvocationError> {
        let raw = Self::recv_abridged(stream, kind).await?;
        if raw.len() == 4 {
            let code = i32::from_le_bytes(raw[..4].try_into().unwrap());
            if code < 0 {
                return Err(InvocationError::Deserialize(format!(
                    "server transport error during DH: code {code}"
                )));
            }
        }
        if raw.len() < 20 {
            return Err(InvocationError::Deserialize("plain frame too short".into()));
        }
        if u64::from_le_bytes(raw[..8].try_into().unwrap()) != 0 {
            return Err(InvocationError::Deserialize(
                "expected auth_key_id=0 in plaintext".into(),
            ));
        }
        let body_len = u32::from_le_bytes(raw[16..20].try_into().unwrap()) as usize;
        if raw.len() < 20 + body_len {
            return Err(InvocationError::Deserialize(format!(
                "plain frame truncated: have {} bytes, need {}",
                raw.len(),
                20 + body_len
            )));
        }
        let mut cur = Cursor::from_slice(&raw[20..20 + body_len]);
        T::deserialize(&mut cur).map_err(Into::into)
    }
}
