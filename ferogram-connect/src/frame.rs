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

use ferogram_tl_types::{Cursor, Deserializable};

use crate::connection::FrameKind;
use crate::error::ConnectError;
use crate::transport::{recv_abridged, send_abridged};

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
        FrameKind::FakeTls { cipher, .. } => {
            // Real ee framing: PaddedIntermediate frame (len + payload + 0-15
            // random pad bytes), AES-256-CTR encrypted, then wrapped in TLS
            // Application Data records. The leading ChangeCipherSpec decoy
            // record was already sent once as part of the handshake
            // (alongside the connection-start nonce), so every send_frame
            // call after that is a plain Application Data write.
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
            crate::tls_record::wrap_application_data(&frame, &mut wire);
            stream.write_all(&wire).await?;
            Ok(())
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
            static PLAIN_RECV_CALL_COUNTER: std::sync::atomic::AtomicU64 =
                std::sync::atomic::AtomicU64::new(0);
            let full_call_id =
                PLAIN_RECV_CALL_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            tracing::trace!(
                call_id = full_call_id,
                recv_seqno = recv_seqno.load(std::sync::atomic::Ordering::Relaxed),
                "[ferogram::connect] Full transport: recv_frame_plain entered"
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
            tracing::trace!(
                call_id = full_call_id,
                total_len,
                recv_seq,
                expected_seq,
                "[ferogram::connect] Full transport: frame received, seqno verified"
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
        FrameKind::FakeTls {
            cipher,
            decoded_pending,
            ..
        } => {
            let mut len_buf = [0u8; 4];
            faketls_read_exact(stream, cipher, decoded_pending, &mut len_buf).await?;
            let len = u32::from_le_bytes(len_buf) as usize;
            if len == 0 || len > 1 << 24 {
                return Err(ConnectError::other(format!(
                    "FakeTLS plaintext: implausible length {len}"
                )));
            }
            let mut buf = vec![0u8; len];
            faketls_read_exact(stream, cipher, decoded_pending, &mut buf).await?;
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

/// Pull `out.len()` decrypted PaddedIntermediate-stream bytes for the FakeTLS
/// transport, reading and unwrapping whole TLS records from `stream` one at
/// a time as needed. Used only pre-auth (DH handshake): a handful of
/// round-trips, so one-record-at-a-time blocking reads are simplest and
/// avoid any partial-record bookkeeping here (that bookkeeping lives in
/// `MtpSender`'s bulk read path instead, sharing `decoded_pending` with this
/// function across the handshake -> post-auth handoff).
/// Pull `out.len()` decrypted PaddedIntermediate-stream bytes for the FakeTLS
/// transport, reading and unwrapping whole TLS records from `stream` one at
/// a time as needed. Shares `decoded_pending` with whatever else uses this
/// `FrameKind::FakeTls` (e.g. across the pre-auth handshake -> `MtpSender`
/// handoff, or `DcConnection`'s own recv loop), so partial reads never lose
/// bytes across callers.
pub async fn faketls_read_exact(
    stream: &mut TcpStream,
    cipher: &std::sync::Arc<tokio::sync::Mutex<ferogram_crypto::ObfuscatedCipher>>,
    decoded_pending: &std::sync::Arc<tokio::sync::Mutex<Vec<u8>>>,
    out: &mut [u8],
) -> Result<(), ConnectError> {
    loop {
        {
            let decoded = decoded_pending.lock().await;
            if decoded.len() >= out.len() {
                break;
            }
        }
        let rec = crate::tls_record::read_one_record(stream)
            .await
            .map_err(ConnectError::Io)?;
        match rec.rec_type {
            crate::tls_record::RECORD_APPLICATION_DATA
            | crate::tls_record::RECORD_CHANGE_CIPHER_SPEC => {
                let mut payload = rec.bytes[crate::tls_record::RECORD_HEADER_LEN..].to_vec();
                cipher.lock().await.decrypt(&mut payload);
                decoded_pending.lock().await.extend_from_slice(&payload);
            }
            other => {
                return Err(ConnectError::other(format!(
                    "FakeTLS: unexpected TLS record type 0x{other:02x} in data stream"
                )));
            }
        }
    }

    let mut decoded = decoded_pending.lock().await;
    out.copy_from_slice(&decoded[..out.len()]);
    decoded.drain(..out.len());
    Ok(())
}
