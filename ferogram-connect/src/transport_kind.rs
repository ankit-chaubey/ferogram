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

/// Which MTProto wire framing to use for a connection.
///
/// | Variant | Init bytes | Notes |
/// |---------|-----------|-------|
/// | `Abridged` | `0xEF` | Lightest framing |
/// | `Intermediate` | `0xEEEEEEEE` | 4-byte length prefix |
/// | `Full` | none | length + seqno + CRC32 |
/// | `Obfuscated` | random 64B | Bypasses DPI / MTProxy: **default** |
/// | `PaddedIntermediate` | random 64B (`0xDDDDDDDD` tag) | Required for `0xDD` MTProxy secrets |
/// | `FakeTls` | TLS 1.3 ClientHello | Most DPI-resistant; required for `0xEE` MTProxy secrets |
#[derive(Clone, Debug)]
pub enum TransportKind {
    /// MTProto [Abridged] transport: length prefix is 1 or 4 bytes.
    Abridged,
    /// MTProto [Intermediate] transport: 4-byte LE length prefix.
    Intermediate,
    /// MTProto [Full] transport: 4-byte length + seqno + CRC32.
    Full,
    /// [Obfuscated2] transport: AES-256-CTR over Abridged framing.
    /// Required for MTProxy and networks with deep-packet inspection.
    /// **Default**: works on all networks, bypasses DPI, negligible CPU cost.
    ///
    /// `secret` is the 16-byte MTProxy secret, or `None` for keyless obfuscation.
    Obfuscated { secret: Option<[u8; 16]> },
    /// Obfuscated PaddedIntermediate transport (`0xDDDDDDDD` tag in nonce).
    ///
    /// Same AES-256-CTR obfuscation as `Obfuscated`, but uses Intermediate
    /// framing and appends 0-15 random padding bytes to each frame.
    /// Required for `0xDD` MTProxy secrets.
    PaddedIntermediate { secret: Option<[u8; 16]> },
    /// FakeTLS transport (`0xEE` prefix in MTProxy secret).
    ///
    /// Wraps all MTProto data in fake TLS 1.3 records.
    FakeTls { secret: [u8; 16], domain: String },
    /// HTTP transport fallback: sends raw MTProto frames as HTTP POST to port 80.
    Http,
}

impl Default for TransportKind {
    fn default() -> Self {
        TransportKind::Obfuscated { secret: None }
    }
}
