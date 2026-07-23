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

use std::sync::Arc;
use std::time::Duration;

use socket2::TcpKeepalive;
use tokio::io::AsyncWriteExt;
use tokio::net::TcpStream;

use ferogram_mtproto::{EncryptedSession, Session, authentication as auth};
use ferogram_tl_types as tl;

use crate::error::ConnectError;
use crate::frame::{recv_frame_plain, send_frame};
use crate::pfs::decode_bind_response;
use crate::transport::recv_raw_frame;
use crate::transport_kind::TransportKind;

pub const PING_DELAY_SECS: u64 = 60;
pub const NO_PING_DISCONNECT: i32 = 75;

const TCP_KEEPALIVE_IDLE_SECS: u64 = 10;
const TCP_KEEPALIVE_INTERVAL_SECS: u64 = 5;
#[cfg(not(target_os = "windows"))]
const TCP_KEEPALIVE_PROBES: u32 = 3;

/// How framing bytes are sent/received on a connection.
///
/// `Obfuscated` carries an `Arc<Mutex<ObfuscatedCipher>>` so the same cipher
/// state is shared (safely) between the writer task (TX / `encrypt`) and the
/// reader task (RX / `decrypt`).  The two directions are separate AES-CTR
/// instances inside `ObfuscatedCipher`, so locking is only needed to prevent
/// concurrent mutation of the struct, not to serialise TX vs RX.
#[derive(Clone)]
pub enum FrameKind {
    Abridged,
    Intermediate,
    Full {
        send_seqno: Arc<std::sync::atomic::AtomicU32>,
        recv_seqno: Arc<std::sync::atomic::AtomicU32>,
    },
    /// Obfuscated2 over Abridged framing.
    Obfuscated {
        cipher: std::sync::Arc<tokio::sync::Mutex<ferogram_crypto::ObfuscatedCipher>>,
    },
    /// Obfuscated2 over Intermediate+padding framing (`0xDD` MTProxy).
    PaddedIntermediate {
        cipher: std::sync::Arc<tokio::sync::Mutex<ferogram_crypto::ObfuscatedCipher>>,
    },
    /// FakeTLS framing (`0xEE` MTProxy). Same Obfuscated2/PaddedIntermediate
    /// transport as `PaddedIntermediate`, carried inside a decoy TLS
    /// handshake + TLS record byte-stream framing. `cipher` is the *real*
    /// data cipher (a fresh Obfuscated2 keypair generated after the decoy
    /// handshake completes) -- never derived from the ClientHello HMAC.
    FakeTls {
        cipher: std::sync::Arc<tokio::sync::Mutex<ferogram_crypto::ObfuscatedCipher>>,
        /// Raw wire bytes not yet unwrapped from TLS record framing (may
        /// hold a partial trailing record between reads).
        tls_raw_pending: std::sync::Arc<tokio::sync::Mutex<Vec<u8>>>,
        /// Decrypted PaddedIntermediate-stream bytes extracted from
        /// complete TLS records but not yet consumed by frame peeling.
        decoded_pending: std::sync::Arc<tokio::sync::Mutex<Vec<u8>>>,
    },
}

/// A single server-provided salt with its validity window.
///
#[derive(Clone, Debug)]
pub struct FutureSalt {
    pub valid_since: i32,
    /// Stored as `u32` because Telegram sends validity windows that extend
    /// past 2038 (e.g. valid_until ≈ 2_751_656_413, year 2057).  Those values
    /// overflow `i32` and wrap negative, making every salt look expired when
    /// compared against the current server time with a signed comparison.
    pub valid_until: u32,
    pub salt: i64,
}

/// Delay (seconds) before a salt is considered usable after its `valid_since`.
///
pub const SALT_USE_DELAY: i32 = 60;

pub struct Connection {
    pub stream: TcpStream,
    pub enc: EncryptedSession,
    pub frame_kind: FrameKind,
    /// When PFS is active, the permanent auth key (stored in session).
    /// `enc` holds the temp key; this field holds the perm key so
    /// `auth_key_bytes()` returns the right value to persist.
    pub perm_auth_key: Option<[u8; 256]>,
}

