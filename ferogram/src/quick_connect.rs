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

//! [`Client::quick_connect`] - connect and authenticate in one call.
//!
//! For advanced options (proxy, PFS, custom transport, catch-up, etc.)
//! use [`Client::builder()`] directly.

use std::io::{self, Write};

use crate::{Client, InvocationError, ShutdownToken, SignInError, builder::BuilderError};

impl Client {
    /// Connect and authenticate in a single call.
    ///
    /// Prompts interactively (stdin) for a phone number or bot token, then
    /// drives the full auth flow - login code and 2FA password if required.
    /// If the session is already authorized the prompt is skipped entirely.
    ///
    /// For advanced options (proxy, custom transport, PFS, catch-up, etc.)
    /// use [`Client::builder()`] instead.
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// use ferogram::Client;
    ///
    /// const API_ID: i32 = 0;
    /// const API_HASH: &str = "";
    ///
    /// # #[tokio::main] async fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// let (client, _) = Client::quick_connect("my.session", API_ID, API_HASH).await?;
    /// # Ok(()) }
    /// ```
    pub async fn quick_connect(
        session: impl AsRef<std::path::Path>,
        api_id: i32,
        api_hash: &str,
    ) -> Result<(Client, ShutdownToken), QuickConnectError> {
        Client::builder()
            .session(session)
            .api_id(api_id)
            .api_hash(api_hash)
            .connect()
            .await
    }
}

/// Shared interactive login: skip if already authorized, otherwise prompt for
/// a phone number or bot token, drive the full auth flow (code, 2FA), and
/// persist the result. Used by both [`Client::quick_connect`] and
/// [`crate::builder::ClientBuilder::connect_and_login`] so there is exactly
/// one implementation of this flow to keep correct.
pub(crate) async fn login_interactive(client: &Client) -> Result<(), QuickConnectError> {
    if client
        .is_authorized()
        .await
        .map_err(QuickConnectError::Auth)?
    {
        println!("✅ Already signed in");
        return Ok(());
    }

    println!("🔑 Not signed in, starting login flow");
    let credential = prompt("Enter phone number or bot token: ")?;

    if is_bot_token(&credential) {
        println!("🤖 Detected bot token, authenticating …");
        let name = client
            .bot_sign_in(&credential)
            .await
            .map_err(QuickConnectError::Auth)?;
        println!("✅ Signed in as bot: {name}");
    } else {
        sign_in_user(client, &credential).await?;
    }

    client
        .save_session()
        .await
        .map_err(QuickConnectError::Auth)?;
    println!("💾 Session saved");

    Ok(())
}

async fn sign_in_user(client: &Client, phone: &str) -> Result<(), QuickConnectError> {
    println!("📱 Requesting login code for {phone} …");
    let token = client
        .request_login_code(phone)
        .await
        .map_err(QuickConnectError::Auth)?;
    println!("📨 Code sent, check Telegram (or SMS)");

    let code = prompt("Enter the login code: ")?;

    match client.sign_in(&token, &code).await {
        Ok(name) => {
            println!("✅ Signed in as {name}");
        }
        Err(SignInError::PasswordRequired(pw_token)) => {
            println!("🔒 Two-step verification enabled");
            let password = prompt("Enter your 2FA password: ")?;
            let name = client
                .check_password(*pw_token, password.as_bytes())
                .await
                .map_err(QuickConnectError::Auth)?;
            println!("✅ Signed in as {name}");
        }
        Err(SignInError::InvalidCode) => return Err(QuickConnectError::InvalidCode),
        Err(SignInError::SignUpRequired) => return Err(QuickConnectError::SignUpRequired),
        Err(SignInError::Other(e)) => return Err(QuickConnectError::Auth(e)),
    }

    Ok(())
}

// Bot tokens are always `<digits>:<alphanumeric>`, e.g. `123456789:AABBcc...`
fn is_bot_token(s: &str) -> bool {
    match s.split_once(':') {
        Some((id, _)) => !id.is_empty() && id.chars().all(|c| c.is_ascii_digit()),
        None => false,
    }
}

fn prompt(msg: &str) -> Result<String, QuickConnectError> {
    print!("{msg}");
    io::stdout().flush().map_err(QuickConnectError::Io)?;
    let mut buf = String::new();
    io::stdin()
        .read_line(&mut buf)
        .map_err(QuickConnectError::Io)?;
    Ok(buf.trim().to_string())
}

/// Errors returned by [`Client::quick_connect`].
#[derive(Debug)]
pub enum QuickConnectError {
    /// [`ClientBuilder::connect`] failed (missing api_id/hash or network error).
    Builder(BuilderError),
    /// An MTProto RPC call during authentication failed.
    Auth(InvocationError),
    /// The login code entered was incorrect.
    InvalidCode,
    /// Phone number is not registered on Telegram.
    SignUpRequired,
    /// Failed to read from stdin while prompting.
    Io(std::io::Error),
}

impl From<BuilderError> for QuickConnectError {
    fn from(e: BuilderError) -> Self {
        Self::Builder(e)
    }
}

impl std::fmt::Display for QuickConnectError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Builder(e) => write!(f, "connection failed: {e}"),
            Self::Auth(e) => write!(f, "authentication error: {e}"),
            Self::InvalidCode => f.write_str("❌ Invalid login code, please try again"),
            Self::SignUpRequired => f.write_str(
                "❌ Phone number not registered on Telegram, sign up via the official app first",
            ),
            Self::Io(e) => write!(f, "stdin error: {e}"),
        }
    }
}

impl std::error::Error for QuickConnectError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Builder(e) => Some(e),
            Self::Auth(e) => Some(e),
            Self::Io(e) => Some(e),
            _ => None,
        }
    }
}
