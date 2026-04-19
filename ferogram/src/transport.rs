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

//! Async TCP transport for MTProto (abridged framing).
//!
//! Handles the low-level abridged transport protocol over tokio's async TCP.

use std::io;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

/// Async abridged MTProto transport.
#[allow(dead_code)]
pub struct AsyncAbridged {
    stream: TcpStream,
    /// Whether the 0xef init byte has been sent.
    init_sent: bool,
}

#[allow(dead_code)]
impl AsyncAbridged {
    pub async fn connect(addr: &str) -> io::Result<Self> {
        let stream = TcpStream::connect(addr).await?;
        Ok(Self {
            stream,
            init_sent: false,
        })
    }

    pub async fn send(&mut self, data: &[u8]) -> io::Result<()> {
        if !self.init_sent {
            self.stream.write_all(&[0xef]).await?;
            self.init_sent = true;
        }
        let words = data.len() / 4;
        if words < 0x7f {
            self.stream.write_all(&[words as u8]).await?;
        } else {
            let b0 = 0x7f_u8;
            let b1 = (words & 0xff) as u8;
            let b2 = ((words >> 8) & 0xff) as u8;
            let b3 = ((words >> 16) & 0xff) as u8;
            self.stream.write_all(&[b0, b1, b2, b3]).await?;
        }
        self.stream.write_all(data).await
    }

    pub async fn recv(&mut self) -> io::Result<Vec<u8>> {
        let mut h = [0u8; 1];
        self.stream.read_exact(&mut h).await?;
        let words = if h[0] < 0x7f {
            h[0] as usize
        } else {
            let mut b = [0u8; 3];
            self.stream.read_exact(&mut b).await?;
            b[0] as usize | (b[1] as usize) << 8 | (b[2] as usize) << 16
        };
        let mut buf = vec![0u8; words * 4];
        self.stream.read_exact(&mut buf).await?;
        Ok(buf)
    }

    pub fn into_split(
        self,
    ) -> (
        tokio::net::tcp::OwnedReadHalf,
        tokio::net::tcp::OwnedWriteHalf,
    ) {
        self.stream.into_split()
    }
}