impl Connection {
    /// Open a TCP stream, optionally via SOCKS5, and apply transport init bytes.
    async fn open_stream(
        addr: &str,
        socks5: Option<&crate::socks5::Socks5Config>,
        transport: &TransportKind,
        dc_id: i16,
    ) -> Result<(TcpStream, FrameKind), ConnectError> {
        let stream = match socks5 {
            Some(proxy) => proxy.connect(addr).await?,
            None => {
                let stream = TcpStream::connect(addr).await.map_err(ConnectError::Io)?;
                stream.set_nodelay(true).ok();
                {
                    let sock = socket2::SockRef::from(&stream);
                    let keepalive = TcpKeepalive::new()
                        .with_time(Duration::from_secs(TCP_KEEPALIVE_IDLE_SECS))
                        .with_interval(Duration::from_secs(TCP_KEEPALIVE_INTERVAL_SECS));
                    #[cfg(not(target_os = "windows"))]
                    let keepalive = keepalive.with_retries(TCP_KEEPALIVE_PROBES);
                    sock.set_tcp_keepalive(&keepalive).ok();
                }
                stream
            }
        };
        Self::apply_transport_init(stream, transport, dc_id).await
    }

    /// Open a stream routed through an MTProxy (connects to proxy host:port,
    /// not to the Telegram DC address).
    async fn open_stream_mtproxy(
        mtproxy: &crate::proxy::MtProxyConfig,
        dc_id: i16,
    ) -> Result<(TcpStream, FrameKind), ConnectError> {
        let stream = mtproxy.connect().await?;
        stream.set_nodelay(true).ok();
        Self::apply_transport_init(stream, &mtproxy.transport, dc_id).await
    }

