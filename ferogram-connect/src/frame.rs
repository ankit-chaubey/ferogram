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
            getrandom::getrandom(&mut pad_len_buf).ok();
            let pad_len = (pad_len_buf[0] & 0x0f) as usize;
            let total_payload = data.len() + pad_len;
            let mut frame = Vec::with_capacity(4 + total_payload);
            frame.extend_from_slice(&(total_payload as u32).to_le_bytes());
            frame.extend_from_slice(data);
            let mut pad = vec![0u8; pad_len];
            getrandom::getrandom(&mut pad).ok();
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
            getrandom::getrandom(&mut pad_len_buf).ok();
            let pad_len = (pad_len_buf[0] & 0x0f) as usize;
            let total_payload = data.len() + pad_len;
            let mut frame = Vec::with_capacity(4 + total_payload);
            frame.extend_from_slice(&(total_payload as u32).to_le_bytes());
            frame.extend_from_slice(data);
            let mut pad = vec![0u8; pad_len];
            getrandom::getrandom(&mut pad).ok();
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
                if code < 0 {
                    return Err(ConnectError::TransportCode(code));
                }
            }
            Ok(buf)
        }
        FrameKind::Intermediate => {
            let mut len_buf = [0u8; 4];
            stream.read_exact(&mut len_buf).await?;
            let len_i32 = i32::from_le_bytes(len_buf);
            if len_i32 < 0 {
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
            let mut len_buf = [0u8; 4];
            stream.read_exact(&mut len_buf).await?;
            let total_len_i32 = i32::from_le_bytes(len_buf);
            if total_len_i32 < 0 {
                return Err(ConnectError::TransportCode(total_len_i32));
            }
            let total_len = total_len_i32 as usize;
            if total_len < 12 {
                return Err(ConnectError::other("Full transport: packet too short"));
            }
            let mut rest = vec![0u8; total_len - 4];
            stream.read_exact(&mut rest).await?;
            let (body, crc_bytes) = rest.split_at(rest.len() - 4);
            let expected_crc = u32::from_le_bytes(crc_bytes.try_into().unwrap());
            let mut check_input = Vec::with_capacity(4 + body.len());
            check_input.extend_from_slice(&len_buf);
            check_input.extend_from_slice(body);
            let actual_crc = crate::util::crc32_ieee(&check_input);
            if actual_crc != expected_crc {
                return Err(ConnectError::other(format!(
                    "Full transport: CRC mismatch (got {actual_crc:#010x}, expected {expected_crc:#010x})"
                )));
            }
            let recv_seq = u32::from_le_bytes(body[..4].try_into().unwrap());
            let expected_seq = recv_seqno.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            if recv_seq != expected_seq {
                return Err(ConnectError::other(format!(
                    "Full transport: seqno mismatch (got {recv_seq}, expected {expected_seq})"
                )));
            }
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
                if code < 0 {
                    return Err(ConnectError::TransportCode(code));
                }
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
            let expected_seq = recv_seqno.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            if recv_seq != expected_seq {
                return Err(ConnectError::other(format!(
                    "Full plaintext: seqno mismatch (got {recv_seq}, expected {expected_seq})"
                )));
            }

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
