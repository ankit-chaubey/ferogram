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

use crate::*;
#[allow(unused_imports)]
use crate::{
    InputMessage, InvocationError, PeerRef,
    dialog::{Dialog, DialogIter, MessageIter},
    inline_iter, media, participants, search, update,
};
use ferogram_tl_types::{Cursor, Deserializable};

impl Client {
    /// Sign in as a bot.
    pub async fn bot_sign_in(&self, token: &str) -> Result<String, InvocationError> {
        tracing::info!("[ferogram::auth] not signed in; authenticating with bot token");
        let req = tl::functions::auth::ImportBotAuthorization {
            flags: 0,
            api_id: self.inner.api_id,
            api_hash: self.inner.api_hash.clone(),
            bot_auth_token: token.to_string(),
        };

        let result = self.invoke(&req).await?;

        let name = match result {
            tl::enums::auth::Authorization::Authorization(a) => {
                self.cache_user(&a.user).await;
                Self::extract_user_name(&a.user)
            }
            tl::enums::auth::Authorization::SignUpRequired(_) => {
                return Err(InvocationError::Deserialize(
                    "unexpected SignUpRequired during bot sign-in".into(),
                ));
            }
        };
        tracing::info!("[ferogram::auth] bot signed in: {name}");
        self.inner
            .is_bot
            .store(true, std::sync::atomic::Ordering::Relaxed);
        self.inner
            .signed_in
            .store(true, std::sync::atomic::Ordering::SeqCst);
        let _ = self.sync_pts_state().await;
        Ok(name)
    }