    async fn apply_transport_init(
        mut stream: TcpStream,
        transport: &TransportKind,
        dc_id: i16,
    ) -> Result<(TcpStream, FrameKind), ConnectError> {
        match transport {
            TransportKind::Abridged => {
                stream.write_all(&[0xef]).await?;
                Ok((stream, FrameKind::Abridged))
            }
            TransportKind::Intermediate => {
                stream.write_all(&[0xee, 0xee, 0xee, 0xee]).await?;
                Ok((stream, FrameKind::Intermediate))
            }
            TransportKind::Full => {
                // Full transport has no init byte.
                Ok((
                    stream,
                    FrameKind::Full {
                        send_seqno: Arc::new(std::sync::atomic::AtomicU32::new(0)),
                        recv_seqno: Arc::new(std::sync::atomic::AtomicU32::new(0)),
                    },
                ))
            }
            TransportKind::Obfuscated { secret } => {
                let proxy_secret = secret.as_ref().map(|s| s.as_ref());
                let (nonce, cipher) =
                    ferogram_crypto::build_obfuscated_init(0xef, dc_id, proxy_secret);
                stream.write_all(&nonce).await?;
                let cipher_arc = std::sync::Arc::new(tokio::sync::Mutex::new(cipher));
                Ok((stream, FrameKind::Obfuscated { cipher: cipher_arc }))
            }
            TransportKind::PaddedIntermediate { secret } => {
                let proxy_secret = secret.as_ref().map(|s| s.as_ref());
                let (nonce, cipher) =
                    ferogram_crypto::build_obfuscated_init(0xdd, dc_id, proxy_secret);
                stream.write_all(&nonce).await?;
                let cipher_arc = std::sync::Arc::new(tokio::sync::Mutex::new(cipher));
                Ok((stream, FrameKind::PaddedIntermediate { cipher: cipher_arc }))
            }
            TransportKind::FakeTls { secret, domain } => {
                // Real MTProxy FakeTLS is the *same* Obfuscated2/PaddedIntermediate
                // transport used by `dd` secrets, carried inside a decoy TLS 1.3
                // handshake + TLS record byte-stream framing so the traffic looks
                // like ordinary HTTPS to DPI. The ClientHello HMAC below only
                // proves we know `secret` to the proxy -- it is NOT the data
                // cipher. The real data cipher is a fresh Obfuscated2 nonce
                // generated *after* this decoy handshake completes (see below).
                let domain_bytes = domain.as_bytes();
                let mut session_id = [0u8; 32];
                ferogram_crypto::fill_random(&mut session_id);

                // GREASE values (RFC 8701): reserved cipher/extension/group/
                // version IDs of the form 0x?A?A. A real FakeTLS/DPI-aware
                // proxy expects to see these -- a ClientHello that omits them
                // (and omits key_share/signature_algorithms entirely, as the
                // previous minimal hello did) doesn't parse as a plausible
                // TLS 1.3 ClientHello and gets dropped before any reply is
                // sent, which is exactly the "early eof" symptom.
                let mut grease_seed = [0u8; 4];
                ferogram_crypto::fill_random(&mut grease_seed);
                let grease_u16 = |b: u8| -> u16 {
                    let v = (b & 0xf0) | 0x0a;
                    ((v as u16) << 8) | v as u16
                };
                let grease_cipher = grease_u16(grease_seed[0]);
                let grease_group = grease_u16(grease_seed[1]);
                let grease_version = grease_u16(grease_seed[2]);
                let grease_ext = grease_u16(grease_seed[3]);

                // Build ClientHello body (random placeholder = zeros)
                let mut cipher_suites = Vec::new();
                cipher_suites.extend_from_slice(&grease_cipher.to_be_bytes());
                for c in [
                    0x1301u16, 0x1302, 0x1303, 0xc02b, 0xc02f, 0xc02c, 0xc030, 0xcca9, 0xcca8,
                    0xc013, 0xc014, 0x009c, 0x009d, 0x002f, 0x0035,
                ] {
                    cipher_suites.extend_from_slice(&c.to_be_bytes());
                }
                let compression: &[u8] = &[0x01, 0x00];

                let sni_name_len = domain_bytes.len() as u16;
                let sni_list_len = sni_name_len + 3;
                let sni_ext_len = sni_list_len + 2;
                let mut sni_ext = Vec::new();
                sni_ext.extend_from_slice(&[0x00, 0x00]);
                sni_ext.extend_from_slice(&sni_ext_len.to_be_bytes());
                sni_ext.extend_from_slice(&sni_list_len.to_be_bytes());
                sni_ext.push(0x00);
                sni_ext.extend_from_slice(&sni_name_len.to_be_bytes());
                sni_ext.extend_from_slice(domain_bytes);

                let sup_grp: [u8; 12] = [
                    0x00,
                    0x0a,
                    0x00,
                    0x08,
                    0x00,
                    0x06,
                    (grease_group >> 8) as u8,
                    grease_group as u8,
                    0x00,
                    0x1d,
                    0x00,
                    0x17,
                ];
                let ec_point_fmt: &[u8] = &[0x00, 0x0b, 0x00, 0x02, 0x01, 0x00];
                let sig_algs: &[u8] = &[
                    0x00, 0x0d, 0x00, 0x12, 0x00, 0x10, 0x04, 0x03, 0x08, 0x04, 0x04, 0x01, 0x05,
                    0x03, 0x08, 0x05, 0x05, 0x01, 0x08, 0x06, 0x06, 0x01,
                ];
                let alpn: &[u8] = &[
                    0x00, 0x10, 0x00, 0x0e, 0x00, 0x0c, 0x02, b'h', b'2', 0x08, b'h', b't', b't',
                    b'p', b'/', b'1', b'.', b'1',
                ];
                let ems: &[u8] = &[0x00, 0x17, 0x00, 0x00];
                let sess_tick: &[u8] = &[0x00, 0x23, 0x00, 0x00];
                let sup_ver: Vec<u8> = {
                    let mut v = vec![0x00, 0x2b, 0x00, 0x05, 0x04];
                    v.extend_from_slice(&grease_version.to_be_bytes());
                    v.extend_from_slice(&[0x03, 0x04]);
                    v
                };
                let psk_modes: &[u8] = &[0x00, 0x2d, 0x00, 0x02, 0x01, 0x01];
                let mut key_share_pub = [0u8; 32];
                ferogram_crypto::fill_random(&mut key_share_pub);
                let key_share: Vec<u8> = {
                    let mut v = vec![0x00, 0x33, 0x00, 0x26, 0x00, 0x24, 0x00, 0x1d, 0x00, 0x20];
                    v.extend_from_slice(&key_share_pub);
                    v
                };
                let reneg_info: &[u8] = &[0xff, 0x01, 0x00, 0x01, 0x00];
                let grease_ext_entry: [u8; 6] = [
                    (grease_ext >> 8) as u8,
                    grease_ext as u8,
                    0x00,
                    0x02,
                    0x00,
                    0x00,
                ];

                let mut ext_body = Vec::new();
                ext_body.extend_from_slice(&sni_ext);
                ext_body.extend_from_slice(&sup_grp);
                ext_body.extend_from_slice(ec_point_fmt);
                ext_body.extend_from_slice(sig_algs);
                ext_body.extend_from_slice(alpn);
                ext_body.extend_from_slice(ems);
                ext_body.extend_from_slice(sess_tick);
                ext_body.extend_from_slice(&sup_ver);
                ext_body.extend_from_slice(psk_modes);
                ext_body.extend_from_slice(&key_share);
                ext_body.extend_from_slice(reneg_info);
                ext_body.extend_from_slice(&grease_ext_entry);

                // Pad the whole ClientHello up to a size typical of a real
                // browser hello (avoids a suspiciously short record, which
                // is itself a signal a FakeTLS-detecting DPI box looks for).
                const TARGET_HELLO_LEN: usize = 517;
                let prefix_len = 2 /*version*/ + 32 /*random*/ + 1 /*sid len*/
                    + session_id.len() + 2 /*cs len*/ + cipher_suites.len()
                    + compression.len() + 2 /*ext len field*/;
                let unpadded_total = prefix_len + ext_body.len();
                if unpadded_total < TARGET_HELLO_LEN {
                    let pad_needed = TARGET_HELLO_LEN - unpadded_total - 4; // minus padding ext header
                    ext_body.extend_from_slice(&[0x00, 0x15]);
                    ext_body.extend_from_slice(&(pad_needed as u16).to_be_bytes());
                    ext_body.extend(std::iter::repeat_n(0u8, pad_needed));
                }

                let mut extensions = Vec::new();
                extensions.extend_from_slice(&(ext_body.len() as u16).to_be_bytes());
                extensions.extend_from_slice(&ext_body);

                let mut hello_body = Vec::new();
                hello_body.extend_from_slice(&[0x03, 0x03]);
                hello_body.extend_from_slice(&[0u8; 32]); // random placeholder, filled below
                hello_body.push(session_id.len() as u8);
                hello_body.extend_from_slice(&session_id);
                hello_body.extend_from_slice(&(cipher_suites.len() as u16).to_be_bytes());
                hello_body.extend_from_slice(&cipher_suites);
                hello_body.extend_from_slice(compression);
                hello_body.extend_from_slice(&extensions);

                let hs_len = hello_body.len() as u32;
                let mut handshake = vec![
                    0x01,
                    ((hs_len >> 16) & 0xff) as u8,
                    ((hs_len >> 8) & 0xff) as u8,
                    (hs_len & 0xff) as u8,
                ];
                handshake.extend_from_slice(&hello_body);

                let rec_len = handshake.len() as u16;
                let mut record = Vec::new();
                record.push(0x16);
                record.extend_from_slice(&[0x03, 0x01]);
                record.extend_from_slice(&rec_len.to_be_bytes());
                record.extend_from_slice(&handshake);

                // random field lives at: TLS-rec(5) + HS-hdr(4) + version(2) = offset 11
                const CLIENT_RANDOM_OFFSET: usize = 5 + 4 + 2;
                // Digest is HMAC-SHA256(secret, record-with-random-zeroed); the
                // random field is still all-zero at this point.
                let client_digest = ferogram_crypto::fake_tls_client_digest(secret, &record);
                record[CLIENT_RANDOM_OFFSET..CLIENT_RANDOM_OFFSET + 32]
                    .copy_from_slice(&client_digest);
                stream.write_all(&record).await?;

                // Real MTProxy FakeTLS servers reply with exactly three TLS
                // records: Handshake (ServerHello), ChangeCipherSpec, and one
                // Application Data record (padding to round out the
                // digest-covered blob). All three are concatenated (headers
                // included) to verify the server's digest.
                let hello_rec = crate::tls_record::read_one_record(&mut stream)
                    .await
                    .map_err(ConnectError::Io)?;
                if hello_rec.rec_type != crate::tls_record::RECORD_HANDSHAKE {
                    return Err(ConnectError::Other(format!(
                        "FakeTLS: expected ServerHello (Handshake) record, got type 0x{:02x}",
                        hello_rec.rec_type
                    )));
                }
                let ccs_rec = crate::tls_record::read_one_record(&mut stream)
                    .await
                    .map_err(ConnectError::Io)?;
                if ccs_rec.rec_type != crate::tls_record::RECORD_CHANGE_CIPHER_SPEC {
                    return Err(ConnectError::Other(format!(
                        "FakeTLS: expected ChangeCipherSpec record, got type 0x{:02x}",
                        ccs_rec.rec_type
                    )));
                }
                let app_rec = crate::tls_record::read_one_record(&mut stream)
                    .await
                    .map_err(ConnectError::Io)?;
                if app_rec.rec_type != crate::tls_record::RECORD_APPLICATION_DATA {
                    return Err(ConnectError::Other(format!(
                        "FakeTLS: expected Application Data record, got type 0x{:02x}",
                        app_rec.rec_type
                    )));
                }

                let mut packet = Vec::with_capacity(
                    hello_rec.bytes.len() + ccs_rec.bytes.len() + app_rec.bytes.len(),
                );
                packet.extend_from_slice(&hello_rec.bytes);
                packet.extend_from_slice(&ccs_rec.bytes);
                packet.extend_from_slice(&app_rec.bytes);

                // server random/digest lives at the same offset as the client's:
                // TLS-rec(5) + HS-hdr(4) + version(2) = 11, within the ServerHello record.
                const SERVER_DIGEST_OFFSET: usize = 11;
                if packet.len() < SERVER_DIGEST_OFFSET + 32 {
                    return Err(ConnectError::Other("FakeTLS: ServerHello too short".into()));
                }
                let mut server_digest = [0u8; 32];
                server_digest
                    .copy_from_slice(&packet[SERVER_DIGEST_OFFSET..SERVER_DIGEST_OFFSET + 32]);
                packet[SERVER_DIGEST_OFFSET..SERVER_DIGEST_OFFSET + 32].fill(0);

                if !ferogram_crypto::fake_tls_verify_server_digest(
                    secret,
                    &client_digest,
                    &packet,
                    &server_digest,
                ) {
                    return Err(ConnectError::Other(
                        "FakeTLS: server digest verification failed (wrong secret, wrong \
                         domain, or the proxy does not speak FakeTLS)"
                            .into(),
                    ));
                }

                // Decoy handshake verified. Now do the *real* handshake: a
                // fresh Obfuscated2/PaddedIntermediate nonce, sent through the
                // TLS record wrapper exactly like the plain `dd` connection
                // start, prefixed once by the leading ChangeCipherSpec decoy
                // record several proxies require.
                let (nonce, cipher) =
                    ferogram_crypto::build_obfuscated_init(0xdd, dc_id, Some(secret.as_ref()));

                let mut first_write = Vec::new();
                first_write.extend_from_slice(&crate::tls_record::change_cipher_spec_record());
                crate::tls_record::wrap_application_data(&nonce, &mut first_write);
                stream.write_all(&first_write).await?;

                let cipher_arc = std::sync::Arc::new(tokio::sync::Mutex::new(cipher));
                Ok((
                    stream,
                    FrameKind::FakeTls {
                        cipher: cipher_arc,
                        tls_raw_pending: std::sync::Arc::new(tokio::sync::Mutex::new(Vec::new())),
                        decoded_pending: std::sync::Arc::new(tokio::sync::Mutex::new(Vec::new())),
                    },
                ))
            }
            TransportKind::Http => {
                // HTTP transport is handled in dc_pool - fall back to Abridged framing.
                stream.write_all(&[0xef]).await?;
                Ok((stream, FrameKind::Abridged))
            }
        }
    }

