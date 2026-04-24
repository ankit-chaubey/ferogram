// Copyright (c) Ankit Chaubey <ankitchaubey.dev@gmail.com>
// SPDX-License-Identifier: MIT OR Apache-2.0
//
// ferogram: async Telegram MTProto client in Rust
// https://github.com/ankit-chaubey/ferogram
//
// If you use or modify this code, keep this notice at the top of your file
// and include the LICENSE-MIT or LICENSE-APACHE file from this repository:
// https://github.com/ankit-chaubey/ferogram

use crate::InvocationError;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

// Intermediate

/// [MTProto Intermediate] transport framing.
///
/// Init byte: `0xeeeeeeee` (4 bytes).  Each message is prefixed with its
/// 4-byte little-endian byte length.
///
/// [MTProto Intermediate]: https://core.telegram.org/mtproto/mtproto-transports#intermediate
pub struct IntermediateTransport {
    stream: TcpStream,
    init_sent: bool,
}

impl IntermediateTransport {
    /// Connect and send the 4-byte init header.
    pub async fn connect(addr: &str) -> Result<Self, InvocationError> {
        let stream = TcpStream::connect(addr).await?;
        Ok(Self {
            stream,
            init_sent: false,
        })
    }

    /// Wrap an existing stream (the init byte will be sent on first [`send`]).
    pub fn from_stream(stream: TcpStream) -> Self {
        Self {
            stream,
            init_sent: false,
        }
    }

    /// Send a message with Intermediate framing.
    pub async fn send(&mut self, data: &[u8]) -> Result<(), InvocationError> {
        if !self.init_sent {
            self.stream.write_all(&[0xee, 0xee, 0xee, 0xee]).await?;
            self.init_sent = true;
        }
        let len = (data.len() as u32).to_le_bytes();
        self.stream.write_all(&len).await?;
        self.stream.write_all(data).await?;
        Ok(())
    }

    /// Receive the next Intermediate-framed message.
    pub async fn recv(&mut self) -> Result<Vec<u8>, InvocationError> {
        let mut len_buf = [0u8; 4];
        self.stream.read_exact(&mut len_buf).await?;
        let raw = i32::from_le_bytes(len_buf);
        if raw < 0 {
            return Err(InvocationError::Io(std::io::Error::new(
                std::io::ErrorKind::ConnectionRefused,
                format!("transport error: {raw}"),
            )));
        }
        let len = raw as usize;
        let mut buf = vec![0u8; len];
        self.stream.read_exact(&mut buf).await?;
        Ok(buf)
    }

    pub fn into_inner(self) -> TcpStream {
        self.stream
    }
}

// Padded Intermediate

/// [MTProto Padded Intermediate] transport framing.
///
/// Init tag: `0xdddddddd` (4 bytes).  Each message is sent as:
/// `[4-byte LE length of (payload + random padding)][payload][0–15 random bytes]`
///
/// This is the correct framing for `0xDD` MTProxy secrets.
///
/// [MTProto Padded Intermediate]: https://core.telegram.org/mtproto/mtproto-transports#padded-intermediate
pub struct PaddedIntermediateTransport {
    stream: TcpStream,
    init_sent: bool,
}

impl PaddedIntermediateTransport {
    /// Connect to `addr` and lazily send the `0xDDDDDDDD` init tag on first [`send`].
    pub async fn connect(addr: &str) -> Result<Self, InvocationError> {
        let stream = TcpStream::connect(addr).await?;
        Ok(Self {
            stream,
            init_sent: false,
        })
    }

    /// Wrap an existing stream (the init tag will be sent on first [`send`]).
    pub fn from_stream(stream: TcpStream) -> Self {
        Self {
            stream,
            init_sent: false,
        }
    }

    /// Send a message with Padded Intermediate framing.
    ///
    /// Frame layout: `[total_len: u32 LE][data][random_pad: 0–15 bytes]`
    /// where `total_len = data.len() + pad_len`.
    pub async fn send(&mut self, data: &[u8]) -> Result<(), InvocationError> {
        if !self.init_sent {
            self.stream.write_all(&[0xdd, 0xdd, 0xdd, 0xdd]).await?;
            self.init_sent = true;
        }
        let mut pad_len_buf = [0u8; 1];
        getrandom::getrandom(&mut pad_len_buf)
            .map_err(|_| InvocationError::Deserialize("getrandom failed".into()))?;
        let pad_len = (pad_len_buf[0] & 0x0f) as usize;
        let total_len = (data.len() + pad_len) as u32;
        self.stream.write_all(&total_len.to_le_bytes()).await?;
        self.stream.write_all(data).await?;
        if pad_len > 0 {
            let mut pad = vec![0u8; pad_len];
            getrandom::getrandom(&mut pad)
                .map_err(|_| InvocationError::Deserialize("getrandom failed".into()))?;
            self.stream.write_all(&pad).await?;
        }
        Ok(())
    }

