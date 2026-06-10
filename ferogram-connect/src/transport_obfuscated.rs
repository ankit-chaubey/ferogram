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

pub use ferogram_crypto::ObfuscatedCipher;

use crate::ConnectError;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

/// Framing mode for `ObfuscatedStream`.
///
/// * `Abridged` - Obfuscated2 over Abridged framing (`0xEFEFEFEF` nonce tag).
///   Used for plain and `0x??` MTProxy secrets.
/// * `PaddedIntermediate` - Obfuscated2 over Padded Intermediate framing
///   (`0xDDDDDDDD` nonce tag).  Required for `0xDD` MTProxy secrets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ObfuscatedFraming {
    Abridged,
    PaddedIntermediate,
}

pub struct ObfuscatedStream {
    stream: TcpStream,
    cipher: ObfuscatedCipher,
    framing: ObfuscatedFraming,
}

impl ObfuscatedStream {
    /// Connect using Abridged framing (plain MTProxy secret, no 0xDD prefix).
    pub async fn connect(
        addr: &str,
        proxy_secret: Option<&[u8; 16]>,
        dc_id: i16,
    ) -> Result<Self, ConnectError> {
        let stream = TcpStream::connect(addr).await?;
        Self::handshake(stream, proxy_secret, dc_id, ObfuscatedFraming::Abridged).await
    }

    /// Connect using Padded Intermediate framing (0xDD MTProxy secret).
    pub async fn connect_padded(
        addr: &str,
        proxy_secret: Option<&[u8; 16]>,
        dc_id: i16,
    ) -> Result<Self, ConnectError> {
        let stream = TcpStream::connect(addr).await?;
        Self::handshake(
            stream,
            proxy_secret,
            dc_id,
            ObfuscatedFraming::PaddedIntermediate,
        )
        .await
    }

    async fn handshake(
        mut stream: TcpStream,
        proxy_secret: Option<&[u8; 16]>,
        dc_id: i16,
        framing: ObfuscatedFraming,
    ) -> Result<Self, ConnectError> {
        let framing_byte = match framing {
            ObfuscatedFraming::Abridged => 0xef,
            ObfuscatedFraming::PaddedIntermediate => 0xdd,
        };
        let secret = proxy_secret.map(|s| s.as_ref());
        let (nonce, cipher) = ferogram_crypto::build_obfuscated_init(framing_byte, dc_id, secret);
        stream.write_all(&nonce).await?;
        Ok(Self {
            stream,
            cipher,
            framing,
        })
    }

    pub async fn send(&mut self, data: &[u8]) -> Result<(), ConnectError> {
        match self.framing {
            ObfuscatedFraming::Abridged => {
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
                self.cipher.encrypt(&mut frame);
                self.stream.write_all(&frame).await?;
            }
            ObfuscatedFraming::PaddedIntermediate => {
                // Padded intermediate framing: 4-byte LE length of
                // (payload + random 0-15 padding), then payload, then padding.
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
                self.cipher.encrypt(&mut frame);
                self.stream.write_all(&frame).await?;
            }
        }
        Ok(())
    }

    pub async fn recv(&mut self) -> Result<Vec<u8>, ConnectError> {
        match self.framing {
            ObfuscatedFraming::Abridged => {
                let mut h = [0u8; 1];
                self.stream.read_exact(&mut h).await?;
                self.cipher.decrypt(&mut h);

                let words = if h[0] < 0x7f {
                    h[0] as usize
                } else if h[0] == 0x7f {
                    let mut b = [0u8; 3];
                    self.stream.read_exact(&mut b).await?;
                    self.cipher.decrypt(&mut b);
                    b[0] as usize | (b[1] as usize) << 8 | (b[2] as usize) << 16
                } else {
                    let mut rest = [0u8; 3];
                    self.stream.read_exact(&mut rest).await?;
                    self.cipher.decrypt(&mut rest);
                    let code = i32::from_le_bytes([h[0], rest[0], rest[1], rest[2]]);
                    return Err(ConnectError::Io(std::io::Error::new(
                        std::io::ErrorKind::ConnectionRefused,
                        format!("transport error from server: {code}"),
                    )));
                };

                let mut buf = vec![0u8; words * 4];
                self.stream.read_exact(&mut buf).await?;
                self.cipher.decrypt(&mut buf);

                if buf.len() == 4 {
                    let code = i32::from_le_bytes(buf[..4].try_into().unwrap());
                    if code < 0 {
                        return Err(ConnectError::Io(std::io::Error::new(
                            std::io::ErrorKind::ConnectionRefused,
                            format!("transport error from server: {code}"),
                        )));
                    }
                }

                Ok(buf)
            }
            ObfuscatedFraming::PaddedIntermediate => {
                let mut len_buf = [0u8; 4];
                self.stream.read_exact(&mut len_buf).await?;
                self.cipher.decrypt(&mut len_buf);
                let total_len = i32::from_le_bytes(len_buf);
                if total_len < 0 {
                    return Err(ConnectError::Io(std::io::Error::new(
                        std::io::ErrorKind::ConnectionRefused,
                        format!("transport error from server: {total_len}"),
                    )));
                }
                let mut buf = vec![0u8; total_len as usize];
                self.stream.read_exact(&mut buf).await?;
                self.cipher.decrypt(&mut buf);
                if buf.len() >= 24 {
                    let pad = (buf.len() - 24) % 16;
                    buf.truncate(buf.len() - pad);
                }
                Ok(buf)
            }
        }
    }
}
