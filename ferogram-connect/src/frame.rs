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

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio::time::Duration;

use ferogram_tl_types as tl;
use ferogram_tl_types::{Cursor, Deserializable};

use crate::connection::{FrameKind, NO_PING_DISCONNECT, PING_DELAY_SECS};
use crate::error::ConnectError;
use crate::transport::{recv_abridged, send_abridged};
use crate::util::random_i64;

/// Counts every entry into `recv_frame_read`'s `FrameKind::Full` branch
/// across all connections. Incremented before any bytes are consumed so
/// trace logs can correlate call count against recv_seqno advances.
static FULL_RECV_CALL_COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Outcome of a timed frame read attempt.
pub enum FrameOutcome {
    Frame(Vec<u8>),
    Error(ConnectError),
    Keepalive,
}

/// Send a framed message using the active transport kind.
pub async fn send_frame(
    stream: &mut TcpStream,
    data: &[u8],
    kind: &FrameKind,
) -> Result<(), ConnectError> {
    match kind {
        FrameKind::Abridged => send_abridged(stream, data).await,
        FrameKind::Intermediate => {
            let mut frame = Vec::with_capacity(4 + data.len());
            frame.extend_from_slice(&(data.len() as u32).to_le_bytes());
            frame.extend_from_slice(data);
            stream.write_all(&frame).await?;
            Ok(())
        }
        FrameKind::Full { send_seqno, .. } => {
            // Full: [total_len(4)][seq(4)][payload][crc32(4)]
            // total_len covers all 4 fields including itself.
            let seq = send_seqno.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let total_len = (data.len() as u32) + 12;
            let mut packet = Vec::with_capacity(total_len as usize);
            packet.extend_from_slice(&total_len.to_le_bytes());
            packet.extend_from_slice(&seq.to_le_bytes());
            packet.extend_from_slice(data);
            let crc = crate::util::crc32_ieee(&packet);
            packet.extend_from_slice(&crc.to_le_bytes());
            stream.write_all(&packet).await?;
            Ok(())
        }
        FrameKind::Obfuscated { cipher } => {
            // Abridged framing with AES-256-CTR encryption over the whole frame.
            debug_assert_eq!(
                data.len() % 4,
                0,
                "obfuscated send: payload must be 4-byte aligned"
            );
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
            Ok(())
        }
        FrameKind::PaddedIntermediate { cipher } => {
            // Intermediate framing + 0-15 random padding bytes, encrypted.
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
            Ok(())
        }
        FrameKind::FakeTls { cipher } => {
            // Wrap each MTProto message as a TLS Application Data record (type 0x17).
            // Telegram's FakeTLS sends one MTProto frame per TLS record, encrypted
            // with the Obfuscated2 cipher (no real TLS encryption).
            const TLS_APP_DATA: u8 = 0x17;
            const TLS_VER: [u8; 2] = [0x03, 0x03];
            // Split into 2878-byte chunks per TLS record framing.
            const CHUNK: usize = 2878;
            let mut locked = cipher.lock().await;
            for chunk in data.chunks(CHUNK) {
                let chunk_len = chunk.len() as u16;
                let mut record = Vec::with_capacity(5 + chunk.len());
                record.push(TLS_APP_DATA);
                record.extend_from_slice(&TLS_VER);
                record.extend_from_slice(&chunk_len.to_be_bytes());
                record.extend_from_slice(chunk);
                // Encrypt only the payload portion (after the 5-byte header).
                locked.encrypt(&mut record[5..]);
                stream.write_all(&record).await?;
            }
            Ok(())
        }
    }
}

// Split-reader helpers