    /// Receive the next Padded Intermediate message, stripping the random padding.
    pub async fn recv(&mut self) -> Result<Vec<u8>, InvocationError> {
        let mut len_buf = [0u8; 4];
        self.stream.read_exact(&mut len_buf).await?;
        let raw = i32::from_le_bytes(len_buf);
        if raw < 0 {
            return Err(InvocationError::Io(std::io::Error::new(
                std::io::ErrorKind::ConnectionRefused,
                format!("transport error: {raw}"),
            )));
        }
        let total_len = raw as usize;
        let mut buf = vec![0u8; total_len];
        self.stream.read_exact(&mut buf).await?;
        // Strip up to 15 bytes of random padding.
        // The MTProto payload is at minimum 24 bytes (32-byte minimum decrypted frame).
        if buf.len() >= 24 {
            let pad = (buf.len() - 24) % 16;
            buf.truncate(buf.len() - pad);
        }
        Ok(buf)
    }

    pub fn into_inner(self) -> TcpStream {
        self.stream
    }
}

// Full

/// [MTProto Full] transport framing.
///
/// Extends Intermediate with:
/// * 4-byte little-endian **sequence number** (auto-incremented per message).
/// * 4-byte **CRC-32** at the end of each packet covering
///   `[len][seq_no][payload]`.
///
/// No init byte is sent; the full format is detected by the absence of
/// `0xef` / `0xee` in the first byte.
///
/// [MTProto Full]: https://core.telegram.org/mtproto/mtproto-transports#full
pub struct FullTransport {
    stream: TcpStream,
    send_seqno: u32,
    recv_seqno: u32,
}

impl FullTransport {
    pub async fn connect(addr: &str) -> Result<Self, InvocationError> {
        let stream = TcpStream::connect(addr).await?;
        Ok(Self {
            stream,
            send_seqno: 0,
            recv_seqno: 0,
        })
    }

    pub fn from_stream(stream: TcpStream) -> Self {
        Self {
            stream,
            send_seqno: 0,
            recv_seqno: 0,
        }
    }

    /// Send a message with Full framing (length + seqno + payload + crc32).
    pub async fn send(&mut self, data: &[u8]) -> Result<(), InvocationError> {
        let total_len = (data.len() + 12) as u32; // len field + seqno + payload + crc
        let seq = self.send_seqno;
        self.send_seqno = self.send_seqno.wrapping_add(1);

        let mut packet = Vec::with_capacity(total_len as usize);
        packet.extend_from_slice(&total_len.to_le_bytes());
        packet.extend_from_slice(&seq.to_le_bytes());
        packet.extend_from_slice(data);

        let crc = crc32_ieee(&packet);
        packet.extend_from_slice(&crc.to_le_bytes());

        self.stream.write_all(&packet).await?;
        Ok(())
    }

    /// Receive the next Full-framed message; validates the CRC-32.
    pub async fn recv(&mut self) -> Result<Vec<u8>, InvocationError> {
        let mut len_buf = [0u8; 4];
        self.stream.read_exact(&mut len_buf).await?;
        // Negative value = transport-level error code from Telegram.
        let raw = i32::from_le_bytes(len_buf);
        if raw < 0 {
            return Err(InvocationError::Io(std::io::Error::new(
                std::io::ErrorKind::ConnectionRefused,
                format!("transport error: {raw}"),
            )));
        }
        let total_len = raw as usize;
        if total_len < 12 {
            return Err(InvocationError::Deserialize(
                "Full transport: packet too short".into(),
            ));
        }
        let mut rest = vec![0u8; total_len - 4];
        self.stream.read_exact(&mut rest).await?;

        // Verify CRC
        let (body, crc_bytes) = rest.split_at(rest.len() - 4);
        let expected_crc = u32::from_le_bytes(crc_bytes.try_into().unwrap());
        let mut check_input = len_buf.to_vec();
        check_input.extend_from_slice(body);
        let actual_crc = crc32_ieee(&check_input);
        if actual_crc != expected_crc {
            return Err(InvocationError::Deserialize(format!(
                "Full transport: CRC mismatch (got {actual_crc:#010x}, expected {expected_crc:#010x})"
            )));
        }

        // seq_no is the first 4 bytes of `body`
        let recv_seq = u32::from_le_bytes(body[..4].try_into().unwrap());
        if recv_seq != self.recv_seqno {
            return Err(InvocationError::Deserialize(format!(
                "Full transport: seq_no mismatch (got {recv_seq}, expected {})",
                self.recv_seqno
            )));
        }
        self.recv_seqno = self.recv_seqno.wrapping_add(1);

        Ok(body[4..].to_vec())
    }

    pub fn into_inner(self) -> TcpStream {
        self.stream
    }
}

// CRC-32 (IEEE 802.3 polynomial)

/// Compute CRC-32 using the standard IEEE 802.3 polynomial.
pub(crate) fn crc32_ieee(data: &[u8]) -> u32 {
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
