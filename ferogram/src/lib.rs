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

//! Async Rust client for the Telegram MTProto API.
//!
//! ferogram talks to Telegram directly over MTProto with no Bot API proxy. It works
//! for both bots and user accounts. Most things you'd want to do with Telegram
//! are already covered. If something isn't, you can always drop down to
//! [`client.invoke()`](Client::invoke) and call any TL function directly.
//!
//! Still in development but already covers major use cases for production.
//! Check the [CHANGELOG] before upgrading.
//!
//! [CHANGELOG]: https://github.com/ankit-chaubey/ferogram/blob/main/CHANGELOG.md
//!
//! # Quick start: bot
//!
//! ```rust,no_run
//! use ferogram::{Client, update::Update};
//!
//! #[tokio::main]
//! async fn main() -> anyhow::Result<()> {
//!     let (client, _) = Client::builder()
//!         .api_id(std::env::var("API_ID")?.parse()?)
//!         .api_hash(std::env::var("API_HASH")?)
//!         .session("bot.session")
//!         .connect().await?;
//!
//!     client.bot_sign_in(&std::env::var("BOT_TOKEN")?).await?;
//!
//!     let mut stream = client.stream_updates();
//!     while let Some(upd) = stream.next().await {
//!         if let Update::NewMessage(msg) = upd {
//!             if !msg.outgoing() {
//!                 msg.reply(msg.text().unwrap_or_default()).await.ok();
//!             }
//!         }
//!     }
//!     Ok(())
//! }
//! ```
//!
//! # Quick start: user account
//!
//! ```rust,no_run
//! use ferogram::{Client, SignInError};
//! # fn read_line() -> String { String::new() }
//!
//! #[tokio::main]
//! async fn main() -> anyhow::Result<()> {
//!     let (client, _) = Client::builder()
//!         .api_id(std::env::var("API_ID")?.parse()?)
//!         .api_hash(std::env::var("API_HASH")?)
//!         .session("my.session")
//!         .connect().await?;
//!
//!     if !client.is_authorized().await? {
//!         let token = client.request_login_code("+1234567890").await?;
//!         match client.sign_in(&token, &read_line()).await {
//!             Ok(_) => {}
//!             Err(SignInError::PasswordRequired(t)) => {
//!                 client.check_password(*t, &read_line()).await?;
//!             }
//!             Err(e) => return Err(e.into()),
//!         }
//!         client.save_session().await?;
//!     }
//!
//!     client.send_message("me", "Hello from ferogram!").await?;
//!     Ok(())
//! }
//! ```
//!
//! # Dispatcher and filters
//!
//! ```rust,ignore
//! use ferogram::filters::{Dispatcher, command, private, text_contains};
//!
//! let mut dp = Dispatcher::new();
//!
//! dp.on_message(command("start"), |msg| async move {
//!     msg.reply("Hello!").await.ok();
//! });
//!
//! dp.on_message(private() & text_contains("help"), |msg| async move {
//!     msg.reply("Type /start to begin.").await.ok();
//! });
//!
//! while let Some(upd) = stream.next().await {
//!     dp.dispatch(upd).await;
//! }
//! # }
//! ```
//!
//! Filters compose with `&`, `|`, `!`. Built-ins: `command`, `private`, `group`,
//! `channel`, `text`, `media`, `photo`, `forwarded`, `reply`, `album`, `regex`, and more.
//!
//! # FSM
//!
//! ```rust,ignore
//! use std::sync::Arc;
//!
//! #[derive(FsmState, Clone, Debug, PartialEq)]
//! enum Form { Name, Age }
//!
//! dp.with_state_storage(Arc::new(MemoryStorage::new()));
//!
//! dp.on_message_fsm(text(), Form::Name, |msg, state| async move {
//!     state.set_data("name", msg.text().unwrap()).await.ok();
//!     state.transition(Form::Age).await.ok();
//!     msg.reply("How old are you?").await.ok();
//! });
//! ```
//!
//! # Raw API
//!
//! If something isn't wrapped yet, you can call any Layer 225 TL function directly:
//!
//! ```rust,ignore
//! use ferogram::tl;
//!
//! let req = tl::functions::messages::SendMessage {
//!     peer: peer.into(),
//!     message: "Hello!".into(),
//!     random_id: ferogram::random_i64_pub(),
//!     ..Default::default()
//! };
//! client.invoke(&req).await?;
//! ```
//!
//! # Session backends
//!
//! Binary file by default. Switch to SQLite, libSQL, or a base64 string with a
//! feature flag. Bring your own backend by implementing [`SessionBackend`].
//!
//! ```rust,ignore
//! // Portable string session, useful for serverless or env-var setups
//! let s = client.export_session_string().await?;
//! let (client, _) = Client::builder().session_string(s).connect().await?;
//! ```
//!
//! # Features
//!
//! Most common use cases are already covered. Full list in
//! [FEATURES.md](https://github.com/ankit-chaubey/ferogram/blob/main/FEATURES.md).
//!
//! If something's missing, feel free to open a feature request or PR.
//! Check the [contributing guidelines](https://github.com/ankit-chaubey/ferogram#contributing) first.
//!
//! # Community
//!
//! - Channel (releases, news): [t.me/Ferogram](https://t.me/Ferogram)
//! - Chat (questions, help): [t.me/FerogramChat](https://t.me/FerogramChat)
//! - Guide: [ferogram.ankitchaubey.in](https://ferogram.ankitchaubey.in)
//! - GitHub: [ankit-chaubey/ferogram](https://github.com/ankit-chaubey/ferogram)