    /// Open a TCP stream and apply transport framing, returning the stream and FrameKind.
    ///
    /// Used by `ferogram-mtsender` for the connect-with-key path where DH is not needed
    /// (the auth key is already known). Socket options and transport init are handled here.
    pub async fn open_stream_pub(
        addr: &str,
        dc_id: i16,
        transport: &TransportKind,
        socks5: Option<&crate::socks5::Socks5Config>,
        mtproxy: Option<&crate::proxy::MtProxyConfig>,
    ) -> Result<(TcpStream, FrameKind), ConnectError> {
        if let Some(mp) = mtproxy {
            Self::open_stream_mtproxy(mp, dc_id).await
        } else {
            Self::open_stream(addr, socks5, transport, dc_id).await
        }
    }

    /// Open a fresh connection and run the full unauthenticated DH key
    /// exchange to produce a brand new permanent auth key. Use
    /// [`Self::connect_with_key`] instead once an auth key already exists
    /// for the DC, since redoing DH on every reconnect is wasted work.
    pub async fn connect_raw(
        addr: &str,
        socks5: Option<&crate::socks5::Socks5Config>,
        mtproxy: Option<&crate::proxy::MtProxyConfig>,
        transport: &TransportKind,
        dc_id: i16,
    ) -> Result<Self, ConnectError> {
        let t_label = match transport {
            TransportKind::Abridged => "Abridged",
            TransportKind::Obfuscated { .. } => "Obfuscated",
            TransportKind::PaddedIntermediate { .. } => "PaddedIntermediate",
            TransportKind::Http => "Http",
            TransportKind::Intermediate => "Intermediate",
            TransportKind::Full => "Full",
            TransportKind::FakeTls { .. } => "FakeTls",
        };
        tracing::debug!("[ferogram::connect] starting DH handshake with {addr} via {t_label}");

        let addr2 = addr.to_string();
        let socks5_c = socks5.cloned();
        let mtproxy_c = mtproxy.cloned();
        let transport_c = transport.clone();

        let fut = async move {
            let (mut stream, frame_kind) = if let Some(ref mp) = mtproxy_c {
                Self::open_stream_mtproxy(mp, dc_id).await?
            } else {
                Self::open_stream(&addr2, socks5_c.as_ref(), &transport_c, dc_id).await?
            };

            let mut plain = Session::new();

            let (req1, s1) = auth::step1().map_err(|e| ConnectError::other(e.to_string()))?;
            send_frame(
                &mut stream,
                &plain.pack(&req1).to_plaintext_bytes(),
                &frame_kind,
            )
            .await?;
            let res_pq: tl::enums::ResPq = recv_frame_plain(&mut stream, &frame_kind).await?;

            let (req2, s2) = auth::step2(s1, res_pq, dc_id as i32)
                .map_err(|e| ConnectError::other(e.to_string()))?;
            send_frame(
                &mut stream,
                &plain.pack(&req2).to_plaintext_bytes(),
                &frame_kind,
            )
            .await?;
            let dh: tl::enums::ServerDhParams = recv_frame_plain(&mut stream, &frame_kind).await?;

            let (req3, s3) = auth::step3(s2, dh).map_err(|e| ConnectError::other(e.to_string()))?;
            send_frame(
                &mut stream,
                &plain.pack(&req3).to_plaintext_bytes(),
                &frame_kind,
            )
            .await?;
            let ans: tl::enums::SetClientDhParamsAnswer =
                recv_frame_plain(&mut stream, &frame_kind).await?;

            // Retry loop for dh_gen_retry (up to 5 attempts).
            let done = {
                let mut result =
                    auth::finish(s3, ans).map_err(|e| ConnectError::other(e.to_string()))?;
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
                                return Err(ConnectError::other(
                                    "dh_gen_retry exceeded 5 attempts",
                                ));
                            }
                            let (req_retry, s3_retry) = auth::retry_step3(
                                &dh_params,
                                nonce,
                                server_nonce,
                                new_nonce,
                                retry_id,
                            )
                            .map_err(|e| ConnectError::other(e.to_string()))?;
                            send_frame(
                                &mut stream,
                                &plain.pack(&req_retry).to_plaintext_bytes(),
                                &frame_kind,
                            )
                            .await?;
                            let ans_retry: tl::enums::SetClientDhParamsAnswer =
                                recv_frame_plain(&mut stream, &frame_kind).await?;
                            result = auth::finish(s3_retry, ans_retry)
                                .map_err(|e| ConnectError::other(e.to_string()))?;
                        }
                    }
                }
            };
            tracing::debug!("[ferogram::connect] DH handshake complete, auth key established");

            Ok::<Self, ConnectError>(Self {
                stream,
                enc: EncryptedSession::new(done.auth_key, done.first_salt, done.time_offset),
                frame_kind,
                perm_auth_key: None, // connect_raw produces the perm key itself
            })
        };

        tokio::time::timeout(Duration::from_secs(15), fut)
            .await
            .map_err(|_| {
                ConnectError::other(format!("DH handshake with {addr} timed out after 15 s"))
            })?
    }

    /// Open a connection using an `auth_key` already negotiated for this DC,
    /// skipping the DH handshake entirely. If `pfs` is set, also binds a
    /// temporary key for this session via `auth.bindTempAuthKey`, falling
    /// back to the permanent key if the bind fails.
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
    ) -> Result<Self, ConnectError> {
        let addr2 = addr.to_string();
        let socks5_c = socks5.cloned();
        let mtproxy_c = mtproxy.cloned();
        let transport_c = transport.clone();

        let fut = async move {
            let (mut stream, frame_kind) = if let Some(ref mp) = mtproxy_c {
                Self::open_stream_mtproxy(mp, dc_id).await?
            } else {
                Self::open_stream(&addr2, socks5_c.as_ref(), &transport_c, dc_id).await?
            };
            if pfs {
                tracing::debug!("[ferogram::connect] PFS: binding temporary key for DC{dc_id}");
                match Self::do_pfs_bind(&mut stream, &frame_kind, &auth_key, dc_id).await {
                    Ok(temp_enc) => {
                        tracing::debug!(
                            "[ferogram::connect] PFS: temporary key bound for DC{dc_id}"
                        );
                        return Ok(Self {
                            stream,
                            enc: temp_enc,
                            frame_kind,
                            perm_auth_key: Some(auth_key),
                        });
                    }
                    Err(e) => {
                        tracing::warn!(
                            "[ferogram::connect] PFS bind failed for DC{dc_id} ({e}); falling back to permanent key"
                        );
                        // Graceful fallback: reconnect because DH frames left the stream dirty.
                        // Return error and let the caller handle retry without PFS.
                        return Err(e);
                    }
                }
            }
            Ok::<Self, ConnectError>(Self {
                stream,
                enc: EncryptedSession::new(auth_key, first_salt, time_offset),
                frame_kind,
                perm_auth_key: None,
            })
        };

        tokio::time::timeout(Duration::from_secs(30), fut)
            .await
            .map_err(|_| {
                ConnectError::Io(std::io::Error::new(
                    std::io::ErrorKind::TimedOut,
                    format!("connect_with_key to {addr} timed out after 30 s"),
                ))
            })?
    }

    /// Perform a fresh temp-key DH on an already-open stream, then
    /// send `auth.bindTempAuthKey` encrypted with the temp key.
    /// Returns an `EncryptedSession` keyed with the bound temp key.
    async fn do_pfs_bind(
        stream: &mut TcpStream,
        frame_kind: &FrameKind,
        perm_auth_key: &[u8; 256],
        dc_id: i16,
    ) -> Result<EncryptedSession, ConnectError> {
        use ferogram_mtproto::{
            auth_key_id_from_key, encrypt_bind_inner, gen_msg_id, new_seen_msg_ids,
            serialize_bind_temp_auth_key,
        };
        const TEMP_EXPIRES: i32 = 86_400; // 24 h

        // temp-key DH
        let mut plain = Session::new();

        let (req1, s1) = auth::step1().map_err(|e| ConnectError::other(e.to_string()))?;
        send_frame(stream, &plain.pack(&req1).to_plaintext_bytes(), frame_kind).await?;
        let res_pq: tl::enums::ResPq = recv_frame_plain(stream, frame_kind).await?;

        let (req2, s2) = ferogram_mtproto::step2_temp(s1, res_pq, dc_id as i32, TEMP_EXPIRES)
            .map_err(|e| ConnectError::other(e.to_string()))?;
        send_frame(stream, &plain.pack(&req2).to_plaintext_bytes(), frame_kind).await?;
        let dh: tl::enums::ServerDhParams = recv_frame_plain(stream, frame_kind).await?;

        let (req3, s3) = auth::step3(s2, dh).map_err(|e| ConnectError::other(e.to_string()))?;
        send_frame(stream, &plain.pack(&req3).to_plaintext_bytes(), frame_kind).await?;
        let ans: tl::enums::SetClientDhParamsAnswer = recv_frame_plain(stream, frame_kind).await?;

        let done = {
            let mut result =
                auth::finish(s3, ans).map_err(|e| ConnectError::other(e.to_string()))?;
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
                            return Err(ConnectError::other(
                                "PFS temp DH retry exceeded 5 attempts",
                            ));
                        }
                        let (rr, s3r) = ferogram_mtproto::retry_step3(
                            &dh_params,
                            nonce,
                            server_nonce,
                            new_nonce,
                            retry_id,
                        )
                        .map_err(|e| ConnectError::other(e.to_string()))?;
                        send_frame(stream, &plain.pack(&rr).to_plaintext_bytes(), frame_kind)
                            .await?;
                        let ar: tl::enums::SetClientDhParamsAnswer =
                            recv_frame_plain(stream, frame_kind).await?;
                        result = auth::finish(s3r, ar)
                            .map_err(|e| ConnectError::other(e.to_string()))?;
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
        send_frame(stream, &wire, frame_kind).await?;

        // Receive and verify response.
        // The server may send informational frames first (msgs_ack, new_session_created)
        // before the actual rpc_result{boolTrue}, so we loop up to 5 frames.
        for attempt in 0u8..5 {
            let mut raw = recv_raw_frame(stream, frame_kind).await?;
            let decrypted = temp_enc
                .unpack(&mut raw)
                .map_err(|e| ConnectError::other(format!("PFS bind decrypt: {e:?}")))?;
            match decode_bind_response(&decrypted.body) {
                Ok(()) => {
                    // bindTempAuthKey succeeds under the temp key; keep the session
                    // sequence as-is so subsequent RPCs continue from the same MTProto
                    // message stream.
                    return Ok(temp_enc);
                }
                Err(ref e) if e == "__need_more__" => {
                    tracing::debug!(
                        "[ferogram::connect] PFS (DC{dc_id}): got informational frame on attempt {attempt}, reading next"
                    );
                    continue;
                }
                Err(reason) => {
                    tracing::error!(
                        "[ferogram::connect] PFS bind rejected by server for DC{dc_id}: {reason}"
                    );
                    return Err(ConnectError::other(format!(
                        "auth.bindTempAuthKey: {reason}"
                    )));
                }
            }
        }
        Err(ConnectError::other(
            "auth.bindTempAuthKey: no boolTrue after 5 frames",
        ))
    }

    /// The permanent auth key, for persisting to the session. Under PFS this
    /// is `perm_auth_key`, not the short-lived temp key the connection is
    /// actually encrypted with.
    pub fn auth_key_bytes(&self) -> [u8; 256] {
        // When PFS is active, perm_auth_key is the key to persist in the session.
        // enc.auth_key_bytes() would return the short-lived temp key instead.
        self.perm_auth_key
            .unwrap_or_else(|| self.enc.auth_key_bytes())
    }

    /// Open a TCP connection, negotiate transport framing, and complete the MTProto DH handshake.
    ///
    /// Returns `(stream, frame_kind, session)` as owned values. The caller is responsible for
    /// setting up reader/writer tasks. This is the single authoritative connection path;
    /// `ferogram-mtsender` delegates here instead of reimplementing the DH sequence.
    pub async fn connect_to_dc(
        addr: &str,
        dc_id: i16,
        transport: &TransportKind,
        socks5: Option<&crate::socks5::Socks5Config>,
        mtproxy: Option<&crate::proxy::MtProxyConfig>,
    ) -> Result<(TcpStream, FrameKind, EncryptedSession), ConnectError> {
        let conn = Self::connect_raw(addr, socks5, mtproxy, transport, dc_id).await?;
        Ok((conn.stream, conn.frame_kind, conn.enc))
    }
}

/// Free-function wrapper around [`Connection::connect_to_dc`].
///
/// Opens a TCP connection, negotiates transport framing, and completes the
/// MTProto DH handshake. Returns `(stream, frame_kind, session)`.
pub async fn connect_to_dc(
    addr: &str,
    dc_id: i16,
    transport: &TransportKind,
    socks5: Option<&crate::socks5::Socks5Config>,
    mtproxy: Option<&crate::proxy::MtProxyConfig>,
) -> Result<(TcpStream, FrameKind, EncryptedSession), ConnectError> {
    Connection::connect_to_dc(addr, dc_id, transport, socks5, mtproxy).await
}