/// Read one frame with a 60-second keepalive timeout (PING_DELAY_SECS).
///
/// If the timeout fires we send a `PingDelayDisconnect`: this tells Telegram
/// to forcibly close the connection after `NO_PING_DISCONNECT` seconds of
/// silence, giving us a clean EOF to detect rather than a silently stale socket.
/// That mirrors what both  and the official Telegram clients do.
pub async fn recv_frame_with_keepalive(
    rh: &mut OwnedReadHalf,
    fk: &FrameKind,
    writer: &tokio::sync::Mutex<crate::connection::ConnectionWriter>,
    write_half: &tokio::sync::Mutex<OwnedWriteHalf>,
) -> FrameOutcome {
    match tokio::time::timeout(
        Duration::from_secs(PING_DELAY_SECS),
        recv_frame_read(rh, fk),
    )
    .await
    {
        Ok(Ok(raw)) => FrameOutcome::Frame(raw),
        Ok(Err(e)) => FrameOutcome::Error(e),
        Err(_) => {
            // Keepalive timeout: send PingDelayDisconnect so Telegram closes the
            // connection cleanly (EOF) if it hears nothing for NO_PING_DISCONNECT
            // seconds, rather than leaving a silently stale socket.
            let ping_req = tl::functions::PingDelayDisconnect {
                ping_id: random_i64(),
                disconnect_delay: NO_PING_DISCONNECT,
            };
            let (wire, fk) = {
                let mut w = writer.lock().await;
                let fk = w.frame_kind.clone();
                (w.enc.pack(&ping_req), fk)
            };
            match send_frame_write(&mut *write_half.lock().await, &wire, &fk).await {
                Ok(()) => FrameOutcome::Keepalive,
                Err(e) => FrameOutcome::Error(e),
            }
        }
    }
}

/// Send a framed message via an OwnedWriteHalf (split connection).
///
/// Header and payload are combined into a single Vec before calling
/// write_all, reducing write syscalls from 2 -> 1 per frame.  With Abridged
/// framing this previously sent a 1-byte header then the payload in separate
/// syscalls (and two TCP segments even with TCP_NODELAY on fast paths).
pub async fn send_frame_write(
    stream: &mut OwnedWriteHalf,
    data: &[u8],
    kind: &FrameKind,
) -> Result<(), ConnectError> {
    match kind {
        FrameKind::Abridged => {
            // encrypt_data_v2 always produces (24 + 16k) bytes which is always
            // 4-aligned. This assert can never fire; kept as a debug guard in
            // case a future caller passes a non-aligned buffer.
            debug_assert_eq!(
                data.len() % 4,
                0,
                "abridged send: payload must be 4-byte aligned"
            );
            let words = data.len() / 4;
            // Build header + payload in one allocation -> single syscall.
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
            Ok(())
        }
        FrameKind::Intermediate => {
            let mut frame = Vec::with_capacity(4 + data.len());
            frame.extend_from_slice(&(data.len() as u32).to_le_bytes());
            frame.extend_from_slice(data);
            stream.write_all(&frame).await?;
            Ok(())
        }
        FrameKind::Full { send_seqno, .. } => {
            // Full: [total_len(4)][seq(4)][payload][crc32(4)]
            let seq = send_seqno.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let total_len = (data.len() as u32) + 12;
            let mut packet = Vec::with_capacity(total_len as usize);
            packet.extend_from_slice(&total_len.to_le_bytes());
            packet.extend_from_slice(&seq.to_le_bytes());
            packet.extend_from_slice(data);
            let crc = crate::util::crc32_ieee(&packet);
            packet.extend_from_slice(&crc.to_le_bytes());
            stream.write_all(&packet).await?;
            Ok(())
        }
        FrameKind::Obfuscated { cipher } => {
            // Abridged framing + AES-256-CTR encryption (cipher stored).
            debug_assert_eq!(
                data.len() % 4,
                0,
                "obfuscated send: payload must be 4-byte aligned"
            );
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
            Ok(())
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
            Ok(())
        }
        FrameKind::FakeTls { cipher } => {
            const TLS_APP_DATA: u8 = 0x17;
            const TLS_VER: [u8; 2] = [0x03, 0x03];
            const CHUNK: usize = 2878;
            let mut locked = cipher.lock().await;
            for chunk in data.chunks(CHUNK) {
                let chunk_len = chunk.len() as u16;
                let mut record = Vec::with_capacity(5 + chunk.len());
                record.push(TLS_APP_DATA);
                record.extend_from_slice(&TLS_VER);
                record.extend_from_slice(&chunk_len.to_be_bytes());
                record.extend_from_slice(chunk);
                locked.encrypt(&mut record[5..]);
                stream.write_all(&record).await?;
            }
            Ok(())
        }
    }
}

