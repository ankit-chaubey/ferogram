// Copyright (c) Ankit Chaubey <ankitchaubey.dev@gmail.com>
// SPDX-License-Identifier: MIT OR Apache-2.0
//
// ferogram: async Telegram MTProto client in Rust
// https://github.com/ankit-chaubey/ferogram
//
// If you use or modify this code, keep this notice at the top of your file
// and include the LICENSE-MIT or LICENSE-APACHE file from this repository:
// https://github.com/ankit-chaubey/ferogram

use ferogram_mtproto::{
    EncryptedSession, SeenMsgIds, Session, authentication as auth, new_seen_msg_ids, step2_temp,
};
use ferogram_tl_types as tl;
use ferogram_tl_types::{Cursor, Deserializable, RemoteCall};
use std::collections::HashMap;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

use crate::{InvocationError, TransportKind, session::DcEntry};
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
    /// AES-256-CTR cipher for obfuscated transport; None for plain transports.
    cipher: Option<ferogram_crypto::ObfuscatedCipher>,
    /// Persistent dedup ring that outlives individual EncryptedSessions.
    #[allow(dead_code)]
    seen_msg_ids: SeenMsgIds,
}

impl DcConnection {
    /// Race Obfuscated / Abridged / Http transports and return the first to succeed.
    #[tracing::instrument(skip(socks5), fields(addr = %addr, dc_id = dc_id))]
    pub async fn connect_fastest(
        addr: &str,
        socks5: Option<&crate::socks5::Socks5Config>,
        dc_id: i16,
    ) -> Result<(Self, &'static str), InvocationError> {
        use tokio::task::JoinSet;
        let addr = addr.to_owned();
        let socks5 = socks5.cloned();
        tracing::debug!("[dc_pool] probing {addr} with 3 transports");
        let mut set: JoinSet<Result<(DcConnection, &'static str), InvocationError>> =
            JoinSet::new();

        {
            let a = addr.clone();
            let s = socks5.clone();
            set.spawn(async move {
                Ok((
                    DcConnection::connect_raw(
                        &a,
                        s.as_ref(),
                        &TransportKind::Obfuscated { secret: None },
                        dc_id,
                    )
                    .await?,
                    "Obfuscated",
                ))
            });
        }
        {
            let a = addr.clone();
            let s = socks5.clone();
            set.spawn(async move {
                tokio::time::sleep(std::time::Duration::from_millis(200)).await;
                Ok((
                    DcConnection::connect_raw(&a, s.as_ref(), &TransportKind::Abridged, dc_id)
                        .await?,
                    "Abridged",
                ))
            });
        }
        {
            let a = addr.clone();
            set.spawn(async move {
                tokio::time::sleep(std::time::Duration::from_millis(800)).await;
                Ok((
                    DcConnection::connect_raw(&a, None, &TransportKind::Http, dc_id).await?,
                    "Http",
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

    /// Connect and perform full DH handshake.
    #[tracing::instrument(skip(socks5, transport), fields(addr = %addr, dc_id = dc_id))]
    pub async fn connect_raw(
        addr: &str,
        socks5: Option<&crate::socks5::Socks5Config>,
        transport: &TransportKind,
        dc_id: i16,
    ) -> Result<Self, InvocationError> {
        tracing::debug!("[dc_pool] Connecting to {addr} …");
        let mut stream = Self::open_tcp(addr, socks5).await?;
        let mut cipher = Self::send_transport_init(&mut stream, transport, dc_id).await?;

        let mut plain = Session::new();

        let (req1, s1) = auth::step1().map_err(|e| InvocationError::Deserialize(e.to_string()))?;
        Self::send_plain_frame(
            &mut stream,
            &plain.pack(&req1).to_plaintext_bytes(),
            cipher.as_mut(),
        )
        .await?;
        let res_pq: tl::enums::ResPq = Self::recv_plain_frame(&mut stream, cipher.as_mut()).await?;

        let (req2, s2) = auth::step2(s1, res_pq, dc_id as i32)
            .map_err(|e| InvocationError::Deserialize(e.to_string()))?;
        Self::send_plain_frame(
            &mut stream,
            &plain.pack(&req2).to_plaintext_bytes(),
            cipher.as_mut(),
        )
        .await?;
        let dh: tl::enums::ServerDhParams =
            Self::recv_plain_frame(&mut stream, cipher.as_mut()).await?;

        let (req3, s3) =
            auth::step3(s2, dh).map_err(|e| InvocationError::Deserialize(e.to_string()))?;
        Self::send_plain_frame(
            &mut stream,
            &plain.pack(&req3).to_plaintext_bytes(),
            cipher.as_mut(),
        )
        .await?;
        let ans: tl::enums::SetClientDhParamsAnswer =
            Self::recv_plain_frame(&mut stream, cipher.as_mut()).await?;

        // Retry loop for dh_gen_retry (up to 5 attempts).
        let done = {
            let mut result =
                auth::finish(s3, ans).map_err(|e| InvocationError::Deserialize(e.to_string()))?;
            let mut attempts = 0u8;
            loop {
                match result {
                    auth::FinishResult::Done(d) => break d,
                    auth::FinishResult::Retry {
                        retry_id,
                        dh_params,
                        nonce,
                        server_nonce,
                        new_nonce,
                    } => {
                        attempts += 1;
                        if attempts >= 5 {
                            return Err(InvocationError::Deserialize(
                                "dh_gen_retry exceeded 5 attempts".into(),
                            ));
                        }
                        let (req_retry, s3_retry) =
                            auth::retry_step3(&dh_params, nonce, server_nonce, new_nonce, retry_id)
                                .map_err(|e| InvocationError::Deserialize(e.to_string()))?;
                        Self::send_plain_frame(
                            &mut stream,
                            &plain.pack(&req_retry).to_plaintext_bytes(),
                            cipher.as_mut(),
                        )
                        .await?;
                        let ans_retry: tl::enums::SetClientDhParamsAnswer =
                            Self::recv_plain_frame(&mut stream, cipher.as_mut()).await?;
                        result = auth::finish(s3_retry, ans_retry)
                            .map_err(|e| InvocationError::Deserialize(e.to_string()))?;
                    }
                }
            }
        };
        tracing::debug!("[dc_pool] DH complete ✓ for {addr}");

        let seen = new_seen_msg_ids();
        Ok(Self {
            stream,
            cipher,
            enc: EncryptedSession::with_seen(
                done.auth_key,
                done.first_salt,
                done.time_offset,
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
        socks5: Option<&crate::socks5::Socks5Config>,
        mtproxy: Option<&crate::proxy::MtProxyConfig>,
        transport: &TransportKind,
        dc_id: i16,
        pfs: bool,
    ) -> Result<Self, InvocationError> {
        let (mut stream, mut cipher) = if let Some(mp) = mtproxy {
            let mut s = mp.connect().await?;
            s.set_nodelay(true)?;
            let c = Self::send_transport_init(&mut s, &mp.transport, dc_id).await?;
            (s, c)
        } else {
            let mut s = Self::open_tcp(addr, socks5).await?;
            let c = Self::send_transport_init(&mut s, transport, dc_id).await?;
            (s, c)
        };

        if pfs {
            tracing::debug!("[dc_pool] PFS: temp DH bind for DC{dc_id}");
            match Self::do_pool_pfs_bind(&mut stream, cipher.as_mut(), &auth_key, dc_id).await {
                Ok(temp_enc) => {
                    tracing::info!("[dc_pool] PFS bind complete DC{dc_id}");
                    return Ok(Self {
                        stream,
                        cipher,
                        enc: temp_enc,
                        pending_acks: Vec::new(),
                        call_count: 0,
                        seen_msg_ids: new_seen_msg_ids(),
                    });
                }
                Err(e) => {
                    tracing::warn!("[dc_pool] PFS bind failed DC{dc_id} ({e}); falling back");
                    return Err(e);
                }
            }
        }

        let seen = new_seen_msg_ids();
        Ok(Self {
            stream,
            cipher,
            enc: EncryptedSession::with_seen(auth_key, first_salt, time_offset, seen.clone()),
            pending_acks: Vec::new(),
            call_count: 0,
            seen_msg_ids: seen,
        })
    }

    /// Temp-key DH handshake + auth.bindTempAuthKey on an existing stream.
    #[allow(clippy::needless_option_as_deref)]
    async fn do_pool_pfs_bind(
        stream: &mut tokio::net::TcpStream,
        mut cipher: Option<&mut ferogram_crypto::ObfuscatedCipher>,
        perm_auth_key: &[u8; 256],
        dc_id: i16,
    ) -> Result<EncryptedSession, InvocationError> {
        use ferogram_mtproto::{
            auth_key_id_from_key, encrypt_bind_inner, gen_msg_id, new_seen_msg_ids,
            serialize_bind_temp_auth_key,
        };
        const TEMP_EXPIRES: i32 = 86_400; // 24 h

        // temp-key DH
        let mut plain = ferogram_mtproto::Session::new();

        let (req1, s1) = auth::step1().map_err(|e| InvocationError::Deserialize(e.to_string()))?;
        Self::send_plain_frame(
            stream,
            &plain.pack(&req1).to_plaintext_bytes(),
            cipher.as_deref_mut(),
        )
        .await?;
        let res_pq: tl::enums::ResPq =
            Self::recv_plain_frame(stream, cipher.as_deref_mut()).await?;

        let (req2, s2) = step2_temp(s1, res_pq, dc_id as i32, TEMP_EXPIRES)
            .map_err(|e| InvocationError::Deserialize(e.to_string()))?;
        Self::send_plain_frame(
            stream,
            &plain.pack(&req2).to_plaintext_bytes(),
            cipher.as_deref_mut(),
        )
        .await?;
        let dh: tl::enums::ServerDhParams =
            Self::recv_plain_frame(stream, cipher.as_deref_mut()).await?;

        let (req3, s3) =
            auth::step3(s2, dh).map_err(|e| InvocationError::Deserialize(e.to_string()))?;
        Self::send_plain_frame(
            stream,
            &plain.pack(&req3).to_plaintext_bytes(),
            cipher.as_deref_mut(),
        )
        .await?;
        let ans: tl::enums::SetClientDhParamsAnswer =
            Self::recv_plain_frame(stream, cipher.as_deref_mut()).await?;

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
                        Self::send_plain_frame(
                            stream,
                            &plain.pack(&rr).to_plaintext_bytes(),
                            cipher.as_deref_mut(),
                        )
                        .await?;
                        let ar: tl::enums::SetClientDhParamsAnswer =
                            Self::recv_plain_frame(stream, cipher.as_deref_mut()).await?;
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
        getrandom::getrandom(&mut nonce_buf)
            .map_err(|_| InvocationError::Deserialize("getrandom nonce".into()))?;
        let nonce = i64::from_le_bytes(nonce_buf);

        let server_now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
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
        Self::send_abridged(stream, &wire, cipher.as_deref_mut()).await?;

        // Receive and verify response.
        // The server may send informational frames first (msgs_ack, new_session_created)
        // before the actual rpc_result{boolTrue}, so we loop up to 5 frames.
        for attempt in 0u8..5 {
            let mut raw = Self::recv_abridged(stream, cipher.as_deref_mut()).await?;
            let decrypted = temp_enc.unpack(&mut raw).map_err(|e| {
                InvocationError::Deserialize(format!("PFS pool bind decrypt: {e:?}"))
            })?;
            match pfs_pool_decode_bind_response(&decrypted.body) {
                Ok(()) => {
                    // bindTempAuthKey succeeds under the temp key; keep the session
                    // sequence as-is so subsequent RPCs continue from the same MTProto
                    // message stream.
                    return Ok(temp_enc);
                }
                Err(ref e) if e == "__need_more__" => {
                    tracing::debug!(
                        "[ferogram] PFS pool bind (DC{dc_id}): informational frame {attempt}, reading next"
                    );
                    continue;
                }
                Err(reason) => {
                    tracing::error!(
                        "[ferogram] PFS pool bind server response (DC{dc_id}): {reason}"
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

    async fn open_tcp(
        addr: &str,
        socks5: Option<&crate::socks5::Socks5Config>,
    ) -> Result<TcpStream, InvocationError> {
        let stream = match socks5 {
            Some(proxy) => proxy.connect(addr).await?,
            None => TcpStream::connect(addr).await?,
        };
        // Disable Nagle for immediate single-frame delivery.
        stream.set_nodelay(true)?;
        // SO_KEEPALIVE keeps worker connections alive across idle periods.
        {
            let sock = socket2::SockRef::from(&stream);
            let ka = socket2::TcpKeepalive::new()
                .with_time(std::time::Duration::from_secs(10))
                .with_interval(std::time::Duration::from_secs(5));
            #[cfg(not(target_os = "windows"))]
            let ka = ka.with_retries(3);
            sock.set_tcp_keepalive(&ka).ok();
        }
        Ok(stream)
    }

    async fn send_transport_init(
        stream: &mut TcpStream,
        transport: &TransportKind,
        dc_id: i16,
    ) -> Result<Option<ferogram_crypto::ObfuscatedCipher>, InvocationError> {
        match transport {
            TransportKind::Abridged => {
                stream.write_all(&[0xef]).await?;
            }
            TransportKind::Intermediate => {
                stream.write_all(&[0xee, 0xee, 0xee, 0xee]).await?;
            }
            TransportKind::Full => {}
            TransportKind::Obfuscated { secret } => {
                use sha2::Digest;
                let mut nonce = [0u8; 64];
                loop {
                    getrandom::getrandom(&mut nonce)
                        .map_err(|_| InvocationError::Deserialize("getrandom".into()))?;
                    let first = u32::from_le_bytes(nonce[0..4].try_into().unwrap());
                    let second = u32::from_le_bytes(nonce[4..8].try_into().unwrap());
                    let bad = nonce[0] == 0xEF
                        || first == 0x44414548
                        || first == 0x54534F50
                        || first == 0x20544547
                        || first == 0x4954504f  // OPTIONS
                        || first == 0xEEEEEEEE
                        || first == 0xDDDDDDDD
                        || first == 0x02010316
                        || second == 0x00000000;
                    if !bad {
                        break;
                    }
                }
                let tx_raw: [u8; 32] = nonce[8..40].try_into().unwrap();
                let tx_iv: [u8; 16] = nonce[40..56].try_into().unwrap();
                let mut rev48 = nonce[8..56].to_vec();
                rev48.reverse();
                let rx_raw: [u8; 32] = rev48[0..32].try_into().unwrap();
                let rx_iv: [u8; 16] = rev48[32..48].try_into().unwrap();
                let (tx_key, rx_key): ([u8; 32], [u8; 32]) = if let Some(s) = secret {
                    let mut h = sha2::Sha256::new();
                    h.update(tx_raw);
                    h.update(s.as_ref());
                    let tx: [u8; 32] = h.finalize().into();
                    let mut h = sha2::Sha256::new();
                    h.update(rx_raw);
                    h.update(s.as_ref());
                    let rx: [u8; 32] = h.finalize().into();
                    (tx, rx)
                } else {
                    (tx_raw, rx_raw)
                };
                nonce[56] = 0xef;
                nonce[57] = 0xef;
                nonce[58] = 0xef;
                nonce[59] = 0xef;
                let dc_bytes = dc_id.to_le_bytes();
                nonce[60] = dc_bytes[0];
                nonce[61] = dc_bytes[1];
                let mut enc =
                    ferogram_crypto::ObfuscatedCipher::from_keys(&tx_key, &tx_iv, &rx_key, &rx_iv);
                let mut skip = [0u8; 56];
                enc.encrypt(&mut skip);
                enc.encrypt(&mut nonce[56..64]);
                stream.write_all(&nonce).await?;
                return Ok(Some(enc));
            }
            TransportKind::PaddedIntermediate { secret } => {
                use sha2::Digest;
                let mut nonce = [0u8; 64];
                loop {
                    getrandom::getrandom(&mut nonce)
                        .map_err(|_| InvocationError::Deserialize("getrandom".into()))?;
                    let first = u32::from_le_bytes(nonce[0..4].try_into().unwrap());
                    let second = u32::from_le_bytes(nonce[4..8].try_into().unwrap());
                    let bad = nonce[0] == 0xEF
                        || first == 0x44414548
                        || first == 0x54534F50
                        || first == 0x20544547
                        || first == 0x4954504f
                        || first == 0xEEEEEEEE
                        || first == 0xDDDDDDDD
                        || first == 0x02010316
                        || second == 0x00000000;
                    if !bad {
                        break;
                    }
                }
                let tx_raw: [u8; 32] = nonce[8..40].try_into().unwrap();
                let tx_iv: [u8; 16] = nonce[40..56].try_into().unwrap();
                let mut rev48 = nonce[8..56].to_vec();
                rev48.reverse();
                let rx_raw: [u8; 32] = rev48[0..32].try_into().unwrap();
                let rx_iv: [u8; 16] = rev48[32..48].try_into().unwrap();
                let (tx_key, rx_key): ([u8; 32], [u8; 32]) = if let Some(s) = secret {
                    let mut h = sha2::Sha256::new();
                    h.update(tx_raw);
                    h.update(s.as_ref());
                    let tx: [u8; 32] = h.finalize().into();
                    let mut h = sha2::Sha256::new();
                    h.update(rx_raw);
                    h.update(s.as_ref());
                    let rx: [u8; 32] = h.finalize().into();
                    (tx, rx)
                } else {
                    (tx_raw, rx_raw)
                };
                nonce[56] = 0xdd;
                nonce[57] = 0xdd;
                nonce[58] = 0xdd;
                nonce[59] = 0xdd;
                let dc_bytes = dc_id.to_le_bytes();
                nonce[60] = dc_bytes[0];
                nonce[61] = dc_bytes[1];
                let mut enc =
                    ferogram_crypto::ObfuscatedCipher::from_keys(&tx_key, &tx_iv, &rx_key, &rx_iv);
                let mut skip = [0u8; 56];
                enc.encrypt(&mut skip);
                enc.encrypt(&mut nonce[56..64]);
                stream.write_all(&nonce).await?;
                return Ok(Some(enc));
            }
            TransportKind::FakeTls { .. } => {
                // FakeTls requires a full TLS 1.2 ClientHello handshake which is not yet
                // implemented in DcPool worker connections. Use Obfuscated or
                // PaddedIntermediate for proxy connections instead.
                return Err(InvocationError::Deserialize(
                    "FakeTls transport is not supported for DcPool connections".into(),
                ));
            }
            TransportKind::Http => {}
        }
        Ok(None)
    }

    pub fn auth_key_bytes(&self) -> [u8; 256] {
        self.enc.auth_key_bytes()
    }
    pub fn first_salt(&self) -> i64 {
        self.enc.salt
    }
    pub fn time_offset(&self) -> i32 {
        self.enc.time_offset
    }

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
            let _ = Self::send_abridged(&mut self.stream, &ping_wire, self.cipher.as_mut()).await;
        }

        // Flush pending acks.
        if !self.pending_acks.is_empty() {
            let ack_body = build_msgs_ack_body(&self.pending_acks);
            let (ack_wire, _) = self.enc.pack_body_with_msg_id(&ack_body, false);
            let _ = Self::send_abridged(&mut self.stream, &ack_wire, self.cipher.as_mut()).await;
            self.pending_acks.clear();
        }

        // Track sent msg_id to verify rpc_result.req_msg_id and discard stale responses.
        let (wire, mut sent_msg_id) = self.enc.pack_with_msg_id(req);
        Self::send_abridged(&mut self.stream, &wire, self.cipher.as_mut()).await?;
        let mut salt_retries = 0u8;
        let mut session_resets = 0u8;
        loop {
            let mut raw = Self::recv_abridged(&mut self.stream, self.cipher.as_mut()).await?;
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
                    Self::send_abridged(&mut self.stream, &ack_wire, self.cipher.as_mut()).await;
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
                    let _ = Self::send_abridged(&mut self.stream, &ack_wire, self.cipher.as_mut())
                        .await;
                    self.pending_acks.clear();
                }
                // Keep the current session sequence. new_session_created updates the
                // server salt and may require resending stale requests, but it does
                // not require zeroing the local MTProto seq counter.
                if scan_result.is_none() {
                    // No result yet; resend using the current MTProto sequence.
                    tracing::debug!(
                        "[dc_pool] new_session_created: resending [{session_resets}/2]"
                    );
                    let (wire, new_id) = self.enc.pack_with_msg_id(req);
                    sent_msg_id = new_id;
                    Self::send_abridged(&mut self.stream, &wire, self.cipher.as_mut()).await?;
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
                        self.enc.correct_seq_no(bad_msg_code.unwrap());
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
                    "[dc_pool] resend in transfer conn (code={bad_msg_code:?}) [{salt_retries}/5]"
                );
                if !self.pending_acks.is_empty() {
                    let ack_body = build_msgs_ack_body(&self.pending_acks);
                    let (ack_wire, _) = self.enc.pack_body_with_msg_id(&ack_body, false);
                    let _ = Self::send_abridged(&mut self.stream, &ack_wire, self.cipher.as_mut())
                        .await;
                    self.pending_acks.clear();
                }
                let (wire, new_id) = self.enc.pack_with_msg_id(req);
                sent_msg_id = new_id;
                Self::send_abridged(&mut self.stream, &wire, self.cipher.as_mut()).await?;
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
        let cid = u32::from_le_bytes(body[..4].try_into().unwrap());
        match cid {
            0xf35c6d01 /* rpc_result: CID(4) + req_msg_id(8) + result */ => {
                if body.len() >= 12
                    && let Some(expected) = sent_msg_id {
                        let resp_id = i64::from_le_bytes(body[4..12].try_into().unwrap());
                        if resp_id != expected {
                            tracing::debug!(
                                "[dc_pool] rpc_result req_msg_id mismatch \
                                 (got {resp_id:#018x}, want {expected:#018x}); skipping"
                            );
                            return Ok(None);
                        }
                    }
                let inner = if body.len() >= 12 { &body[12..] } else { body };
                // Inner body may itself be gzip_packed (e.g. help.Config inside rpc_result).
                if inner.len() >= 4
                    && u32::from_le_bytes(inner[..4].try_into().unwrap()) == 0x3072cfa1
                {
                    let mut dummy_salt = *salt;
                    let mut nr = false; let mut nsr = false;
                    let mut bc = None; let mut bsi = None;
                    if let Some(r) = Self::scan_body(inner, &mut dummy_salt, &mut nr, &mut nsr, &mut bc, &mut bsi, None, server_msg_id)? {
                        return Ok(Some(r));
                    }
                    // Unwrap the gzip directly and return the decompressed bytes.
                    if let Some(compressed) = tl_read_bytes(&inner[4..]) {
                        let dec = flate2::read::GzDecoder::new(compressed.as_slice());
                        let mut limited = std::io::Read::take(dec, 16 * 1024 * 1024);
                        let mut out = Vec::new();
                        if std::io::Read::read_to_end(&mut limited, &mut out).is_ok() {
                            return Ok(Some(out));
                        }
                    }
                    return Ok(None);
                }
                if inner.len() >= 8
                    && u32::from_le_bytes(inner[..4].try_into().unwrap()) == 0x2144ca19
                {
                    let code = i32::from_le_bytes(inner[4..8].try_into().unwrap());
                    let message = tl_read_string(&inner[8..]).unwrap_or_default();
                    return Err(InvocationError::Rpc(
                        crate::RpcError::from_telegram(code, &message),
                    ));
                }
                Ok(Some(inner.to_vec()))
            }
            0x2144ca19 /* rpc_error */ => {
                if body.len() < 8 {
                    return Err(InvocationError::Deserialize("rpc_error short".into()));
                }
                let code = i32::from_le_bytes(body[4..8].try_into().unwrap());
                let message = tl_read_string(&body[8..]).unwrap_or_default();
                Err(InvocationError::Rpc(crate::RpcError::from_telegram(code, &message)))
            }
            0xedab447b /* bad_server_salt */ => {
                // bad_server_salt#edab447b bad_msg_id:long bad_msg_seqno:int error_code:int new_server_salt:long
                if body.len() >= 28 {
                    let bad_msg_id = i64::from_le_bytes(body[4..12].try_into().unwrap());
                    let new_salt   = i64::from_le_bytes(body[20..28].try_into().unwrap());
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
                    let first_msg_id = i64::from_le_bytes(body[4..12].try_into().unwrap());
                    let unique_id    = i64::from_le_bytes(body[12..20].try_into().unwrap());
                    let server_salt  = i64::from_le_bytes(body[20..28].try_into().unwrap());
                    tracing::debug!(
                        "[dc_pool] new_session_created: unique_id={unique_id:#018x} \
                         first_msg_id={first_msg_id} salt={server_salt}"
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
                    let bad_msg_id  = i64::from_le_bytes(body[4..12].try_into().unwrap());
                    // body[12..16] = bad_msg_seqno, not used for recovery.
                    let error_code  = u32::from_le_bytes(body[16..20].try_into().unwrap());
                    tracing::debug!(
                        "[dc_pool] bad_msg_notification: bad_msg_id={bad_msg_id:#018x} code={error_code}"
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
                                "[dc_pool] bad_msg code 48 (wrong salt): will resend with current salt"
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
                    let pong_req_id = i64::from_le_bytes(body[4..12].try_into().unwrap());
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
                let count = u32::from_le_bytes(body[4..8].try_into().unwrap()) as usize;
                let mut pos = 8usize;
                // Do not early-return: containers may bundle new_session_created + rpc_result
                // together; all items must be processed so session/salt flags are observed.
                let mut found: Option<Vec<u8>> = None;
                for _ in 0..count {
                    if pos + 16 > body.len() { break; }
                    let inner_bytes =
                        u32::from_le_bytes(body[pos + 12..pos + 16].try_into().unwrap()) as usize;
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
                if let Some(compressed) = tl_read_bytes(&body[4..]) {
                    let decoder = flate2::read::GzDecoder::new(compressed.as_slice());
                    let mut limited = std::io::Read::take(decoder, 16 * 1024 * 1024);
                    let mut decompressed = Vec::new();
                    if std::io::Read::read_to_end(&mut limited, &mut decompressed).is_ok()
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
            let _ = Self::send_abridged(&mut self.stream, &ack_wire, self.cipher.as_mut()).await;
            self.pending_acks.clear();
        }
        let (wire, mut sent_msg_id) = self.enc.pack_serializable_with_msg_id(req);
        Self::send_abridged(&mut self.stream, &wire, self.cipher.as_mut()).await?;
        let mut salt_retries = 0u8;
        let mut session_resets = 0u8;
        loop {
            let mut raw = Self::recv_abridged(&mut self.stream, self.cipher.as_mut()).await?;
            let msg = self
                .enc
                .unpack(&mut raw)
                .map_err(|e| InvocationError::Deserialize(e.to_string()))?;
            self.pending_acks.push(msg.msg_id);
            if self.pending_acks.len() >= PENDING_ACKS_THRESHOLD {
                let ack_body = build_msgs_ack_body(&self.pending_acks);
                let (ack_wire, _) = self.enc.pack_body_with_msg_id(&ack_body, false);
                let _ =
                    Self::send_abridged(&mut self.stream, &ack_wire, self.cipher.as_mut()).await;
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
                    let _ = Self::send_abridged(&mut self.stream, &ack_wire, self.cipher.as_mut())
                        .await;
                    self.pending_acks.clear();
                }
                if scan_result.is_none() {
                    let (wire, new_id) = self.enc.pack_serializable_with_msg_id(req);
                    sent_msg_id = new_id;
                    Self::send_abridged(&mut self.stream, &wire, self.cipher.as_mut()).await?;
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
                        self.enc.correct_seq_no(bad_msg_code.unwrap());
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
                    "[dc_pool] resend serializable (code={bad_msg_code:?}) [{salt_retries}/5]"
                );
                if !self.pending_acks.is_empty() {
                    let ack_body = build_msgs_ack_body(&self.pending_acks);
                    let (ack_wire, _) = self.enc.pack_body_with_msg_id(&ack_body, false);
                    let _ = Self::send_abridged(&mut self.stream, &ack_wire, self.cipher.as_mut())
                        .await;
                    self.pending_acks.clear();
                }
                let (wire, new_id) = self.enc.pack_serializable_with_msg_id(req);
                sent_msg_id = new_id;
                Self::send_abridged(&mut self.stream, &wire, self.cipher.as_mut()).await?;
            }
            if let Some(result) = scan_result {
                return Ok(result);
            }
        }
    }

    /// Send pre-serialized raw bytes and receive the raw response.
    /// Used by CDN download connections (no MTProto encryption layer).
    pub async fn rpc_call_raw(&mut self, body: &[u8]) -> Result<Vec<u8>, InvocationError> {
        Self::send_abridged(&mut self.stream, body, self.cipher.as_mut()).await?;
        Self::recv_abridged(&mut self.stream, self.cipher.as_mut()).await
    }

    async fn send_abridged(
        stream: &mut TcpStream,
        data: &[u8],
        cipher: Option<&mut ferogram_crypto::ObfuscatedCipher>,
    ) -> Result<(), InvocationError> {
        // Single write_all: avoids Nagle stalls and partial-write corruption.
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
        if let Some(c) = cipher {
            c.encrypt(&mut frame);
        }
        stream.write_all(&frame).await?;
        Ok(())
    }

    async fn recv_abridged(
        stream: &mut TcpStream,
        mut cipher: Option<&mut ferogram_crypto::ObfuscatedCipher>,
    ) -> Result<Vec<u8>, InvocationError> {
        // 60-second recv timeout: prevents hung reads on silently closed connections.
        use tokio::time::{Duration, timeout};
        const RECV_TIMEOUT: Duration = Duration::from_secs(60);

        let mut h = [0u8; 1];
        timeout(RECV_TIMEOUT, stream.read_exact(&mut h))
            .await
            .map_err(|_| {
                InvocationError::Io(std::io::Error::new(
                    std::io::ErrorKind::TimedOut,
                    "transfer recv: header timeout (60 s)",
                ))
            })??;
        if let Some(ref mut c) = cipher.as_mut() {
            c.decrypt(&mut h);
        }

        // 0x7f = extended length; next 3 bytes are the LE word count.
        let words = if h[0] == 0x7f {
            let mut b = [0u8; 3];
            timeout(RECV_TIMEOUT, stream.read_exact(&mut b))
                .await
                .map_err(|_| {
                    InvocationError::Io(std::io::Error::new(
                        std::io::ErrorKind::TimedOut,
                        "transfer recv: length timeout (60 s)",
                    ))
                })??;
            if let Some(ref mut c) = cipher.as_mut() {
                c.decrypt(&mut b);
            }
            b[0] as usize | (b[1] as usize) << 8 | (b[2] as usize) << 16
        } else {
            h[0] as usize
        };

        let mut buf = vec![0u8; words * 4];
        timeout(RECV_TIMEOUT, stream.read_exact(&mut buf))
            .await
            .map_err(|_| {
                InvocationError::Io(std::io::Error::new(
                    std::io::ErrorKind::TimedOut,
                    "transfer recv: body timeout (60 s)",
                ))
            })??;
        if let Some(c) = cipher {
            c.decrypt(&mut buf);
        }

        // Transport errors are exactly 4 bytes (negative LE i32).
        // A valid encrypted MTProto frame is always ≥ 68 bytes:
        //   auth_key_id(8) + msg_key(16) + encrypted[salt(8)+session_id(8)+
        //   msg_id(8)+seq_no(4)+data_len(4)+body(≥4)+padding(≥12)] ≥ 68 bytes.
        // Checking buf.len() == 4 is therefore both necessary and sufficient to
        // distinguish a transport error from a valid encrypted frame; this is
        // correct by protocol structure, not merely empirically safe.
        if buf.len() == 4 {
            let code = i32::from_le_bytes(buf[..4].try_into().unwrap());
            if code < 0 {
                return Err(InvocationError::Io(std::io::Error::new(
                    std::io::ErrorKind::ConnectionRefused,
                    format!("transport error from server: {code}"),
                )));
            }
        }

        Ok(buf)
    }

    async fn send_plain_frame(
        stream: &mut TcpStream,
        data: &[u8],
        cipher: Option<&mut ferogram_crypto::ObfuscatedCipher>,
    ) -> Result<(), InvocationError> {
        // Abridged framing uses word-count (len/4): pad to 4-byte boundary.
        // TL parsers ignore trailing zero bytes.
        if !data.len().is_multiple_of(4) {
            let mut padded = data.to_vec();
            let pad = 4 - (data.len() % 4);
            padded.resize(data.len() + pad, 0);
            Self::send_abridged(stream, &padded, cipher).await
        } else {
            Self::send_abridged(stream, data, cipher).await
        }
    }

    async fn recv_plain_frame<T: Deserializable>(
        stream: &mut TcpStream,
        cipher: Option<&mut ferogram_crypto::ObfuscatedCipher>,
    ) -> Result<T, InvocationError> {
        let raw = Self::recv_abridged(stream, cipher).await?;
        // A 4-byte negative payload is a transport error code from the server.
        // Surface it directly rather than masking it with "plain frame too short".
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

/// Check a decrypted PFS bind response body for boolTrue.
/// Decode one bare MTProto message body for the auth.bindTempAuthKey response (pool path).
fn pfs_pool_decode_bind_single(body: &[u8]) -> Result<(), String> {
    const RPC_RESULT: u32 = 0xf35c6d01;
    const BOOL_TRUE: u32 = 0x9972_75b5;
    const BOOL_FALSE: u32 = 0xbc79_9737;
    const RPC_ERROR: u32 = 0x2144_ca19;
    const BAD_MSG: u32 = 0xa7ef_f811;
    const BAD_SALT: u32 = 0xedab_447b;
    const NEW_SESSION: u32 = 0x9ec2_0908;
    const FUTURE_SALTS: u32 = 0xae50_0895;
    const MSGS_ACK: u32 = 0x62d6_b459; // msgs_ack#62d6b459
    const PONG: u32 = 0x0347_73c5;

    if body.len() < 4 {
        return Err("skip".into());
    }
    let ctor = u32::from_le_bytes(body[..4].try_into().unwrap());

    match ctor {
        BOOL_TRUE => Ok(()),
        BOOL_FALSE => Err("server returned boolFalse (binding rejected)".into()),
        NEW_SESSION | FUTURE_SALTS | MSGS_ACK | PONG => Err("skip".into()),

        RPC_RESULT if body.len() >= 16 => {
            let inner = u32::from_le_bytes(body[12..16].try_into().unwrap());
            match inner {
                BOOL_TRUE => Ok(()),
                BOOL_FALSE => Err("rpc_result{boolFalse} (server rejected binding)".into()),
                RPC_ERROR if body.len() >= 20 => {
                    let code = i32::from_le_bytes(body[16..20].try_into().unwrap());
                    let msg = tl_read_string(body.get(20..).unwrap_or(&[])).unwrap_or_default();
                    Err(format!("rpc_error code={code} message={msg:?}"))
                }
                _ => Err(format!("rpc_result inner ctor={inner:#010x}")),
            }
        }

        BAD_MSG if body.len() >= 16 => {
            let code = u32::from_le_bytes(body[12..16].try_into().unwrap());
            let desc = match code {
                16 => "msg_id too low (clock skew)",
                17 => "msg_id too high (clock skew)",
                18 => "incorrect lower 2 bits of msg_id",
                19 => "duplicate msg_id",
                20 => "message too old (>300s)",
                32 => "msg_seqno too low",
                33 => "msg_seqno too high",
                48 => "incorrect server salt",
                _ => "unknown code",
            };
            Err(format!("bad_msg_notification code={code} ({desc})"))
        }

        BAD_SALT if body.len() >= 24 => {
            let new_salt = i64::from_le_bytes(body[16..24].try_into().unwrap());
            Err(format!(
                "bad_server_salt, server wants salt={new_salt:#018x}"
            ))
        }

        _ => Err(format!("unknown ctor={ctor:#010x}")),
    }
}

/// Decode the server response to auth.bindTempAuthKey (pool path).
///
/// Handles bare messages AND msg_container (the server frequently bundles
/// new_session_created + rpc_result together in a container).
fn pfs_pool_decode_bind_response(body: &[u8]) -> Result<(), String> {
    const MSG_CONTAINER: u32 = 0x73f1f8dc;

    if body.len() < 4 {
        return Err(format!("response body too short ({} bytes)", body.len()));
    }
    let ctor = u32::from_le_bytes(body[..4].try_into().unwrap());

    if ctor != MSG_CONTAINER {
        return pfs_pool_decode_bind_single(body).map_err(|e| {
            if e == "skip" {
                "__need_more__".into()
            } else {
                e
            }
        });
    }

    if body.len() < 8 {
        return Err("msg_container too short to read count".into());
    }
    let count = u32::from_le_bytes(body[4..8].try_into().unwrap()) as usize;
    let mut pos = 8usize;
    let mut last_real_err: Option<String> = None;

    for i in 0..count {
        if pos + 16 > body.len() {
            return Err(format!(
                "msg_container truncated at message {i}/{count} (pos={pos} body_len={})",
                body.len()
            ));
        }
        let msg_bytes = u32::from_le_bytes(body[pos + 12..pos + 16].try_into().unwrap()) as usize;
        pos += 16;

        if pos + msg_bytes > body.len() {
            return Err(format!(
                "msg_container message {i} body overflows (need {msg_bytes}, have {})",
                body.len() - pos
            ));
        }
        let msg_body = &body[pos..pos + msg_bytes];
        pos += msg_bytes;

        match pfs_pool_decode_bind_single(msg_body) {
            Ok(()) => return Ok(()),
            Err(e) if e == "skip" => continue,
            Err(e) => {
                last_real_err = Some(e);
            }
        }
    }

    Err(last_real_err.unwrap_or_else(|| "__need_more__".into()))
}

fn tl_read_bytes(data: &[u8]) -> Option<Vec<u8>> {
    if data.is_empty() {
        return Some(vec![]);
    }
    let (len, start) = if data[0] < 254 {
        (data[0] as usize, 1)
    } else if data.len() >= 4 {
        (
            data[1] as usize | (data[2] as usize) << 8 | (data[3] as usize) << 16,
            4,
        )
    } else {
        return None;
    };
    if data.len() < start + len {
        return None;
    }
    Some(data[start..start + len].to_vec())
}

fn tl_read_string(data: &[u8]) -> Option<String> {
    tl_read_bytes(data).map(|b| String::from_utf8_lossy(&b).into_owned())
}

// Max simultaneous connections per DC.
const MAX_CONNS_PER_DC: usize = 3;

/// One slot in the per-DC connection pool.
/// `in_flight` lets the pool pick the least-busy slot without locking it.
pub(crate) struct ConnSlot {
    pub conn: tokio::sync::Mutex<DcConnection>,
    pub in_flight: std::sync::atomic::AtomicUsize,
}

/// Pool of per-DC authenticated connections.
/// Each DC holds up to MAX_CONNS_PER_DC slots. The pool lock is dropped
/// before any network I/O so concurrent callers don't serialize on it.
pub struct DcPool {
    /// Per-DC connection slots; inner Vec holds slot Arcs.
    pub(crate) conns: HashMap<i32, Vec<std::sync::Arc<ConnSlot>>>,
    addrs: HashMap<i32, String>,
    #[allow(dead_code)]
    home_dc_id: i32,
    /// Proxy config forwarded to auto-reconnect.
    socks5: Option<crate::socks5::Socks5Config>,
    /// Transport kind reused for secondary DC connections.
    transport: TransportKind,
    /// DCs that have already received `invokeWithLayer(initConnection(...))`.
    init_done: std::collections::HashSet<i32>,
}

impl DcPool {
    pub fn new(
        home_dc_id: i32,
        dc_entries: &[DcEntry],
        socks5: Option<crate::socks5::Socks5Config>,
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

        let slots = self.conns.get(&dc_id).unwrap();

        // pick least-busy slot
        let best = slots
            .iter()
            .min_by_key(|s| s.in_flight.load(Ordering::Relaxed))
            .unwrap()
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
            self.conns.get_mut(&dc_id).unwrap().push(new_slot);
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
fn build_msgs_ack_body(msg_ids: &[i64]) -> Vec<u8> {
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
fn build_msgs_ack_ping_body(ping_id: i64) -> Vec<u8> {
    // ping_delay_disconnect#f3427b8c ping_id:long disconnect_delay:int = Pong
    let mut out = Vec::with_capacity(4 + 8 + 4);
    out.extend_from_slice(&0xf3427b8c_u32.to_le_bytes()); // constructor
    out.extend_from_slice(&ping_id.to_le_bytes());
    out.extend_from_slice(&75_i32.to_le_bytes()); // disconnect_delay = 75 s
    out
}
