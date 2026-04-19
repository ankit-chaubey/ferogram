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

//! Error types for ferogram.
//!
//! Error types for invoke and I/O failures.

use std::{fmt, io};

// RpcError

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
        // Try to find a numeric suffix after the last underscore.
        // e.g. "FLOOD_WAIT_30" → name = "FLOOD_WAIT", value = Some(30)
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
    ///
    /// # Examples
    /// - `err.is("FLOOD_WAIT")`: exact match
    /// - `err.is("PHONE_CODE_*")`: starts-with match  
    /// - `err.is("*_INVALID")`: ends-with match
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
}

// InvocationError

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
    /// DC migration required: handled internally by [`crate::Client`].
    /// Not returned to callers; present only for internal routing.
    #[doc(hidden)]
    Migrate(i32),
    /// No access hash is cached for this peer.
    ///
    /// The peer has been seen before but its `access_hash` was never stored
    /// (e.g. it arrived as a *min* user with no message context, or as a
    /// channel you have not yet opened).
    ///
    /// **Fix:** resolve the peer first via `client.resolve_peer(id)` or ensure
    /// that at least one message from this peer flows through the update loop
    /// before using it as a target.
    ///
    /// Alternatively, enable [`crate::ExperimentalFeatures::allow_zero_hash`]
    /// to fall back to `access_hash = 0` (valid for bots only per the
    /// Telegram spec; causes `USER_ID_INVALID` / `CHANNEL_INVALID` on user
    /// accounts).
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
}

// SignInError

/// Errors returned by [`crate::Client::sign_in`].
#[derive(Debug)]
pub enum SignInError {
    /// The phone number is new: must sign up via the official Telegram app first.
    SignUpRequired,
    /// 2FA is enabled; the contained token must be passed to [`crate::Client::check_password`].
    PasswordRequired(Box<PasswordToken>),
    /// The code entered was wrong or has expired.
    InvalidCode,
    /// Any other error.
    Other(InvocationError),
}

impl fmt::Display for SignInError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SignUpRequired => write!(f, "sign up required: use official Telegram app"),
            Self::PasswordRequired(_) => write!(f, "2FA password required"),
            Self::InvalidCode => write!(f, "invalid or expired code"),
            Self::Other(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for SignInError {}

impl From<InvocationError> for SignInError {
    fn from(e: InvocationError) -> Self {
        Self::Other(e)
    }
}

// PasswordToken

/// Opaque 2FA challenge token returned in [`SignInError::PasswordRequired`].
///
/// Pass to [`crate::Client::check_password`] together with the user's password.
pub struct PasswordToken {
    pub(crate) password: ferogram_tl_types::types::account::Password,
}

impl PasswordToken {
    /// The password hint set by the account owner, if any.
    pub fn hint(&self) -> Option<&str> {
        self.password.hint.as_deref()
    }
}

impl fmt::Debug for PasswordToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "PasswordToken {{ hint: {:?} }}", self.hint())
    }
}

// LoginToken

/// Opaque token returned by [`crate::Client::request_login_code`].
///
/// Pass to [`crate::Client::sign_in`] with the received code.
pub struct LoginToken {
    pub(crate) phone: String,
    pub(crate) phone_code_hash: String,
}
