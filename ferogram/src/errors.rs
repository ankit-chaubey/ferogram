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

use std::fmt;

// Re-export from ferogram-mtsender, single source of truth.
pub use ferogram_mtsender::{InvocationError, RpcError};

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