/// Receive a framed message via an OwnedReadHalf (split connection).
pub async fn recv_frame_read(
    stream: &mut OwnedReadHalf,
    kind: &FrameKind,
) -> Result<Vec<u8>, ConnectError> {
    match kind {
        FrameKind::Abridged => {
            // h[0] ranges: 0x00-0x7e = word count, 0x7f = extended, 0x80-0xFF = transport error
            let mut h = [0u8; 1];
            stream.read_exact(&mut h).await?;
            let words = if h[0] < 0x7f {
                h[0] as usize
            } else if h[0] == 0x7f {
                let mut b = [0u8; 3];
                stream.read_exact(&mut b).await?;
                let w = b[0] as usize | (b[1] as usize) << 8 | (b[2] as usize) << 16;
                if w > 4 * 1024 * 1024 {
                    return Err(ConnectError::other(format!(
                        "abridged: implausible word count {w}"
                    )));
                }
                w
            } else {
                let mut rest = [0u8; 3];
                stream.read_exact(&mut rest).await?;
                let code = i32::from_le_bytes([h[0], rest[0], rest[1], rest[2]]);
                return Err(ConnectError::TransportCode(code));
            };
            if words == 0 {
                return Err(ConnectError::other("abridged: zero-length frame"));
            }
            let mut buf = vec![0u8; words * 4];
            stream.read_exact(&mut buf).await?;
            if words == 1 {
                let code = i32::from_le_bytes(buf[..4].try_into().unwrap());
                // A valid encrypted MTProto frame is always >= 40 bytes
                // (key_id=8 + msg_key=16 + 1 AES block=16).  A 1-word frame
                // is always a transport error code, positive or negative.
                // Previously only negative codes were caught here; positive
                // codes slipped into decrypt_data_v2 as InvalidBuffer.
                log::warn!(
                    "[ferogram/connect] abridged 4-byte transport code: {code} ({code:#010x}) \
                     raw=[{:02x} {:02x} {:02x} {:02x}]",
                    buf[0],
                    buf[1],
                    buf[2],
                    buf[3],
                );
                return Err(ConnectError::TransportCode(code));
            }
            Ok(buf)
        }
        FrameKind::Intermediate => {
            let mut len_buf = [0u8; 4];
            stream.read_exact(&mut len_buf).await?;
            let len_i32 = i32::from_le_bytes(len_buf);
            if len_i32 < 0 {
                let status = (-len_i32) as u32;
                log::error!(
                    "[full] transport code triggered raw={:02x} {:02x} {:02x} {:02x} signed={} status={} unsigned={}",
                    len_buf[0],
                    len_buf[1],
                    len_buf[2],
                    len_buf[3],
                    len_i32,
                    status,
                    u32::from_le_bytes(len_buf)
                );
                return Err(ConnectError::TransportCode(len_i32));
            }
            if len_i32 <= 4 {
                let mut code_buf = [0u8; 4];
                stream.read_exact(&mut code_buf).await?;
                let code = i32::from_le_bytes(code_buf);
                return Err(ConnectError::TransportCode(code));
            }
            let len = len_i32 as usize;
            let mut buf = vec![0u8; len];
            stream.read_exact(&mut buf).await?;
            Ok(buf)
        }
        FrameKind::Full { recv_seqno, .. } => {
            // Log before consuming any bytes so early exits (transport error,
            // short frame, CRC mismatch) still appear in the trace.
            let full_call_id =
                FULL_RECV_CALL_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            log::trace!(
                "[full] recv_frame_read entry: call#{full_call_id} recv_seqno={}",
                recv_seqno.load(std::sync::atomic::Ordering::Relaxed),
            );
            let mut len_buf = [0u8; 4];
            stream.read_exact(&mut len_buf).await?;
            let len_i32 = i32::from_le_bytes(len_buf);
            if len_i32 < 0 {
                let status = (-len_i32) as u32;
                log::error!(
                    "[full] transport code triggered raw={:02x} {:02x} {:02x} {:02x} signed={} status={} unsigned={} (call#{full_call_id} recv_seqno={})",
                    len_buf[0],
                    len_buf[1],
                    len_buf[2],
                    len_buf[3],
                    len_i32,
                    status,
                    u32::from_le_bytes(len_buf),
                    recv_seqno.load(std::sync::atomic::Ordering::Relaxed),
                );
                return Err(ConnectError::TransportCode(len_i32));
            }
            if len_i32 < 12 {
                log::error!(
                    "[full] packet too short: len={len_i32} (call#{full_call_id} recv_seqno={})",
                    recv_seqno.load(std::sync::atomic::Ordering::Relaxed),
                );
                return Err(ConnectError::other(format!(
                    "Full transport: packet too short ({len_i32})"
                )));
            }
            let total_len = len_i32 as usize;
            let mut rest = vec![0u8; total_len - 4];
            stream.read_exact(&mut rest).await?;
            let (body, crc_bytes) = rest.split_at(rest.len() - 4);
            let expected_crc = u32::from_le_bytes(crc_bytes.try_into().unwrap());
            let mut check_input = Vec::with_capacity(4 + body.len());
            check_input.extend_from_slice(&len_buf);
            check_input.extend_from_slice(body);
            let actual_crc = crate::util::crc32_ieee(&check_input);
            if actual_crc != expected_crc {
                log::error!(
                    "[full] CRC mismatch: got {actual_crc:#010x}, expected {expected_crc:#010x} (call#{full_call_id} len={total_len} recv_seqno={})",
                    recv_seqno.load(std::sync::atomic::Ordering::Relaxed),
                );
                return Err(ConnectError::other(format!(
                    "Full transport: CRC mismatch (got {actual_crc:#010x}, expected {expected_crc:#010x})"
                )));
            }
            // Grammers-style strict receive sequence validation.
            let recv_seq = i32::from_le_bytes(body[..4].try_into().unwrap());
            let expected_seq = recv_seqno.load(std::sync::atomic::Ordering::Relaxed) as i32;

            log::trace!(
                "[full] recv frame: call#{full_call_id} len={} recv_seq={} expected_seq={} crc={:#010x}",
                total_len,
                recv_seq,
                expected_seq,
                actual_crc
            );

            if recv_seq != expected_seq {
                let hex: String = len_buf
                    .iter()
                    .chain(body.iter().take(16))
                    .map(|b| format!("{b:02x}"))
                    .collect::<Vec<_>>()
                    .join(" ");
                if recv_seq > expected_seq {
                    // Forward gap: server skipped seqnos. This can happen legitimately
                    // after reconnect when the server has already sent frames we didn't
                    // receive. Resync recv_seqno and continue rather than reconnecting.
                    tracing::warn!(
                        "[ferogram/frame] Full transport: forward seqno gap                          (got {recv_seq}, expected {expected_seq}, call#{full_call_id},                          gap={}) first_bytes=[{hex}]; resyncing",
                        recv_seq.wrapping_sub(expected_seq),
                    );
                    recv_seqno.store(
                        recv_seq.wrapping_add(1) as u32,
                        std::sync::atomic::Ordering::Relaxed,
                    );
                    return Ok(body[4..].to_vec());
                } else {
                    // Backward gap: could be replay or stream corruption. Hard error.
                    tracing::error!(
                        "[ferogram/frame] Full transport: backward seqno (got {recv_seq},                          expected {expected_seq}, call#{full_call_id}) first_bytes=[{hex}]"
                    );
                    return Err(ConnectError::other(format!(
                        "Full transport: bad seq (got {}, expected {})",
                        recv_seq, expected_seq
                    )));
                }
            }

            recv_seqno.store(
                expected_seq.wrapping_add(1) as u32,
                std::sync::atomic::Ordering::Relaxed,
            );

            Ok(body[4..].to_vec())
        }
        FrameKind::Obfuscated { cipher } => {
            let mut h = [0u8; 1];
            stream.read_exact(&mut h).await?;
            cipher.lock().await.decrypt(&mut h);
            let words = if h[0] < 0x7f {
                h[0] as usize
            } else if h[0] == 0x7f {
                let mut b = [0u8; 3];
                stream.read_exact(&mut b).await?;
                cipher.lock().await.decrypt(&mut b);
                let w = b[0] as usize | (b[1] as usize) << 8 | (b[2] as usize) << 16;
                if w > 4 * 1024 * 1024 {
                    return Err(ConnectError::other(format!(
                        "obfuscated: implausible word count {w}"
                    )));
                }
                w
            } else {
                let mut rest = [0u8; 3];
                stream.read_exact(&mut rest).await?;
                cipher.lock().await.decrypt(&mut rest);
                let code = i32::from_le_bytes([h[0], rest[0], rest[1], rest[2]]);
                return Err(ConnectError::TransportCode(code));
            };
            if words == 0 {
                return Err(ConnectError::other("obfuscated: zero-length frame"));
            }
            let mut buf = vec![0u8; words * 4];
            stream.read_exact(&mut buf).await?;
            cipher.lock().await.decrypt(&mut buf);
            if words == 1 {
                let code = i32::from_le_bytes(buf[..4].try_into().unwrap());
                // Same as plain abridged: a 1-word frame is always a transport
                // error code regardless of sign.
                log::warn!(
                    "[ferogram/connect] obfuscated 4-byte transport code: {code} ({code:#010x}) \
                     raw=[{:02x} {:02x} {:02x} {:02x}]",
                    buf[0],
                    buf[1],
                    buf[2],
                    buf[3],
                );
                return Err(ConnectError::TransportCode(code));
            }
            Ok(buf)
        }
        FrameKind::PaddedIntermediate { cipher } => {
            // Read 4-byte encrypted length prefix, then payload+padding.
            let mut len_buf = [0u8; 4];
            stream.read_exact(&mut len_buf).await?;
            cipher.lock().await.decrypt(&mut len_buf);
            let total_len = i32::from_le_bytes(len_buf);
            if total_len < 0 {
                return Err(ConnectError::TransportCode(total_len));
            }
            let mut buf = vec![0u8; total_len as usize];
            stream.read_exact(&mut buf).await?;
            cipher.lock().await.decrypt(&mut buf);
            if buf.len() >= 24 {
                let pad = (buf.len() - 24) % 16;
                buf.truncate(buf.len() - pad);
            }
            Ok(buf)
        }
        FrameKind::FakeTls { cipher } => {
            // Read TLS Application Data record: 5-byte header + payload.
            let mut hdr = [0u8; 5];
            stream.read_exact(&mut hdr).await?;
            if hdr[0] != 0x17 {
                return Err(ConnectError::other(format!(
                    "FakeTLS: unexpected record type 0x{:02x}",
                    hdr[0]
                )));
            }
            let payload_len = u16::from_be_bytes([hdr[3], hdr[4]]) as usize;
            let mut buf = vec![0u8; payload_len];
            stream.read_exact(&mut buf).await?;
            cipher.lock().await.decrypt(&mut buf);
            Ok(buf)
        }
    }
}

