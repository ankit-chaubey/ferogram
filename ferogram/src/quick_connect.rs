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

use crate::{
    Client, InvocationError, SendCodeOutcome, ShutdownToken, SignInError, builder::BuilderError,
};

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
            .connect_and_login()
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
        let name = client.interactive_sign_in(&credential).await?;
        println!("✅ Signed in as {name}");
    }

    client
        .save_session()
        .await
        .map_err(QuickConnectError::Auth)?;
    println!("💾 Session saved");

    Ok(())
}

impl Client {
    /// Prompt (stdin) for a login code, and a 2FA password if required, then
    /// sign in as a user. Returns the signed-in user's display name.
    ///
    /// Skips the code prompt entirely if a replayed logout token already
    /// authorized the session (see [`SendCodeOutcome::AlreadyAuthorized`]).
    pub async fn interactive_sign_in(&self, phone: &str) -> Result<String, InvocationError> {
        println!("📱 Requesting login code for {phone} …");
        let outcome = self.request_login_code(phone).await?;

        let token = match outcome {
            SendCodeOutcome::AlreadyAuthorized(name) => return Ok(name),
            SendCodeOutcome::CodeRequired(token) => token,
        };

        println!("📨 Code sent, check Telegram (or SMS)");
        let code = prompt_io("Enter the login code: ")?;

        match self.sign_in(&token, &code).await {
            Ok(name) => Ok(name),
            Err(SignInError::PasswordRequired(pw_token)) => {
                println!("🔒 Two-step verification enabled");
                let password = prompt_io("Enter your 2FA password: ")?;
                Ok(self.check_password(*pw_token, password.as_bytes()).await?)
            }
            Err(e) => Err(e.into()),
        }
    }
}

// Bot tokens are always `<digits>:<alphanumeric>`, e.g. `123456789:AABBcc...`
fn is_bot_token(s: &str) -> bool {
    match s.split_once(':') {
        Some((id, _)) => !id.is_empty() && id.chars().all(|c| c.is_ascii_digit()),
        None => false,
    }
}

fn prompt(msg: &str) -> Result<String, QuickConnectError> {
    prompt_io(msg).map_err(QuickConnectError::Io)
}

fn prompt_io(msg: &str) -> io::Result<String> {
    print!("{msg}");
    io::stdout().flush()?;
    let mut buf = String::new();
    io::stdin().read_line(&mut buf)?;
    Ok(buf.trim().to_string())
}

/// Errors returned by [`Client::quick_connect`].
#[derive(Debug)]
pub enum QuickConnectError {
    /// [`crate::ClientBuilder::connect`] failed (missing api_id/hash or network error).
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

impl From<InvocationError> for QuickConnectError {
    fn from(e: InvocationError) -> Self {
        match &e {
            InvocationError::Rpc(rpc) if rpc.name == "PHONE_CODE_INVALID" => Self::InvalidCode,
            InvocationError::Rpc(rpc) if rpc.name == "PHONE_NUMBER_UNOCCUPIED" => {
                Self::SignUpRequired
            }
            _ => Self::Auth(e),
        }
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
