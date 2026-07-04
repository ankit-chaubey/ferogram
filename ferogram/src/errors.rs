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

impl From<SignInError> for InvocationError {
    fn from(e: SignInError) -> Self {
        match e {
            SignInError::Other(err) => err,
            SignInError::InvalidCode => InvocationError::Rpc(RpcError {
                code: 400,
                name: "PHONE_CODE_INVALID".into(),
                value: None,
            }),
            SignInError::SignUpRequired => InvocationError::Rpc(RpcError {
                code: 400,
                name: "PHONE_NUMBER_UNOCCUPIED".into(),
                value: None,
            }),
            SignInError::PasswordRequired(_) => InvocationError::Rpc(RpcError {
                code: 401,
                name: "SESSION_PASSWORD_NEEDED".into(),
                value: None,
            }),
        }
    }
}

// PasswordToken

/// 2FA challenge token returned in [`SignInError::PasswordRequired`].
///
/// Pass to [`crate::Client::check_password`] together with the user's password.
#[derive(Clone)]
pub struct PasswordToken {
    pub password: ferogram_tl_types::types::account::Password,
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

/// Token returned by [`crate::Client::request_login_code`].
///
/// Pass to [`crate::Client::sign_in`] with the received code.
///
/// A stateless server can store `phone_code_hash` externally between requests
/// and reconstruct the token later using the phone number it already has.
#[derive(Clone, Debug)]
pub struct LoginToken {
    pub phone: String,
    pub phone_code_hash: String,
}

// SendCodeOptions

/// Settings forwarded to Telegram's `auth.sendCode` `code_settings` field.
///
/// Construct with [`SendCodeOptions::default`] for standard SMS/app delivery,
/// or fill specific fields to opt into alternative delivery methods or to
/// attach logout tokens for fast re-authentication.
///
/// # Fast re-authentication via logout tokens
///
/// When a user logs out through [`crate::Client::sign_out`] while having
/// logged in before, Telegram can issue a `future_auth_token` inside the
/// `auth.loggedOut` constructor. `sign_out()` captures this automatically
/// and [`crate::Client::request_login_code`] replays it on the next login,
/// so [`SendCodeOutcome::AlreadyAuthorized`] can happen with no extra work.
///
/// Set [`SendCodeOptions::logout_tokens`] yourself only to override that, for
/// example to supply a token obtained outside this client.
#[derive(Clone, Default)]
pub struct SendCodeOptions {
    /// Allow delivery via flash call (the caller ID carries the code digits).
    pub allow_flashcall: bool,
    /// Hint that this is the current number registered on the device.
    pub current_number: bool,
    /// Allow the app-signature hash to be sent (lets the OS auto-fill the
    /// code on some platforms without reading SMS content).
    pub allow_app_hash: bool,
    /// Allow delivery via missed call (last N digits of the caller ID).
    pub allow_missed_call: bool,
    /// Allow Firebase-backed SMS delivery.
    pub allow_firebase: bool,
    /// Hint that the phone number is not yet known to belong to this device.
    pub unknown_number: bool,
    /// Future auth tokens obtained from a previous `auth.loggedOut` response.
    ///
    /// Supply these to allow Telegram to short-circuit the login flow and
    /// return [`SendCodeOutcome::AlreadyAuthorized`] when the account is
    /// recognized.
    pub logout_tokens: Option<Vec<Vec<u8>>>,
    /// Firebase attestation/integrity token, required on some platforms when
    /// [`SendCodeOptions::allow_firebase`] is set.
    pub token: Option<String>,
    /// Whether `token` was issued by a Firebase sandbox (testing) environment
    /// rather than production.
    pub app_sandbox: Option<bool>,
}

// SendCodeOutcome

/// Result of [`crate::Client::request_login_code`].
///
/// Match on this to determine whether a verification code is needed or
/// whether Telegram already authorized the session via a logout token.
#[derive(Debug)]
pub enum SendCodeOutcome {
    /// Telegram sent a code; pass the contained [`LoginToken`] to
    /// [`crate::Client::sign_in`] together with the code the user received.
    CodeRequired(LoginToken),
    /// Telegram recognized a logout token and authorized the session
    /// immediately. The contained `String` is the account's display name.
    /// No code entry is needed, the client is already signed in.
    AlreadyAuthorized(String),
}

// Typed error helpers

#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum ErrorKind {
    /// The transfer was cancelled by the caller.
    Cancelled,
    /// Telegram rate limit. Contains seconds to wait.
    FloodWait(u64),
    /// Network or I/O failure.
    Network,
    /// Authentication / session error.
    Auth,
    /// DC migration redirect. Contains target DC id.
    Migration(i32),
    /// Generic Telegram RPC error.
    Rpc { code: i32, name: String },
    /// File or media transfer error.
    Transfer,
    /// Other / unclassified.
    Other,
}

