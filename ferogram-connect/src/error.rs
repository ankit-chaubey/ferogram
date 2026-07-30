/*
 * Copyright (c) 2026 Ankit Chaubey <ankitchaubey.dev@gmail.com>
 * https://github.com/ankit-chaubey
 *
 * Project: ferogram
 * Website: https://ferogram.dev
 *
 * Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
 * https://www.apache.org/licenses/LICENSE-2.0> or the MIT license
 * <LICENSE-MIT or https://opensource.org/licenses/MIT>, at your option.
 * This file may not be copied, modified, or distributed except according
 * to those terms.
 */

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
    /// Build the `Other` variant from any string-like value.
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
