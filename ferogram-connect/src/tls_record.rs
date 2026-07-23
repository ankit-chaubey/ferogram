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

//! TLS record byte-stream framing used by the FakeTLS (`0xEE`) MTProxy
//! transport.
//!
//! Real MTProxy FakeTLS is the *same* Obfuscated2/PaddedIntermediate
//! transport used by `dd` secrets. It is simply carried inside a decoy TLS
//! 1.3 handshake and TLS record byte-stream framing so the traffic looks
//! like ordinary HTTPS to DPI. These helpers only deal with that outer
//! record framing -- they know nothing about the inner Obfuscated2 cipher
//! or PaddedIntermediate frame shape, so they're pure and independently
//! testable.

pub const RECORD_HANDSHAKE: u8 = 0x16;
pub const RECORD_CHANGE_CIPHER_SPEC: u8 = 0x14;
pub const RECORD_APPLICATION_DATA: u8 = 0x17;
pub const RECORD_HEADER_LEN: usize = 5;
/// TLS max plaintext record payload size (2^14), the same chunk ceiling
/// tdesktop and other clients split at.
pub const RECORD_MAX_CHUNK: usize = 16384;

/// Wrap already-encrypted bytes into one or more TLS Application Data
/// records (`0x17 0x03 0x03 <len(be16)> <data>`), splitting at
/// [`RECORD_MAX_CHUNK`]. Appends to `out`.
pub fn wrap_application_data(ciphertext: &[u8], out: &mut Vec<u8>) {
    if ciphertext.is_empty() {
        return;
    }
    for chunk in ciphertext.chunks(RECORD_MAX_CHUNK) {
        out.push(RECORD_APPLICATION_DATA);
        out.extend_from_slice(&[0x03, 0x03]);
        out.extend_from_slice(&(chunk.len() as u16).to_be_bytes());
        out.extend_from_slice(chunk);
    }
}

/// The one-time leading ChangeCipherSpec decoy record
/// (`0x14 0x03 0x03 0x00 0x01 0x01`) real MTProxy FakeTLS servers expect
/// before the first real Application Data record.
pub fn change_cipher_spec_record() -> [u8; 6] {
    [0x14, 0x03, 0x03, 0x00, 0x01, 0x01]
}

/// Result of scanning a byte buffer for complete TLS records.
pub struct Unwrapped {
    /// Ciphertext bytes extracted from complete data-bearing records
    /// (ChangeCipherSpec and Application Data), concatenated in wire order.
    pub ciphertext: Vec<u8>,
    /// Bytes consumed from the front of the input (always a whole number of
    /// complete records). The caller should drain this many bytes from its
    /// pending buffer and keep the remainder (a partial trailing record, if
    /// any) for the next call.
    pub consumed: usize,
}

/// Scan `pending` for complete TLS records during steady-state (post
/// handshake) I/O.
///
/// ChangeCipherSpec (`0x14`) and Application Data (`0x17`) record payloads
/// are both treated as real ciphertext bytes: some MTProxy servers echo a
/// ChangeCipherSpec-typed record as their own one-time decoy prefix,
/// mirroring the client's own leading record (see
/// [`change_cipher_spec_record`]), and it must be folded into the same
/// byte stream the client's leading record occupies on the wire.
///
/// A Handshake (`0x16`) record at this point is a protocol error: the decoy
/// handshake is already over by the time this is called.
///
/// Stops at the first incomplete/partial record, leaving it (and anything
/// after) unconsumed.
pub fn unwrap_records(pending: &[u8]) -> Result<Unwrapped, String> {
    let mut offset = 0usize;
    let mut ciphertext = Vec::new();
    loop {
        if pending.len() < offset + RECORD_HEADER_LEN {
            break;
        }
        let rec_type = pending[offset];
        let len = u16::from_be_bytes([pending[offset + 3], pending[offset + 4]]) as usize;
        let total = RECORD_HEADER_LEN + len;
        if pending.len() < offset + total {
            break;
        }
        match rec_type {
            RECORD_APPLICATION_DATA | RECORD_CHANGE_CIPHER_SPEC => {
                ciphertext.extend_from_slice(&pending[offset + RECORD_HEADER_LEN..offset + total]);
            }
            RECORD_HANDSHAKE => {
                return Err(
                    "FakeTLS: unexpected Handshake record after handshake completed".into(),
                );
            }
            other => {
                return Err(format!("FakeTLS: unexpected TLS record type 0x{other:02x}"));
            }
        }
        offset += total;
    }
    Ok(Unwrapped {
        ciphertext,
        consumed: offset,
    })
}

/// A single raw TLS record as read during the handshake: its type byte and
/// full wire bytes (5-byte header included).
pub struct RawRecord {
    pub rec_type: u8,
    pub bytes: Vec<u8>,
}

/// Read exactly one TLS record from `stream`, blocking until the 5-byte
/// header and its declared-length payload have both arrived.
pub async fn read_one_record(stream: &mut tokio::net::TcpStream) -> std::io::Result<RawRecord> {
    use tokio::io::AsyncReadExt;
    let mut hdr = [0u8; RECORD_HEADER_LEN];
    stream.read_exact(&mut hdr).await?;
    let len = u16::from_be_bytes([hdr[3], hdr[4]]) as usize;
    let mut bytes = Vec::with_capacity(RECORD_HEADER_LEN + len);
    bytes.extend_from_slice(&hdr);
    let mut body = vec![0u8; len];
    stream.read_exact(&mut body).await?;
    bytes.extend_from_slice(&body);
    Ok(RawRecord {
        rec_type: hdr[0],
        bytes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wrap_then_unwrap_roundtrip() {
        let data = vec![7u8; 40000]; // spans multiple 16384-byte records
        let mut wire = Vec::new();
        wrap_application_data(&data, &mut wire);

        let result = unwrap_records(&wire).expect("unwrap");
        assert_eq!(result.consumed, wire.len());
        assert_eq!(result.ciphertext, data);
    }

    #[test]
    fn partial_record_left_unconsumed() {
        let data = vec![1u8, 2, 3, 4, 5];
        let mut wire = Vec::new();
        wrap_application_data(&data, &mut wire);
        wire.truncate(wire.len() - 1); // chop the last byte off

        let result = unwrap_records(&wire).expect("unwrap");
        assert_eq!(result.consumed, 0);
        assert!(result.ciphertext.is_empty());
    }

    #[test]
    fn change_cipher_spec_payload_is_folded_in() {
        let mut wire = Vec::new();
        wire.extend_from_slice(&change_cipher_spec_record());
        wrap_application_data(&[9, 9, 9], &mut wire);

        let result = unwrap_records(&wire).expect("unwrap");
        assert_eq!(result.consumed, wire.len());
        assert_eq!(result.ciphertext, vec![1, 9, 9, 9]);
    }
}