/// Extension trait adding `.kind()` and `.friendly()` to [`InvocationError`].
pub trait InvocationErrorExt {
    fn kind(&self) -> ErrorKind;
    fn friendly(&self) -> String;
}

impl InvocationErrorExt for InvocationError {
    fn kind(&self) -> ErrorKind {
        match self {
            Self::Rpc(e) => {
                if e.code == 420 {
                    return ErrorKind::FloodWait(e.value.unwrap_or(0) as u64);
                }
                if e.code == 303 {
                    return ErrorKind::Migration(e.value.unwrap_or(1) as i32);
                }
                if e.code == 401
                    || e.name.contains("AUTH")
                    || e.name == "SESSION_EXPIRED"
                    || e.name == "SESSION_REVOKED"
                {
                    return ErrorKind::Auth;
                }
                if e.name.contains("FILE") || e.name.contains("UPLOAD") {
                    return ErrorKind::Transfer;
                }
                ErrorKind::Rpc {
                    code: e.code,
                    name: e.name.clone(),
                }
            }
            Self::Io(_) | Self::Dropped => ErrorKind::Network,
            Self::Migrate(dc) => ErrorKind::Migration(*dc),
            Self::StaleHash | Self::PeerNotCached(_) => ErrorKind::Auth,
            Self::Deserialize(s) if s.contains("cancel") => ErrorKind::Cancelled,
            Self::Deserialize(_) => ErrorKind::Other,
            _ => ErrorKind::Other,
        }
    }

    fn friendly(&self) -> String {
        match self {
            Self::Rpc(e) => {
                if e.code == 420 {
                    let secs = e.value.unwrap_or(0);
                    return format!("Telegram rate limit reached. Retry after {secs} seconds.");
                }
                if e.code == 303 {
                    let dc = e.value.unwrap_or(1);
                    return format!("Redirecting to datacenter {dc}.");
                }
                if e.code == 401 {
                    return format!("Authentication error: {}. Please log in again.", e.name);
                }
                if e.code == 400 && e.name == "PHONE_CODE_INVALID" {
                    return "Invalid or expired verification code.".into();
                }
                if e.code == 400 && e.name == "PASSWORD_HASH_INVALID" {
                    return "Wrong 2FA password.".into();
                }
                if e.code == 400 && e.name == "PEER_ID_INVALID" {
                    return "Peer not found or not cached. Try resolving by username first.".into();
                }
                if e.name == "CHAT_WRITE_FORBIDDEN" {
                    return "You do not have write access to this chat.".into();
                }
                if e.name == "USER_BANNED_IN_CHANNEL" {
                    return "You are banned in this channel.".into();
                }
                format!(
                    "Telegram error ({code}): {name}",
                    code = e.code,
                    name = e.name
                )
            }
            Self::Io(e) => format!("Network error: {e}"),
            Self::Deserialize(s) if s.contains("cancel") => "Transfer cancelled.".into(),
            Self::Deserialize(s) => format!("Response parse error: {s}"),
            Self::Dropped => "Request dropped (connection closed).".into(),
            Self::Migrate(dc) => format!("DC migration to {dc}."),
            Self::StaleHash => "Access hash expired. Please retry.".into(),
            Self::PeerNotCached(s) => format!("Peer not cached: {s}. Try resolving it first."),
            _ => format!("{self}"),
        }
    }
}
