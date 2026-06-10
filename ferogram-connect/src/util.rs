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

use std::time::Duration;

use crate::error::ConnectError;

/// CRC-32 using the standard IEEE 802.3 polynomial (for Full transport framing).
pub fn crc32_ieee(data: &[u8]) -> u32 {
    const POLY: u32 = 0xedb88320;
    let mut crc: u32 = 0xffffffff;
    for &byte in data {
        let mut b = byte as u32;
        for _ in 0..8 {
            let mix = (crc ^ b) & 1;
            crc >>= 1;
            if mix != 0 {
                crc ^= POLY;
            }
            b >>= 1;
        }
    }
    crc ^ 0xffffffff
}

/// Minimum body size above which we attempt zlib compression.
pub const COMPRESSION_THRESHOLD: usize = 512;

const ID_GZIP_PACKED: u32 = 0x3072cfa1;
const ID_MSGS_ACK: u32 = 0x62d6b459;
const ID_MSG_CONTAINER: u32 = 0x73f1f8dc;

pub fn random_i64() -> i64 {
    let mut b = [0u8; 8];
    ferogram_crypto::fill_random(&mut b);
    i64::from_le_bytes(b)
}

/// Apply ±20 % random jitter to a backoff delay.
/// Prevents thundering-herd when many clients reconnect simultaneously
/// (e.g. after a server restart or a shared network outage).
pub fn jitter_delay(base_ms: u64) -> Duration {
    // Use two random bytes for the jitter factor (0..=65535 -> 0.80 … 1.20).
    let mut b = [0u8; 2];
    ferogram_crypto::fill_random(&mut b);
    let rand_frac = u16::from_le_bytes(b) as f64 / 65535.0; // 0.0 … 1.0
    let factor = 0.80 + rand_frac * 0.40; // 0.80 … 1.20
    Duration::from_millis((base_ms as f64 * factor) as u64)
}

pub fn tl_read_bytes(data: &[u8]) -> Option<Vec<u8>> {
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

pub fn tl_read_string(data: &[u8]) -> Option<String> {
    tl_read_bytes(data).map(|b| String::from_utf8_lossy(&b).into_owned())
}

pub fn gz_inflate(data: &[u8]) -> Result<Vec<u8>, ConnectError> {
    use std::io::Read;
    let mut out = Vec::new();
    if flate2::read::GzDecoder::new(data)
        .read_to_end(&mut out)
        .is_ok()
        && !out.is_empty()
    {
        return Ok(out);
    }
    out.clear();
    flate2::read::ZlibDecoder::new(data)
        .read_to_end(&mut out)
        .map_err(|_| ConnectError::other("decompression failed"))?;
    Ok(out)
}

pub fn maybe_gz_decompress(body: Vec<u8>) -> Result<Vec<u8>, ConnectError> {
    const ID_GZIP_PACKED_LOCAL: u32 = 0x3072cfa1;
    if body.len() >= 4 && u32::from_le_bytes(body[0..4].try_into().unwrap()) == ID_GZIP_PACKED_LOCAL
    {
        let bytes = tl_read_bytes(&body[4..]).unwrap_or_default();
        gz_inflate(&bytes)
    } else {
        Ok(body)
    }
}

/// TL `bytes` wire encoding (used inside gzip_packed).
pub fn tl_write_bytes(data: &[u8]) -> Vec<u8> {
    let len = data.len();
    let mut out = Vec::with_capacity(4 + len);
    if len < 254 {
        out.push(len as u8);
        out.extend_from_slice(data);
        let pad = (4 - (1 + len) % 4) % 4;
        out.extend(std::iter::repeat_n(0u8, pad));
    } else {
        out.push(0xfe);
        out.extend_from_slice(&(len as u32).to_le_bytes()[..3]);
        out.extend_from_slice(data);
        let pad = (4 - (4 + len) % 4) % 4;
        out.extend(std::iter::repeat_n(0u8, pad));
    }
    out
}

/// Wrap `data` in a `gzip_packed#3072cfa1 packed_data:bytes` TL frame.
pub fn gz_pack_body(data: &[u8]) -> Vec<u8> {
    use std::io::Write;
    let mut enc = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
    let _ = enc.write_all(data);
    let compressed = enc.finish().unwrap_or_default();
    let mut out = Vec::with_capacity(4 + 4 + compressed.len());
    out.extend_from_slice(&ID_GZIP_PACKED.to_le_bytes());
    out.extend(tl_write_bytes(&compressed));
    out
}

/// Optionally compress `data`.  Returns the compressed `gzip_packed` wrapper
/// if it is shorter than the original; otherwise returns `data` unchanged.
pub fn maybe_gz_pack(data: &[u8]) -> Vec<u8> {
    if data.len() <= COMPRESSION_THRESHOLD {
        return data.to_vec();
    }
    let packed = gz_pack_body(data);
    if packed.len() < data.len() {
        packed
    } else {
        data.to_vec()
    }
}

// +: MsgsAck body builder

/// Build the TL body for `msgs_ack#62d6b459 msg_ids:Vector<long>`.
pub fn build_msgs_ack_body(msg_ids: &[i64]) -> Vec<u8> {
    // msgs_ack#62d6b459 msg_ids:Vector<long>
    // Vector<long>: 0x1cb5c415 + count:int + [i64...]
    let mut out = Vec::with_capacity(4 + 4 + 4 + msg_ids.len() * 8);
    out.extend_from_slice(&ID_MSGS_ACK.to_le_bytes());
    out.extend_from_slice(&0x1cb5c415_u32.to_le_bytes()); // Vector constructor
    out.extend_from_slice(&(msg_ids.len() as u32).to_le_bytes());
    for &id in msg_ids {
        out.extend_from_slice(&id.to_le_bytes());
    }
    out
}

/// Build the body of a `msg_container#73f1f8dc` from a list of
/// `(msg_id, seqno, body)` inner messages.
///
/// The caller is responsible for allocating msg_id and seqno for each entry
/// via `EncryptedSession::alloc_msg_seqno`.
pub fn build_container_body(messages: &[(i64, i32, &[u8])]) -> Vec<u8> {
    let total_body: usize = messages.iter().map(|(_, _, b)| 16 + b.len()).sum();
    let mut out = Vec::with_capacity(8 + total_body);
    out.extend_from_slice(&ID_MSG_CONTAINER.to_le_bytes());
    out.extend_from_slice(&(messages.len() as u32).to_le_bytes());
    for &(msg_id, seqno, body) in messages {
        out.extend_from_slice(&msg_id.to_le_bytes());
        out.extend_from_slice(&seqno.to_le_bytes());
        out.extend_from_slice(&(body.len() as u32).to_le_bytes());
        out.extend_from_slice(body);
    }
    out
}
