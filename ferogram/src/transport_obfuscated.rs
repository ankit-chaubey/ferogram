// Copyright (c) Ankit Chaubey <ankitchaubey.dev@gmail.com>
// SPDX-License-Identifier: MIT OR Apache-2.0
//
// ferogram: async Telegram MTProto client in Rust
// https://github.com/ankit-chaubey/ferogram
//
//
// If you use or modify this code, keep this notice at the top of your file
// and include the LICENSE-MIT or LICENSE-APACHE file from this repository:
// https://github.com/ankit-chaubey/ferogram

//! Obfuscated MTProto transport (Obfuscated2).

pub use ferogram_crypto::ObfuscatedCipher;

use crate::InvocationError;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

pub struct ObfuscatedStream {
    stream: TcpStream,
    cipher: ObfuscatedCipher,
}

impl ObfuscatedStream {
    pub async fn connect(
        addr: &str,
        proxy_secret: Option<&[u8; 16]>,
        dc_id: i16,
    ) -> Result<Self, InvocationError> {
        let stream = TcpStream::connect(addr).await?;
        Self::handshake(stream, proxy_secret, dc_id).await
    }

    async fn handshake(
        mut stream: TcpStream,
        proxy_secret: Option<&[u8; 16]>,
        dc_id: i16,
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

        nonce[56] = 0xef;
        nonce[57] = 0xef;
        nonce[58] = 0xef;
        nonce[59] = 0xef;
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
        Ok(Self { stream, cipher })
    }

    pub async fn send(&mut self, data: &[u8]) -> Result<(), InvocationError> {
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
        Ok(())
    }

    pub async fn recv(&mut self) -> Result<Vec<u8>, InvocationError> {
        let mut h = [0u8; 1];
        self.stream.read_exact(&mut h).await?;
        self.cipher.decrypt(&mut h);

        // h[0] > 0x7f: first byte of a 4-byte transport error code (negative i32).
        let words = if h[0] < 0x7f {
            h[0] as usize
        } else if h[0] == 0x7f {
            let mut b = [0u8; 3];
            self.stream.read_exact(&mut b).await?;
            self.cipher.decrypt(&mut b);
            b[0] as usize | (b[1] as usize) << 8 | (b[2] as usize) << 16
        } else {
            // h[0] > 0x7f: first byte of a 4-byte transport error.
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
        Ok(buf)
    }
}