    /// Request a login code for a user account.
    pub async fn request_login_code(&self, phone: &str) -> Result<LoginToken, InvocationError> {
        use tl::enums::auth::{SentCode, SentCodeType};

        tracing::info!("[ferogram::auth] not signed in; requesting login code from Telegram");
        let req = self.make_send_code_req(phone);
        let body: Vec<u8> = self.rpc_call_raw(&req).await?;

        let mut cur = Cursor::from_slice(&body);
        let hash = match tl::enums::auth::SentCode::deserialize(&mut cur)? {
            SentCode::SentCode(s) => {
                let sent_via = match &s.r#type {
                    SentCodeType::App(_) => {
                        "a Telegram message on another already-logged-in device/app \
                         (not SMS)"
                    }
                    SentCodeType::Sms(_) => "SMS",
                    SentCodeType::Call(_) => "a voice call reading out the code",
                    SentCodeType::FlashCall(_) => "a flash call (read the caller ID digits)",
                    SentCodeType::MissedCall(_) => "a missed call (read the last digits)",
                    SentCodeType::FragmentSms(_) => "Fragment SMS (fragment.com anonymous number)",
                    SentCodeType::FirebaseSms(_) => "SMS (Firebase-verified)",
                    SentCodeType::SmsWord(_) => "SMS (word code)",
                    SentCodeType::SmsPhrase(_) => "SMS (phrase code)",
                    SentCodeType::EmailCode(_) => "email",
                    SentCodeType::SetUpEmailRequired(_) => "email (setup required first)",
                };
                tracing::info!("[ferogram::auth] login code sent via {sent_via}");
                tracing::debug!(
                    "[ferogram::auth] phone_code_hash acquired (len={})",
                    s.phone_code_hash.len()
                );
                s.phone_code_hash
            }
            SentCode::Success(_) => {
                return Err(InvocationError::Deserialize("unexpected Success".into()));
            }
            SentCode::PaymentRequired(_) => {
                return Err(InvocationError::Deserialize(
                    "payment required to send code".into(),
                ));
            }
        };
        Ok(LoginToken {
            phone: phone.to_string(),
            phone_code_hash: hash,
        })
    }

    /// Complete sign-in with the code sent to the phone.
    pub async fn sign_in(&self, token: &LoginToken, code: &str) -> Result<String, SignInError> {
        tracing::debug!("[ferogram::auth] submitting auth.signIn");
        let req = tl::functions::auth::SignIn {
            phone_number: token.phone.clone(),
            phone_code_hash: token.phone_code_hash.clone(),
            phone_code: Some(code.trim().to_string()),
            email_verification: None,
        };

        let body = match self.rpc_call_raw(&req).await {
            Ok(b) => b,
            Err(e) if e.is("SESSION_PASSWORD_NEEDED") => {
                tracing::info!("[ferogram::auth] 2FA password required");
                let t = self.get_password_info().await.map_err(SignInError::Other)?;
                return Err(SignInError::PasswordRequired(Box::new(t)));
            }
            Err(e) if e.is("PHONE_CODE_*") => {
                tracing::warn!("[ferogram::auth] login code rejected: {e}");
                return Err(SignInError::InvalidCode);
            }
            Err(e) => return Err(SignInError::Other(e)),
        };

        let mut cur = Cursor::from_slice(&body);
        match tl::enums::auth::Authorization::deserialize(&mut cur)
            .map_err(|e| SignInError::Other(e.into()))?
        {
            tl::enums::auth::Authorization::Authorization(a) => {
                self.cache_user(&a.user).await;
                let name = Self::extract_user_name(&a.user);
                tracing::info!("[ferogram::auth] signed in: welcome, {name}");
                self.inner
                    .signed_in
                    .store(true, std::sync::atomic::Ordering::SeqCst);
                let _ = self.sync_pts_state().await;
                Ok(name)
            }
            tl::enums::auth::Authorization::SignUpRequired(_) => {
                tracing::warn!(
                    "[ferogram::auth] phone number not registered on Telegram; sign-up required"
                );
                Err(SignInError::SignUpRequired)
            }
        }
    }

    /// Complete 2FA login.
    pub async fn check_password(
        &self,
        token: PasswordToken,
        password: impl AsRef<[u8]>,
    ) -> Result<String, InvocationError> {
        tracing::debug!("[ferogram::auth] computing SRP 2FA proof");
        let pw = token.password;
        let algo = pw
            .current_algo
            .ok_or_else(|| InvocationError::Deserialize("no current_algo".into()))?;
        let (salt1, salt2, p, g) = extract_password_params(&algo)?;
        let g_b = pw
            .srp_b
            .ok_or_else(|| InvocationError::Deserialize("no srp_b".into()))?;
        let a = pw.secure_random;
        let srp_id = pw
            .srp_id
            .ok_or_else(|| InvocationError::Deserialize("no srp_id".into()))?;

        let (m1, g_a) =
            two_factor_auth::calculate_2fa(salt1, salt2, p, g, &g_b, &a, password.as_ref())
                .map_err(|e| InvocationError::Deserialize(e.to_string()))?;
        let req = tl::functions::auth::CheckPassword {
            password: tl::enums::InputCheckPasswordSrp::InputCheckPasswordSrp(
                tl::types::InputCheckPasswordSrp {
                    srp_id,
                    a: g_a.to_vec(),
                    m1: m1.to_vec(),
                },
            ),
        };
        tracing::debug!("[ferogram::auth] submitting auth.checkPassword");

        let body: Vec<u8> = self.rpc_call_raw(&req).await?;
        let mut cur = Cursor::from_slice(&body);
        match tl::enums::auth::Authorization::deserialize(&mut cur)? {
            tl::enums::auth::Authorization::Authorization(a) => {
                self.cache_user(&a.user).await;
                let name = Self::extract_user_name(&a.user);
                tracing::info!("[ferogram::auth] 2FA verified; signed in: welcome, {name}");
                self.inner
                    .signed_in
                    .store(true, std::sync::atomic::Ordering::SeqCst);
                let _ = self.sync_pts_state().await;
                Ok(name)
            }
            tl::enums::auth::Authorization::SignUpRequired(_) => Err(InvocationError::Deserialize(
                "unexpected SignUpRequired after 2FA".into(),
            )),
        }
    }

    /// Sign out and invalidate the current session.
    pub async fn sign_out(&self) -> Result<bool, InvocationError> {
        let req = tl::functions::auth::LogOut {};
        match self.rpc_call_raw(&req).await {
            Ok(_) => {
                tracing::info!("[ferogram::auth] signed out");
                // Clear all pooled connections and cached auth keys so that
                // stale sockets cannot survive logout/reset.
                self.inner.dc_pool.lock().await.conns.clear();
                self.inner.transfer_pool.lock().await.conns.clear();
                {
                    let mut opts: tokio::sync::MutexGuard<
                        '_,
                        std::collections::HashMap<i32, DcEntry>,
                    > = self.inner.dc_options.lock().await;
                    for entry in opts.values_mut() {
                        entry.auth_key = None;
                        entry.first_salt = 0;
                    }
                }
                // Clear per-DC connect gates so fresh connections can be made after re-login.
                self.inner.dc_connect_gates.lock().clear();
                Ok(true)
            }
            Err(e) if e.is("AUTH_KEY_UNREGISTERED") => Ok(false),
            Err(e) => Err(e),
        }
    }

    /// Fetch user info by ID. Returns `None` for each ID that is not found.
    ///
    /// Used internally by [`update::IncomingMessage::sender_user`].
    pub async fn get_users_by_id(
        &self,
        ids: &[i64],
    ) -> Result<Vec<Option<crate::types::User>>, InvocationError> {
        let cache: tokio::sync::RwLockReadGuard<'_, crate::PeerCache> =
            self.inner.peer_cache.read().await;
        let input_ids: Vec<tl::enums::InputUser> = ids
            .iter()
            .map(|&id| {
                if id == 0 {
                    tl::enums::InputUser::UserSelf
                } else {
                    let hash = cache.users.get(&id).copied().unwrap_or(0);
                    tl::enums::InputUser::InputUser(tl::types::InputUser {
                        user_id: id,
                        access_hash: hash,
                    })
                }
            })
            .collect();
        drop(cache);
        let req = tl::functions::users::GetUsers { id: input_ids };
        let body: Vec<u8> = self.rpc_call_raw(&req).await?;
        let mut cur = Cursor::from_slice(&body);
        let users = Vec::<tl::enums::User>::deserialize(&mut cur)?;
        self.cache_users_slice(&users).await;
        Ok(users
            .into_iter()
            .map(crate::types::User::from_raw)
            .collect())
    }

    /// Resolve a user by message context (no access_hash required).
    /// Returns `None` if Telegram returns no matching user.
    pub async fn get_user_from_message(
        &self,
        peer: tl::enums::InputPeer,
        msg_id: i32,
        user_id: i64,
    ) -> Result<Option<tl::types::User>, InvocationError> {
        let req = tl::functions::users::GetUsers {
            id: vec![tl::enums::InputUser::FromMessage(
                tl::types::InputUserFromMessage {
                    peer,
                    msg_id,
                    user_id,
                },
            )],
        };
        let body: Vec<u8> = self.rpc_call_raw(&req).await?;
        let mut cur = Cursor::from_slice(&body);
        let users = Vec::<tl::enums::User>::deserialize(&mut cur)?;
        self.cache_users_slice(&users).await;
        Ok(users.into_iter().find_map(|u| match u {
            tl::enums::User::User(u) => Some(u),
            _ => None,
        }))
    }

    /// The logged-in account's numeric user ID, if already known.
    ///
    /// This is a cheap, synchronous, no-network lookup of a value captured
    /// automatically the first time a `User` object with `is_self == true`
    /// was cached (sign-in, `get_me()`, etc.). Returns `None` only if neither
    /// has happened yet in this session - call `my_id_or_fetch()` for a
    /// version that falls back to a network call in that case.
    pub fn my_id(&self) -> Option<i64> {
        match self
            .inner
            .self_user_id
            .load(std::sync::atomic::Ordering::Relaxed)
        {
            0 => None,
            id => Some(id),
        }
    }

    /// Like [`my_id`](Self::my_id), but performs a one-time `get_me()` call
    /// to populate the cache if it isn't already known.
    pub async fn my_id_or_fetch(&self) -> Result<i64, InvocationError> {
        if let Some(id) = self.my_id() {
            return Ok(id);
        }
        Ok(self.get_me().await?.id)
    }

    /// Fetch information about the logged-in user.
    pub async fn get_me(&self) -> Result<tl::types::User, InvocationError> {
        let req = tl::functions::users::GetUsers {
            id: vec![tl::enums::InputUser::UserSelf],
        };
        let body: Vec<u8> = self.rpc_call_raw(&req).await?;
        let mut cur = Cursor::from_slice(&body);
        let users = Vec::<tl::enums::User>::deserialize(&mut cur)?;
        self.cache_users_slice(&users).await;
        users
            .into_iter()
            .find_map(|u| match u {
                tl::enums::User::User(u) => Some(u),
                _ => None,
            })
            .ok_or_else(|| InvocationError::Deserialize("getUsers returned no user".into()))
    }

    async fn get_password_info(&self) -> Result<PasswordToken, InvocationError> {
        tracing::debug!("[ferogram::auth] fetching 2FA password parameters");
        let body = self
            .rpc_call_raw(&tl::functions::account::GetPassword {})
            .await?;
        let mut cur = Cursor::from_slice(&body);
        let tl::enums::account::Password::Password(pw) =
            tl::enums::account::Password::deserialize(&mut cur)?;
        Ok(PasswordToken { password: pw })
    }

    fn make_send_code_req(&self, phone: &str) -> tl::functions::auth::SendCode {
        tl::functions::auth::SendCode {
            phone_number: phone.to_string(),
            api_id: self.inner.api_id,
            api_hash: self.inner.api_hash.clone(),
            settings: tl::enums::CodeSettings::CodeSettings(tl::types::CodeSettings {
                allow_flashcall: false,
                current_number: false,
                allow_app_hash: false,
                allow_missed_call: false,
                allow_firebase: false,
                unknown_number: false,
                logout_tokens: None,
                token: None,
                app_sandbox: None,
            }),
        }
    }

    fn extract_user_name(user: &tl::enums::User) -> String {
        match user {
            tl::enums::User::User(u) => format!(
                "{} {}",
                u.first_name.as_deref().unwrap_or(""),
                u.last_name.as_deref().unwrap_or("")
            )
            .trim()
            .to_string(),
            tl::enums::User::Empty(_) => "(unknown)".into(),
        }
    }

    /// Generate a QR-code login token.
    ///
    /// Returns `(token_bytes, expires_unix_ts)`. Encode `token_bytes` as a
    /// `tg://login?token=<base64url>` URL and present as a QR code.
    ///
    /// Call `import_qr_token` once the user scans it, then poll until you
    /// receive `Update::Raw` with `updateLoginToken` (constructor `0x564fe691`),
    /// or call `export_login_token` again to check.
    ///
    /// # Example
    /// ```rust,no_run
    /// # use ferogram::Client;
    /// # async fn ex(client: Client) -> Result<(), Box<dyn std::error::Error>> {
    /// let (token, expires) = client.export_login_token().await?;
    /// // base64url-encode `token` and make a QR code
    /// # Ok(()) }
    /// ```
    pub async fn export_login_token(&self) -> Result<(Vec<u8>, i32), InvocationError> {
        use ferogram_tl_types::{Cursor, Deserializable};
        let req = tl::functions::auth::ExportLoginToken {
            api_id: self.inner.api_id,
            api_hash: self.inner.api_hash.clone(),
            except_ids: vec![],
        };
        let body: Vec<u8> = self.rpc_call_raw(&req).await?;
        let mut cur = Cursor::from_slice(&body);
        match tl::enums::auth::LoginToken::deserialize(&mut cur)? {
            tl::enums::auth::LoginToken::LoginToken(t) => Ok((t.token, t.expires)),
            tl::enums::auth::LoginToken::MigrateTo(m) => {
                // Migrate and retry
                self.migrate_to(m.dc_id).await?;
                let req2 = tl::functions::auth::ImportLoginToken { token: m.token };
                let body2 = self.rpc_call_raw(&req2).await?;
                let mut cur2 = Cursor::from_slice(&body2);
                match tl::enums::auth::LoginToken::deserialize(&mut cur2)? {
                    tl::enums::auth::LoginToken::LoginToken(t) => Ok((t.token, t.expires)),
                    _ => Err(InvocationError::Deserialize(
                        "QR login: unexpected token state after migration".into(),
                    )),
                }
            }
            tl::enums::auth::LoginToken::Success(s) => {
                // Already authorised (user scanned before we called this)
                if let tl::enums::auth::Authorization::Authorization(a) = s.authorization {
                    self.cache_user(&a.user).await;
                    Self::extract_user_name(&a.user);
                    self.inner
                        .signed_in
                        .store(true, std::sync::atomic::Ordering::SeqCst);
                    let _ = self.sync_pts_state().await;
                }
                Ok((vec![], 0))
            }
        }
    }

    /// Check whether a QR-code token has been scanned.
    ///
    /// Returns `Some(username)` if the user has scanned and confirmed the QR
    /// code, or `None` if still pending.
    pub async fn check_qr_login(&self, token: Vec<u8>) -> Result<Option<String>, InvocationError> {
        use ferogram_tl_types::{Cursor, Deserializable};
        let req = tl::functions::auth::ImportLoginToken { token };
        let body: Vec<u8> = self.rpc_call_raw(&req).await?;
        let mut cur = Cursor::from_slice(&body);
        match tl::enums::auth::LoginToken::deserialize(&mut cur)? {
            tl::enums::auth::LoginToken::Success(s) => {
                if let tl::enums::auth::Authorization::Authorization(a) = s.authorization {
                    self.cache_user(&a.user).await;
                    let name = Self::extract_user_name(&a.user);
                    self.inner
                        .signed_in
                        .store(true, std::sync::atomic::Ordering::SeqCst);
                    let _ = self.sync_pts_state().await;
                    Ok(Some(name))
                } else {
                    Ok(None)
                }
            }
            _ => Ok(None),
        }
    }
}

// Helper: extract SRP parameters from a PasswordKdfAlgo.
#[allow(clippy::type_complexity)]
fn extract_password_params(
    algo: &tl::enums::PasswordKdfAlgo,
) -> Result<(&[u8], &[u8], &[u8], i32), InvocationError> {
    // Match the SHA256^2 + PBKDF2 + ModPow variant (only non-Unknown variant)
    match algo {
        tl::enums::PasswordKdfAlgo::Sha256Sha256Pbkdf2Hmacsha512iter100000Sha256ModPow(a) => {
            Ok((&a.salt1, &a.salt2, &a.p, a.g))
        }
        _ => Err(InvocationError::Deserialize("unknown 2FA algo".into())),
    }
}
