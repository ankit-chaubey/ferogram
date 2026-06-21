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
