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

use std::{fmt, io};

/// Errors produced by [`Connection`](crate::Connection) and transport helpers.
#[derive(Debug)]
pub enum ConnectError {
    /// Network / I/O failure.
    Io(io::Error),
    /// Protocol violation or decoding failure.
    Other(String),
    /// Telegram transport-level error code (negative 4-byte word).
    TransportCode(i32),
    /// RPC error returned by Telegram (code + message string).
    Rpc { code: i32, message: String },
}

impl ConnectError {
    pub fn other(msg: impl Into<String>) -> Self {
        Self::Other(msg.into())
    }
}

impl fmt::Display for ConnectError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(e) => write!(f, "I/O error: {e}"),
            Self::Other(s) => write!(f, "connect error: {s}"),
            Self::TransportCode(c) => write!(f, "Telegram transport error: {c}"),
            Self::Rpc { code, message } => write!(f, "RPC {code}: {message}"),
        }
    }
}

impl std::error::Error for ConnectError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(e) => Some(e),
            _ => None,
        }
    }
}

impl From<io::Error> for ConnectError {
    fn from(e: io::Error) -> Self {
        Self::Io(e)
    }
}

impl From<ferogram_tl_types::deserialize::Error> for ConnectError {
    fn from(e: ferogram_tl_types::deserialize::Error) -> Self {
        Self::Other(format!("TL deserialize error: {e:?}"))
    }
}
