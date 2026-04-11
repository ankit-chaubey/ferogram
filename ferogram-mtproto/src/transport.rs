// Copyright (c) Ankit Chaubey <ankitchaubey.dev@gmail.com>
// SPDX-License-Identifier: MIT OR Apache-2.0
//
// ferogram: async Telegram MTProto client in Rust
// https://github.com/ankit-chaubey/ferogram
//
// Based on layer: https://github.com/ankit-chaubey/layer
// Follows official Telegram client behaviour (tdesktop, TDLib).
//
// If you use or modify this code, keep this notice at the top of your file
// and include the LICENSE-MIT or LICENSE-APACHE file from this repository:
// https://github.com/ankit-chaubey/ferogram

//! Pluggable transport layer.
//!
//! Implement [`Transport`] over TCP, WebSocket, or any other byte-stream
//! protocol to get MTProto message framing for free.
//!
//! # Transport types
//!
//! | Type | Use when |
//! |------|----------|
//! | [`AbridgedTransport`] | Direct connection, no ISP blocks |
//! | [`ObfuscatedAbridged`] | ISP DPI blocking plain Telegram traffic |

use std::io::{Read, Write};
use std::net::TcpStream;

use ferogram_crypto::ObfuscatedCipher;

// Core trait

/// A full-duplex byte-stream transport.
pub trait Transport {
    /// The error type returned by [`send`](Transport::send) and [`recv`](Transport::recv).
    type Error: std::error::Error + Send + Sync + 'static;
    /// Send raw bytes over the transport.
    fn send(&mut self, data: &[u8]) -> Result<(), Self::Error>;
    /// Receive raw bytes from the transport.
    fn recv(&mut self) -> Result<Vec<u8>, Self::Error>;
}

/// Transport that exposes the 4-byte tag needed by [`ObfuscatedAbridged`].
pub trait Tagged {
    /// Return the 4-byte init tag embedded into the obfuscated header.
    fn init_tag(&mut self) -> [u8; 4];
}

// Abridged framing

/// Wraps a `Transport` and applies MTProto Abridged framing.
/// Use [`ObfuscatedAbridged`] instead if your ISP blocks plain Telegram.
pub struct AbridgedTransport<T: Transport> {
    inner: T,
    init_sent: bool,
}

impl<T: Transport> AbridgedTransport<T> {
    /// Create a new [`AbridgedTransport`] wrapping `inner`.
    pub fn new(inner: T) -> Self {
        Self {
            inner,
            init_sent: false,
        }
    }

    /// Send one MTProto message with Abridged length-prefix framing.
    pub fn send_message(&mut self, data: &[u8]) -> Result<(), T::Error> {
        if !self.init_sent {
            self.inner.send(&[0xef])?;
            self.init_sent = true;
        }
        let len = data.len() / 4;
        let header: Vec<u8> = if len < 127 {
            vec![len as u8]
        } else {
            vec![
                0x7f,
                (len & 0xff) as u8,
                ((len >> 8) & 0xff) as u8,
                ((len >> 16) & 0xff) as u8,
            ]
        };
        self.inner.send(&header)?;
        self.inner.send(data)
    }

    /// Receive one MTProto message from the underlying transport.
    pub fn recv_message(&mut self) -> Result<Vec<u8>, T::Error> {
        self.inner.recv()
    }

    /// Return a mutable reference to the inner transport.
    pub fn inner_mut(&mut self) -> &mut T {
        &mut self.inner
    }
}

impl<T: Transport> Tagged for AbridgedTransport<T> {
    fn init_tag(&mut self) -> [u8; 4] {
        self.init_sent = true; // suppress plain 0xef; obfuscation header carries it
        [0xef, 0xef, 0xef, 0xef]
    }
}

// Obfuscated + Abridged

const FORBIDDEN: &[[u8; 4]] = &[
    [b'H', b'E', b'A', b'D'],
    [b'P', b'O', b'S', b'T'],
    [b'G', b'E', b'T', b' '],
    [b'O', b'P', b'T', b'I'],
    [0x16, 0x03, 0x01, 0x02],
    [0xdd, 0xdd, 0xdd, 0xdd],
    [0xee, 0xee, 0xee, 0xee],
];

/// Obfuscated + Abridged framing over a raw `TcpStream`.
///
/// **Use this in production** to bypass ISP Deep Packet Inspection.
/// Drop-in for `AbridgedTransport`: same `send_message` / `recv_message` API.
///
/// ```rust,no_run
/// let stream = TcpStream::connect("149.154.167.51:443")?;
/// stream.set_read_timeout(Some(Duration::from_secs(15)))?;
/// stream.set_write_timeout(Some(Duration::from_secs(15)))?;
/// let mut transport = ObfuscatedAbridged::new(stream)?;
/// transport.send_message(&payload)?;
/// let response = transport.recv_message()?;
/// ```
pub struct ObfuscatedAbridged {
    stream: TcpStream,
    cipher: ObfuscatedCipher,
    header: Option<[u8; 64]>,
}

impl ObfuscatedAbridged {
    /// Create a new [`ObfuscatedAbridged`] transport, performing the obfuscation
    /// handshake and sending the 64-byte init header to the server.
    pub fn new(stream: TcpStream) -> std::io::Result<Self> {
        let mut init = [0u8; 64];
        loop {
            getrandom::getrandom(&mut init).expect("getrandom");
            if init[0] == 0xef {
                continue;
            }
            if init[4..8] == [0u8; 4] {
                continue;
            }
            if FORBIDDEN.iter().any(|f| f == &init[..4]) {
                continue;
            }
            break;
        }
        // Embed Abridged tag at bytes 56-59.
        init[56..60].copy_from_slice(&[0xef, 0xef, 0xef, 0xef]);
        // Build cipher, then encrypt bytes 56-63 in the header.
        let mut cipher = ObfuscatedCipher::new(&init);
        let mut enc = init.to_vec();
        cipher.encrypt(&mut enc);
        init[56..64].copy_from_slice(&enc[56..64]);
        Ok(Self {
            stream,
            cipher,
            header: Some(init),
        })
    }

    /// Send one MTProto message, encrypting with the obfuscated cipher and
    /// prepending the Abridged length prefix.
    pub fn send_message(&mut self, data: &[u8]) -> std::io::Result<()> {
        if let Some(hdr) = self.header.take() {
            self.stream.write_all(&hdr)?;
        }
        let len = data.len() / 4;
        let mut frame: Vec<u8> = if len < 127 {
            vec![len as u8]
        } else {
            vec![
                0x7f,
                (len & 0xff) as u8,
                ((len >> 8) & 0xff) as u8,
                ((len >> 16) & 0xff) as u8,
            ]
        };
        frame.extend_from_slice(data);
        self.cipher.encrypt(&mut frame);
        self.stream.write_all(&frame)
    }

    /// Receive and decrypt one MTProto message from the obfuscated stream.
    pub fn recv_message(&mut self) -> std::io::Result<Vec<u8>> {
        let mut first = [0u8; 1];
        self.stream.read_exact(&mut first)?;
        self.cipher.decrypt(&mut first);
        let words = if first[0] < 0x7f {
            first[0] as usize
        } else {
            let mut rest = [0u8; 3];
            self.stream.read_exact(&mut rest)?;
            self.cipher.decrypt(&mut rest);
            rest[0] as usize | (rest[1] as usize) << 8 | (rest[2] as usize) << 16
        };
        let mut payload = vec![0u8; words * 4];
        self.stream.read_exact(&mut payload)?;
        self.cipher.decrypt(&mut payload);
        Ok(payload)
    }
}
