// Copyright (c) Ankit Chaubey <ankitchaubey.dev@gmail.com>
// SPDX-License-Identifier: MIT OR Apache-2.0
//
// ferogram: async Telegram MTProto client in Rust
// https://github.com/ankit-chaubey/ferogram
//
// If you use or modify this code, keep this notice at the top of your file
// and include the LICENSE-MIT or LICENSE-APACHE file from this repository:
// https://github.com/ankit-chaubey/ferogram

pub use ferogram_crypto::ObfuscatedCipher;

use crate::InvocationError;
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
    ) -> Result<Self, InvocationError> {
        let stream = TcpStream::connect(addr).await?;
        Self::handshake(stream, proxy_secret, dc_id, ObfuscatedFraming::Abridged).await
    }

    /// Connect using Padded Intermediate framing (0xDD MTProxy secret).
    pub async fn connect_padded(
        addr: &str,
        proxy_secret: Option<&[u8; 16]>,
        dc_id: i16,
    ) -> Result<Self, InvocationError> {
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
    ) -> Result<Self, InvocationError> {
        use sha2::Digest;

        let mut nonce = [0u8; 64];
        loop {
            getrandom::getrandom(&mut nonce)
                .map_err(|_| InvocationError::Deserialize("getrandom failed".into()))?;
            let first = u32::from_le_bytes(nonce[0..4].try_into().unwrap());
            let second = u32::from_le_bytes(nonce[4..8].try_into().unwrap());
            let bad = nonce[0] == 0xEF
                || first == 0x44414548
                || first == 0x54534F50
                || first == 0x20544547
                || first == 0x4954504f
                || first == 0xEEEEEEEE
                || first == 0xDDDDDDDD
                || first == 0x02010316
                || second == 0x00000000;
            if !bad {
                break;
            }
        }

        let tx_raw: [u8; 32] = nonce[8..40].try_into().unwrap();
        let tx_iv: [u8; 16] = nonce[40..56].try_into().unwrap();
        let mut rev48 = nonce[8..56].to_vec();
        rev48.reverse();
        let rx_raw: [u8; 32] = rev48[0..32].try_into().unwrap();
        let rx_iv: [u8; 16] = rev48[32..48].try_into().unwrap();

        let (tx_key, rx_key): ([u8; 32], [u8; 32]) = if let Some(s) = proxy_secret {
            let mut h = sha2::Sha256::new();
            h.update(tx_raw);
            h.update(s.as_ref());
            let tx: [u8; 32] = h.finalize().into();
            let mut h = sha2::Sha256::new();
            h.update(rx_raw);
            h.update(s.as_ref());
            let rx: [u8; 32] = h.finalize().into();
            (tx, rx)
        } else {
            (tx_raw, rx_raw)
        };

        // Stamp the correct protocol tag at nonce[56..60].
        // Abridged  -> 0xEFEFEFEF
        // PaddedIntermediate -> 0xDDDDDDDD (required for 0xDD MTProxy secrets)
        match framing {
            ObfuscatedFraming::Abridged => {
                nonce[56] = 0xef;
                nonce[57] = 0xef;
                nonce[58] = 0xef;
                nonce[59] = 0xef;
            }
            ObfuscatedFraming::PaddedIntermediate => {
                nonce[56] = 0xdd;
                nonce[57] = 0xdd;
                nonce[58] = 0xdd;
                nonce[59] = 0xdd;
            }
        }
        let dc_bytes = dc_id.to_le_bytes();
        nonce[60] = dc_bytes[0];
        nonce[61] = dc_bytes[1];

        // Single continuous cipher: advance TX past plaintext nonce[0..56], then
        // encrypt nonce[56..64].  The same instance is stored for all later TX so
        // the AES-CTR stream continues from position 64.
        let mut cipher = ObfuscatedCipher::from_keys(&tx_key, &tx_iv, &rx_key, &rx_iv);
        let mut skip = [0u8; 56];
        cipher.encrypt(&mut skip);
        cipher.encrypt(&mut nonce[56..64]);

        stream.write_all(&nonce).await?;
        Ok(Self {
            stream,
            cipher,
            framing,
        })
    }

    pub async fn send(&mut self, data: &[u8]) -> Result<(), InvocationError> {
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
                getrandom::getrandom(&mut pad_len_buf).ok();
                let pad_len = (pad_len_buf[0] & 0x0f) as usize;
                let total_payload = data.len() + pad_len;
                let mut frame = Vec::with_capacity(4 + total_payload);
                frame.extend_from_slice(&(total_payload as u32).to_le_bytes());
                frame.extend_from_slice(data);
                let mut pad = vec![0u8; pad_len];
                getrandom::getrandom(&mut pad).ok();
                frame.extend_from_slice(&pad);
                self.cipher.encrypt(&mut frame);
                self.stream.write_all(&frame).await?;
            }
        }
        Ok(())
    }

    pub async fn recv(&mut self) -> Result<Vec<u8>, InvocationError> {
        match self.framing {
            ObfuscatedFraming::Abridged => {
                let mut h = [0u8; 1];
                self.stream.read_exact(&mut h).await?;
                self.cipher.decrypt(&mut h);

                // Three cases for the first decrypted byte:
                //   < 0x7f  -> Abridged word count
                //   == 0x7f -> extended 3-byte word count
                //   > 0x7f  -> transport error: read 3 more bytes and form i32
                let words = if h[0] < 0x7f {
                    h[0] as usize
                } else if h[0] == 0x7f {
                    let mut b = [0u8; 3];
                    self.stream.read_exact(&mut b).await?;
                    self.cipher.decrypt(&mut b);
                    b[0] as usize | (b[1] as usize) << 8 | (b[2] as usize) << 16
                } else {
                    // First byte > 0x7f, transport error code.
                    // Read the remaining 3 bytes, form a signed i32, return error.
                    let mut rest = [0u8; 3];
                    self.stream.read_exact(&mut rest).await?;
                    self.cipher.decrypt(&mut rest);
                    let code = i32::from_le_bytes([h[0], rest[0], rest[1], rest[2]]);
                    return Err(InvocationError::Io(std::io::Error::new(
                        std::io::ErrorKind::ConnectionRefused,
                        format!("transport error from server: {code}"),
                    )));
                };

                let mut buf = vec![0u8; words * 4];
                self.stream.read_exact(&mut buf).await?;
                self.cipher.decrypt(&mut buf);

                // Secondary transport error check for the post-read payload.
                if buf.len() == 4 {
                    let code = i32::from_le_bytes(buf[..4].try_into().unwrap());
                    if code < 0 {
                        return Err(InvocationError::Io(std::io::Error::new(
                            std::io::ErrorKind::ConnectionRefused,
                            format!("transport error from server: {code}"),
                        )));
                    }
                }

                Ok(buf)
            }
            ObfuscatedFraming::PaddedIntermediate => {
                // Padded intermediate recv: read encrypted 4-byte LE
                // length prefix, then read and decrypt payload+padding, strip padding.
                let mut len_buf = [0u8; 4];
                self.stream.read_exact(&mut len_buf).await?;
                self.cipher.decrypt(&mut len_buf);
                let total_len = i32::from_le_bytes(len_buf);
                if total_len < 0 {
                    return Err(InvocationError::Io(std::io::Error::new(
                        std::io::ErrorKind::ConnectionRefused,
                        format!("transport error from server: {total_len}"),
                    )));
                }
                let mut buf = vec![0u8; total_len as usize];
                self.stream.read_exact(&mut buf).await?;
                self.cipher.decrypt(&mut buf);
                // Strip up to 15 bytes of random padding (payload is ≥24 bytes).
                if buf.len() >= 24 {
                    let pad = (buf.len() - 24) % 16;
                    buf.truncate(buf.len() - pad);
                }
                Ok(buf)
            }
        }
    }
}
