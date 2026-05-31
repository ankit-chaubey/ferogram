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
//! const API_ID: i32 = 0; // from https://my.telegram.org
//! const API_HASH: &str = ""; // from https://my.telegram.org
//!
//! #[tokio::main]
//! async fn main() -> anyhow::Result<()> {
//!     let (client, _) = Client::quick_connect("bot.session", API_ID, API_HASH).await?;
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
//! use ferogram::Client;
//!
//! const API_ID: i32 = 0; // from https://my.telegram.org
//! const API_HASH: &str = ""; // from https://my.telegram.org
//!
//! #[tokio::main]
//! async fn main() -> anyhow::Result<()> {
//!     let (client, _) = Client::quick_connect("my.session", API_ID, API_HASH).await?;
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
pub use media::DownloadIter;
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

pub mod file_info;
pub mod fsm;
pub mod middleware;
#[cfg(feature = "experimental")]
pub mod resume;
pub mod transfer;
pub mod update_config;
pub mod util;

pub(crate) mod builder_util;

/// Portable string-session encoding/decoding (V1/V2 binary base64 format).
///
/// Most users never need this module. Just pass any session string
/// directly to [`ClientBuilder::session_string`] and it is handled automatically.
///
/// Use this module only when you need to inspect or construct a
/// [`StringSession`] value programmatically.
pub mod string_session {
    pub use ferogram_session::string_session::{
        FullSession, Session, StringSession, StringSessionError,
    };
}

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
pub use errors::{
    ErrorKind, InvocationError, InvocationErrorExt, LoginToken, PasswordToken, RpcError,
    SignInError,
};
pub use ferogram_connect::TransportKind;
pub use ferogram_connect::random_i64 as random_i64_pub;
pub use file_info::{FileInfo, detect_mime, file_info, file_info_from_path};
pub use guest_chat::GuestChatQuery;
pub use input_message::{ForwardOptions, InputMessage, InvoiceOptions, LinkKind};
pub use keyboard::{Button, InlineKeyboard, ReplyKeyboard};
pub use media::{Document, Downloadable, Photo, Sticker, UploadedFile};
pub use mini_app::{MiniApp, MiniAppSession};
pub use participants::{Participant, ParticipantStatus, ProfilePhotoIter};
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
pub use transfer::{TransferError, TransferHandle, TransferProgress};
pub use types::{Channel, ChannelKind, Chat, Group, User};
pub use typing_guard::TypingGuard;
pub use update::{BotStoppedUpdate, MessageReactionUpdate, PollVoteUpdate};
pub use update::{ButtonFilter, Update};
pub use update::{ChatActionUpdate, JoinRequestUpdate, ParticipantUpdate, UserStatusUpdate};
pub use update::{ChatBoostUpdate, PreCheckoutQueryUpdate, ShippingQueryUpdate};
pub use update_config::{OverflowStrategy, UpdateConfig};

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
/// Return type of [`Client::stats`].
pub enum ChannelStats {
    /// Stats for a broadcast channel.
    Broadcast(tl::enums::stats::BroadcastStats),
    /// Stats for a supergroup (megagroup).
    Megagroup(tl::enums::stats::MegagroupStats),
}

/// Builder returned by [`Client::set_profile`].
///
/// Call `.send().await` to apply changes. Only fields you set are touched;
/// everything else is left exactly as it is.
///
/// `.bio()` and `.name()` work for both users and chats/channels:
/// - On a user peer: `.bio()` sets the account bio, `.name(first, last)` sets
///   the display name.
/// - On a channel/group peer: `.bio()` sets the about text, `.name(first, _)`
///   sets the title (the second argument is ignored for channels).
///
/// The older `.title()` and `.about()` setters are kept for explicit usage and
/// they take priority over `.name()`/`.bio()` when both are set.
pub struct SetProfileBuilder {
    client: Client,
    peer: PeerRef,
    // User fields
    first_name: Option<String>,
    last_name: Option<String>,
    bio: Option<String>,
    emoji_status: Option<(Option<i64>, Option<i32>)>,
    // Chat/channel fields (explicit overrides)
    title: Option<String>,
    about: Option<String>,
    chat_photo: Option<tl::enums::InputChatPhoto>,
    // Shared fields
    username: Option<String>,
    photo: Option<media::UploadedFile>,
    photo_path: Option<std::path::PathBuf>,
}