/// Receive a plaintext (pre-auth) frame and deserialize it.
pub async fn recv_frame_plain<T: Deserializable>(
    stream: &mut TcpStream,
    kind: &FrameKind,
) -> Result<T, ConnectError> {
    // DH handshake uses the same transport framing as all other frames.
    let raw = match kind {
        FrameKind::Abridged => recv_abridged(stream).await?,
        FrameKind::Intermediate => {
            let mut len_buf = [0u8; 4];
            stream.read_exact(&mut len_buf).await?;
            let len = u32::from_le_bytes(len_buf) as usize;
            if len == 0 || len > 1 << 24 {
                return Err(ConnectError::other(format!(
                    "plaintext frame: implausible length {len}"
                )));
            }
            let mut buf = vec![0u8; len];
            stream.read_exact(&mut buf).await?;
            buf
        }
        FrameKind::Full { recv_seqno, .. } => {
            // Full: [total_len(4)][seq(4)][payload][crc32(4)]
            // DH handshake frames use the same seqno counter as encrypted frames.
            // Shares FULL_RECV_CALL_COUNTER with recv_frame_read for tracing.
            let full_call_id =
                FULL_RECV_CALL_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            log::trace!(
                "[full-plain] recv_frame_plain entry: call#{full_call_id} recv_seqno={}",
                recv_seqno.load(std::sync::atomic::Ordering::Relaxed),
            );
            let mut len_buf = [0u8; 4];
            stream.read_exact(&mut len_buf).await?;
            let total_len = u32::from_le_bytes(len_buf) as usize;
            if !(12..=(1 << 24) + 12).contains(&total_len) {
                return Err(ConnectError::other(format!(
                    "Full plaintext frame: implausible total_len {total_len}"
                )));
            }
            let mut rest = vec![0u8; total_len - 4];
            stream.read_exact(&mut rest).await?;

            // Verify CRC-32.
            let (body, crc_bytes) = rest.split_at(rest.len() - 4);
            let expected_crc = u32::from_le_bytes(crc_bytes.try_into().unwrap());
            let mut check_input = Vec::with_capacity(4 + body.len());
            check_input.extend_from_slice(&len_buf);
            check_input.extend_from_slice(body);
            let actual_crc = crate::util::crc32_ieee(&check_input);
            if actual_crc != expected_crc {
                return Err(ConnectError::other(format!(
                    "Full plaintext: CRC mismatch (got {actual_crc:#010x}, expected {expected_crc:#010x})"
                )));
            }

            // Validate and advance seqno.
            let recv_seq = u32::from_le_bytes(body[..4].try_into().unwrap());
            let expected_seq = recv_seqno.load(std::sync::atomic::Ordering::Relaxed);
            log::trace!(
                "[full-plain] recv frame: call#{full_call_id} len={total_len} recv_seq={recv_seq} expected_seq={expected_seq}"
            );
            if recv_seq != expected_seq {
                return Err(ConnectError::other(format!(
                    "Full plaintext: seqno mismatch (got {recv_seq}, expected {expected_seq})"
                )));
            }
            recv_seqno.store(
                expected_seq.wrapping_add(1),
                std::sync::atomic::Ordering::Relaxed,
            );

            body[4..].to_vec()
        }
        FrameKind::Obfuscated { cipher } => {
            // Obfuscated2: Abridged framing with AES-256-CTR decryption.
            let mut h = [0u8; 1];
            stream.read_exact(&mut h).await?;
            cipher.lock().await.decrypt(&mut h);
            let words = if h[0] < 0x7f {
                h[0] as usize
            } else {
                let mut b = [0u8; 3];
                stream.read_exact(&mut b).await?;
                cipher.lock().await.decrypt(&mut b);
                b[0] as usize | (b[1] as usize) << 8 | (b[2] as usize) << 16
            };
            let mut buf = vec![0u8; words * 4];
            stream.read_exact(&mut buf).await?;
            cipher.lock().await.decrypt(&mut buf);
            buf
        }
        FrameKind::PaddedIntermediate { cipher } => {
            let mut len_buf = [0u8; 4];
            stream.read_exact(&mut len_buf).await?;
            cipher.lock().await.decrypt(&mut len_buf);
            let len = u32::from_le_bytes(len_buf) as usize;
            if len == 0 || len > 1 << 24 {
                return Err(ConnectError::other(format!(
                    "PaddedIntermediate plaintext: implausible length {len}"
                )));
            }
            let mut buf = vec![0u8; len];
            stream.read_exact(&mut buf).await?;
            cipher.lock().await.decrypt(&mut buf);
            buf
        }
        FrameKind::FakeTls { cipher } => {
            let mut hdr = [0u8; 5];
            stream.read_exact(&mut hdr).await?;
            if hdr[0] != 0x17 {
                return Err(ConnectError::other(format!(
                    "FakeTLS plaintext: unexpected record type 0x{:02x}",
                    hdr[0]
                )));
            }
            let payload_len = u16::from_be_bytes([hdr[3], hdr[4]]) as usize;
            let mut buf = vec![0u8; payload_len];
            stream.read_exact(&mut buf).await?;
            cipher.lock().await.decrypt(&mut buf);
            buf
        }
    };
    if raw.len() < 20 {
        return Err(ConnectError::other("plaintext frame too short"));
    }
    if u64::from_le_bytes(raw[..8].try_into().unwrap()) != 0 {
        return Err(ConnectError::other("expected auth_key_id=0 in plaintext"));
    }
    let body_len = u32::from_le_bytes(raw[16..20].try_into().unwrap()) as usize;
    if 20 + body_len > raw.len() {
        return Err(ConnectError::other(
            "plaintext frame: body_len exceeds frame size",
        ));
    }
    let mut cur = Cursor::from_slice(&raw[20..20 + body_len]);
    T::deserialize(&mut cur).map_err(Into::into)
}
