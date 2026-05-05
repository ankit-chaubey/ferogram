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

/// An error returned by Telegram's servers in response to an RPC call.
///
/// Numeric values are stripped from the name and placed in [`RpcError::value`].
///
/// # Example
/// `FLOOD_WAIT_30` → `RpcError { code: 420, name: "FLOOD_WAIT", value: Some(30) }`
#[derive(Clone, Debug, PartialEq)]
pub struct RpcError {
    /// HTTP-like status code.
    pub code: i32,
    /// Error name in SCREAMING_SNAKE_CASE with digits removed.
    pub name: String,
    /// Numeric suffix extracted from the name, if any.
    pub value: Option<u32>,
}

impl fmt::Display for RpcError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "RPC {}: {}", self.code, self.name)?;
        if let Some(v) = self.value {
            write!(f, " (value: {v})")?;
        }
        Ok(())
    }
}

impl std::error::Error for RpcError {}

impl RpcError {
    /// Parse a raw Telegram error message like `"FLOOD_WAIT_30"` into an `RpcError`.
    pub fn from_telegram(code: i32, message: &str) -> Self {
        if let Some(idx) = message.rfind('_') {
            let suffix = &message[idx + 1..];
            if !suffix.is_empty()
                && suffix.chars().all(|c| c.is_ascii_digit())
                && let Ok(v) = suffix.parse::<u32>()
            {
                let name = message[..idx].to_string();
                return Self {
                    code,
                    name,
                    value: Some(v),
                };
            }
        }
        Self {
            code,
            name: message.to_string(),
            value: None,
        }
    }

    /// Match on the error name, with optional wildcard prefix/suffix `'*'`.
    pub fn is(&self, pattern: &str) -> bool {
        if let Some(prefix) = pattern.strip_suffix('*') {
            self.name.starts_with(prefix)
        } else if let Some(suffix) = pattern.strip_prefix('*') {
            self.name.ends_with(suffix)
        } else {
            self.name == pattern
        }
    }

    /// Returns the flood-wait duration in seconds, if this is a FLOOD_WAIT error.
    pub fn flood_wait_seconds(&self) -> Option<u64> {
        if self.code == 420 && self.name == "FLOOD_WAIT" {
            self.value.map(|v| v as u64)
        } else {
            None
        }
    }

    /// If this is a DC-migration redirect (code 303), returns the target DC id.
    pub fn migrate_dc_id(&self) -> Option<i32> {
        if self.code != 303 {
            return None;
        }
        let is_migrate = self.name == "PHONE_MIGRATE"
            || self.name == "NETWORK_MIGRATE"
            || self.name == "FILE_MIGRATE"
            || self.name == "USER_MIGRATE"
            || self.name.ends_with("_MIGRATE");
        if is_migrate {
            Some(self.value.unwrap_or(2) as i32)
        } else {
            None
        }
    }
}

/// The error type returned from any `Client` method that talks to Telegram.
#[derive(Debug)]
#[non_exhaustive]
pub enum InvocationError {
    /// Telegram rejected the request.
    Rpc(RpcError),
    /// Network / I/O failure.
    Io(io::Error),
    /// Response deserialization failed.
    Deserialize(String),
    /// The request was dropped (e.g. sender task shut down).
    Dropped,
    /// DC migration required: handled internally by the client.
    #[doc(hidden)]
    Migrate(i32),
    /// The cached access hash was rejected by Telegram.
    StaleHash,
    /// No access hash is cached for this peer.
    PeerNotCached(String),
}

impl fmt::Display for InvocationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Rpc(e) => write!(f, "{e}"),
            Self::Io(e) => write!(f, "I/O error: {e}"),
            Self::Deserialize(s) => write!(f, "deserialize error: {s}"),
            Self::Dropped => write!(f, "request dropped"),
            Self::Migrate(dc) => write!(f, "DC migration to {dc}"),
            Self::StaleHash => write!(f, "stale access hash; peer cache cleared, retry"),
            Self::PeerNotCached(s) => write!(f, "peer not cached: {s}"),
        }
    }
}

impl std::error::Error for InvocationError {}

impl From<io::Error> for InvocationError {
    fn from(e: io::Error) -> Self {
        Self::Io(e)
    }
}

impl From<ferogram_tl_types::deserialize::Error> for InvocationError {
    fn from(e: ferogram_tl_types::deserialize::Error) -> Self {
        Self::Deserialize(e.to_string())
    }
}

impl From<ferogram_connect::ConnectError> for InvocationError {
    fn from(e: ferogram_connect::ConnectError) -> Self {
        use ferogram_connect::ConnectError;
        match e {
            ConnectError::Io(e) => Self::Io(e),
            ConnectError::Other(s) => Self::Deserialize(s),
            ConnectError::TransportCode(code) => {
                Self::Rpc(RpcError::from_telegram(code, "transport error"))
            }
            ConnectError::Rpc { code, message } => {
                Self::Rpc(RpcError::from_telegram(code, &message))
            }
        }
    }
}

impl InvocationError {
    /// Returns `true` if this is the named RPC error (supports `'*'` wildcards).
    pub fn is(&self, pattern: &str) -> bool {
        match self {
            Self::Rpc(e) => e.is(pattern),
            _ => false,
        }
    }

    /// If this is a FLOOD_WAIT error, returns how many seconds to wait.
    pub fn flood_wait_seconds(&self) -> Option<u64> {
        match self {
            Self::Rpc(e) => e.flood_wait_seconds(),
            _ => None,
        }
    }

    /// If this error is a DC-migration redirect, returns the target DC id.
    pub fn migrate_dc_id(&self) -> Option<i32> {
        match self {
            Self::Rpc(r) => r.migrate_dc_id(),
            _ => None,
        }
    }
}
