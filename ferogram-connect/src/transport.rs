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

use crate::connection::FrameKind;
use crate::error::ConnectError;

pub async fn send_abridged(stream: &mut TcpStream, data: &[u8]) -> Result<(), ConnectError> {
    debug_assert_eq!(
        data.len() % 4,
        0,
        "abridged send: payload must be 4-byte aligned"
    );
    let words = data.len() / 4;
    // Single combined write: header and payload together to avoid partial-frame delivery.
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

/// Receive raw MTProto frame bytes, respecting the negotiated FrameKind
/// (including Obfuscated AES-256-CTR decryption). Used for the PFS bind
/// response which arrives as an encrypted frame, not a plaintext one.
pub async fn recv_raw_frame(
    stream: &mut TcpStream,
    kind: &FrameKind,
) -> Result<Vec<u8>, ConnectError> {
    match kind {
        FrameKind::Obfuscated { cipher } => {
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
            if words == 0 || words > 0x8000 {
                return Err(ConnectError::other(format!(
                    "obfuscated recv_raw: implausible word count {words}"
                )));
            }
            let mut buf = vec![0u8; words * 4];
            stream.read_exact(&mut buf).await?;
            cipher.lock().await.decrypt(&mut buf);
            Ok(buf)
        }
        FrameKind::Full { .. } => {
            // Full transport: [total_len(4)][seq(4)][payload][crc32(4)]
            let mut len_buf = [0u8; 4];
            stream.read_exact(&mut len_buf).await?;
            let len_i32 = i32::from_le_bytes(len_buf);
            if len_i32 < 0 {
                return Err(ConnectError::TransportCode(len_i32));
            }
            if len_i32 < 12 {
                return Err(ConnectError::other(format!(
                    "Full transport raw: packet too short ({len_i32})"
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
                return Err(ConnectError::other(format!(
                    "Full transport raw: CRC mismatch (got {actual_crc:#010x}, expected {expected_crc:#010x})"
                )));
            }
            // Strip the 4-byte seqno, return payload only.
            Ok(body[4..].to_vec())
        }
        // Abridged and all other transports: plain framing, no extra layer.
        _ => recv_abridged(stream).await,
    }
}

pub async fn recv_abridged(stream: &mut TcpStream) -> Result<Vec<u8>, ConnectError> {
    let mut h = [0u8; 1];
    stream.read_exact(&mut h).await?;
    let words = if h[0] < 0x7f {
        h[0] as usize
    } else {
        let mut b = [0u8; 3];
        stream.read_exact(&mut b).await?;
        let w = b[0] as usize | (b[1] as usize) << 8 | (b[2] as usize) << 16;
        // word count of 1 after 0xFF = Telegram 4-byte transport error code
        if w == 1 {
            let mut code_buf = [0u8; 4];
            stream.read_exact(&mut code_buf).await?;
            let code = i32::from_le_bytes(code_buf);
            return Err(ConnectError::TransportCode(code));
        }
        w
    };
    // Guard against implausibly large reads: a raw 4-byte transport error
    // whose first byte was mis-read as a word count causes a hang otherwise.
    if words == 0 || words > 0x8000 {
        return Err(ConnectError::other(format!(
            "abridged: implausible word count {words} (possible transport error or framing mismatch)"
        )));
    }
    let mut buf = vec![0u8; words * 4];
    stream.read_exact(&mut buf).await?;
    Ok(buf)
}