impl SetProfileBuilder {
    #[doc(hidden)]
    pub fn new(client: Client, peer: PeerRef) -> Self {
        Self {
            client,
            peer,
            first_name: None,
            last_name: None,
            bio: None,
            emoji_status: None,
            title: None,
            about: None,
            chat_photo: None,
            username: None,
            photo: None,
            photo_path: None,
        }
    }

    /// Set the display name.
    ///
    /// For user accounts: sets first and last name.
    /// For channels/groups: sets the title (`first` is used; `last` is ignored).
    pub fn name(mut self, first: impl Into<String>, last: impl Into<String>) -> Self {
        self.first_name = Some(first.into());
        self.last_name = Some(last.into());
        self
    }

    /// Set bio or about text.
    ///
    /// For user accounts: sets the account bio shown on the profile page.
    /// For channels/groups: sets the about/description text.
    pub fn bio(mut self, bio: impl Into<String>) -> Self {
        self.bio = Some(bio.into());
        self
    }

    /// Set username.
    pub fn username(mut self, u: impl Into<String>) -> Self {
        self.username = Some(u.into());
        self
    }

    /// Set profile photo from an already-uploaded file.
    ///
    /// Use [`photo_path`] if you have a local file path and want the upload
    /// handled automatically inside [`send`].
    ///
    /// [`photo_path`]: SetProfileBuilder::photo_path
    /// [`send`]: SetProfileBuilder::send
    pub fn photo(mut self, file: media::UploadedFile) -> Self {
        self.photo = Some(file);
        self
    }

    /// Set profile photo from a local file path.
    ///
    /// The file is uploaded automatically when [`send`] is called.
    /// If you already have an [`UploadedFile`] use [`photo`] instead.
    ///
    /// [`send`]: SetProfileBuilder::send
    /// [`photo`]: SetProfileBuilder::photo
    /// [`UploadedFile`]: media::UploadedFile
    pub fn photo_path(mut self, path: impl Into<std::path::PathBuf>) -> Self {
        self.photo_path = Some(path.into());
        self
    }

    /// Set emoji status. Pass `None` for `document_id` to clear the status.
    pub fn emoji_status(mut self, document_id: Option<i64>, until: Option<i32>) -> Self {
        self.emoji_status = Some((document_id, until));
        self
    }

    /// Explicitly set chat/channel title, overriding `.name()` for channel peers.
    pub fn title(mut self, t: impl Into<String>) -> Self {
        self.title = Some(t.into());
        self
    }

    /// Explicitly set chat/channel about text, overriding `.bio()` for channel peers.
    pub fn about(mut self, a: impl Into<String>) -> Self {
        self.about = Some(a.into());
        self
    }

    /// Set chat/channel photo from a raw [`InputChatPhoto`].
    ///
    /// [`InputChatPhoto`]: tl::enums::InputChatPhoto
    pub fn chat_photo(mut self, p: tl::enums::InputChatPhoto) -> Self {
        self.chat_photo = Some(p);
        self
    }