#![cfg_attr(docsrs, feature(doc_cfg))]
#![deny(unsafe_code)]

pub mod builder;
mod client;
mod dialog;
mod errors;
mod input_message;
pub mod media;
pub mod message_box;
mod mini_app;
pub mod parsers;
pub mod participants;
mod peer_cache;
pub mod persist;
mod quick_connect;
mod restart;
mod retry;
mod session;
mod two_factor_auth;
pub mod update;

pub mod cdn_download;
pub mod conversation;
pub mod dc_pool;
pub mod dns_resolver;
pub mod filters;
pub mod inline_iter;
pub mod keyboard;
pub mod search;
pub mod session_backend;
pub mod socks5;
pub mod special_config;
pub mod transport_intermediate;
pub mod transport_obfuscated;
pub mod types;
pub mod typing_guard;

#[macro_use]
pub mod macros;
pub mod guest_chat;
pub mod peer_ext;
pub mod peer_ref;
pub mod poll;
pub mod reactions;

pub mod dc_migration;
pub mod proxy;

pub mod fsm;
pub mod middleware;
pub mod util;

// Re-export FsmState at the crate root for convenience.
pub use fsm::FsmState;

// Re-export the derive macro when the feature is enabled.
#[cfg(feature = "derive")]
#[cfg_attr(docsrs, doc(cfg(feature = "derive")))]
pub use ferogram_derive::FsmState;

pub use builder::{BuilderError, ClientBuilder};
pub use client::Client;
pub use client::{Config, ShutdownToken, UpdateStream};
pub use dialog::{Dialog, DialogIter, MessageIter};
pub use errors::{InvocationError, LoginToken, PasswordToken, RpcError, SignInError};
pub use ferogram_connect::TransportKind;
pub use ferogram_connect::random_i64 as random_i64_pub;
pub use guest_chat::GuestChatQuery;
pub use input_message::{ForwardOptions, InputMessage, InvoiceOptions, LinkKind};
pub use keyboard::{Button, InlineKeyboard, ReplyKeyboard};
pub use media::{Document, DownloadIter, Downloadable, Photo, Sticker, UploadedFile};
pub use mini_app::{MiniApp, MiniAppSession};
pub use participants::{Participant, ProfilePhotoIter};
pub use peer_cache::{ExperimentalFeatures, PeerCache, PeerType};
pub use peer_ext::{OptionPeerExt, PeerExt};
pub use peer_ref::PeerRef;
pub use poll::PollBuilder;
pub use proxy::{MtProxyConfig, parse_proxy_link};
pub use quick_connect::QuickConnectError;
pub use restart::{ConnectionRestartPolicy, ExponentialBackoff, FixedInterval, NeverRestart};
pub use retry::{AutoSleep, CircuitBreaker, NoRetries, RetryContext, RetryPolicy};
pub use search::{GlobalSearchBuilder, SearchBuilder};
pub use session::{DcEntry, DcFlags};
#[cfg(feature = "libsql-session")]
#[cfg_attr(docsrs, doc(cfg(feature = "libsql-session")))]
pub use session_backend::LibSqlBackend;
#[cfg(feature = "sqlite-session")]
#[cfg_attr(docsrs, doc(cfg(feature = "sqlite-session")))]
pub use session_backend::SqliteBackend;
pub use session_backend::{
    BinaryFileBackend, InMemoryBackend, SessionBackend, StringSessionBackend, UpdateStateChange,
};
pub use socks5::Socks5Config;
pub use types::ChannelKind;
pub use types::{Channel, Chat, Group, User};
pub use typing_guard::TypingGuard;
pub use update::{BotStoppedUpdate, MessageReactionUpdate, PollVoteUpdate};
pub use update::{ButtonFilter, Update};
pub use update::{ChatActionUpdate, JoinRequestUpdate, ParticipantUpdate, UserStatusUpdate};
pub use update::{ChatBoostUpdate, PreCheckoutQueryUpdate, ShippingQueryUpdate};

/// Re-export of `ferogram_tl_types`.
pub use ferogram_tl_types as tl;

/// Re-export of [`ferogram_mtproto`].
pub use ferogram_mtproto as mtproto;

/// Re-export of [`ferogram_crypto`].
pub use ferogram_crypto as crypto;

#[cfg(feature = "parser")]
pub use ferogram_tl_parser as parser;

#[cfg(feature = "codegen")]
pub use ferogram_tl_gen as codegen;

pub use ferogram_crypto::AuthKey;
pub use ferogram_mtproto::authentication::{self, Finished, finish, step1, step2, step3};
pub use ferogram_tl_types::{Identifiable, LAYER, Serializable};