    /// Apply all changes.
    pub async fn send(mut self) -> Result<(), InvocationError> {
        use ferogram_tl_types as tl;
        // Handle photo_path: upload before resolving anything else.
        if let Some(path) = self.photo_path.take() {
            let uploaded = self.client.upload_file(path, None).await?;
            self.photo = Some(uploaded);
        }

        let peer = self.peer.resolve(&self.client).await?;
        let input_peer = self
            .client
            .inner
            .peer_cache
            .read()
            .await
            .peer_to_input(&peer)?;

        let is_channel_or_chat = matches!(
            &input_peer,
            tl::enums::InputPeer::Channel(_) | tl::enums::InputPeer::Chat(_)
        );

        if is_channel_or_chat {
            // channel or group

            // Title: explicit .title() wins; fall back to .name(first, _).
            let effective_title = self.title.or(self.first_name);
            if let Some(t) = effective_title {
                match &input_peer {
                    tl::enums::InputPeer::Channel(c) => {
                        let req = tl::functions::channels::EditTitle {
                            channel: tl::enums::InputChannel::InputChannel(
                                tl::types::InputChannel {
                                    channel_id: c.channel_id,
                                    access_hash: c.access_hash,
                                },
                            ),
                            title: t,
                        };
                        self.client.rpc_write(&req).await?;
                    }
                    tl::enums::InputPeer::Chat(c) => {
                        let req = tl::functions::messages::EditChatTitle {
                            chat_id: c.chat_id,
                            title: t,
                        };
                        self.client.rpc_write(&req).await?;
                    }
                    _ => {}
                }
            }

            // About: explicit .about() wins; fall back to .bio().
            let effective_about = self.about.or(self.bio);
            if let Some(a) = effective_about {
                let req = tl::functions::messages::EditChatAbout {
                    peer: input_peer.clone(),
                    about: a,
                };
                self.client.rpc_write(&req).await?;
            }

            // Username
            if let Some(u) = self.username {
                let req = tl::functions::account::UpdateUsername { username: u };
                self.client.rpc_write(&req).await?;
            }

            // Photo
            if let Some(file) = self.photo {
                if let Some(chat_photo) = self.chat_photo {
                    // Explicit InputChatPhoto takes priority.
                    match &input_peer {
                        tl::enums::InputPeer::Channel(c) => {
                            let req = tl::functions::channels::EditPhoto {
                                channel: tl::enums::InputChannel::InputChannel(
                                    tl::types::InputChannel {
                                        channel_id: c.channel_id,
                                        access_hash: c.access_hash,
                                    },
                                ),
                                photo: chat_photo,
                            };
                            self.client.rpc_write(&req).await?;
                        }
                        tl::enums::InputPeer::Chat(c) => {
                            let req = tl::functions::messages::EditChatPhoto {
                                chat_id: c.chat_id,
                                photo: chat_photo,
                            };
                            self.client.rpc_write(&req).await?;
                        }
                        _ => {}
                    }
                } else {
                    // UploadedFile: wrap as InputChatPhotoUploaded.
                    let chat_photo = tl::enums::InputChatPhoto::InputChatUploadedPhoto(
                        tl::types::InputChatUploadedPhoto {
                            video: None,
                            file: Some(file.inner),
                            video_start_ts: None,
                            video_emoji_markup: None,
                        },
                    );
                    match &input_peer {
                        tl::enums::InputPeer::Channel(c) => {
                            let req = tl::functions::channels::EditPhoto {
                                channel: tl::enums::InputChannel::InputChannel(
                                    tl::types::InputChannel {
                                        channel_id: c.channel_id,
                                        access_hash: c.access_hash,
                                    },
                                ),
                                photo: chat_photo,
                            };
                            self.client.rpc_write(&req).await?;
                        }
                        tl::enums::InputPeer::Chat(c) => {
                            let req = tl::functions::messages::EditChatPhoto {
                                chat_id: c.chat_id,
                                photo: chat_photo,
                            };
                            self.client.rpc_write(&req).await?;
                        }
                        _ => {}
                    }
                }
            } else if let Some(chat_photo) = self.chat_photo {
                match &input_peer {
                    tl::enums::InputPeer::Channel(c) => {
                        let req = tl::functions::channels::EditPhoto {
                            channel: tl::enums::InputChannel::InputChannel(
                                tl::types::InputChannel {
                                    channel_id: c.channel_id,
                                    access_hash: c.access_hash,
                                },
                            ),
                            photo: chat_photo,
                        };
                        self.client.rpc_write(&req).await?;
                    }
                    tl::enums::InputPeer::Chat(c) => {
                        let req = tl::functions::messages::EditChatPhoto {
                            chat_id: c.chat_id,
                            photo: chat_photo,
                        };
                        self.client.rpc_write(&req).await?;
                    }
                    _ => {}
                }
            }
        } else {
            // user or self

            if self.first_name.is_some() || self.last_name.is_some() || self.bio.is_some() {
                let req = tl::functions::account::UpdateProfile {
                    first_name: self.first_name,
                    last_name: self.last_name,
                    about: self.bio,
                };
                self.client.rpc_write(&req).await?;
            }
            if let Some(u) = self.username {
                let req = tl::functions::account::UpdateUsername { username: u };
                self.client.rpc_write(&req).await?;
            }
            if let Some(file) = self.photo {
                let req = tl::functions::photos::UploadProfilePhoto {
                    fallback: false,
                    bot: None,
                    file: Some(file.inner),
                    video: None,
                    video_start_ts: None,
                    video_emoji_markup: None,
                };
                self.client.rpc_write(&req).await?;
            }
            if let Some((doc_id, until)) = self.emoji_status {
                let emoji_status = match doc_id {
                    None => tl::enums::EmojiStatus::Empty,
                    Some(id) => tl::enums::EmojiStatus::EmojiStatus(tl::types::EmojiStatus {
                        document_id: id,
                        until,
                    }),
                };
                let req = tl::functions::account::UpdateEmojiStatus { emoji_status };
                self.client.rpc_write(&req).await?;
            }
        }

        Ok(())
    }
}
