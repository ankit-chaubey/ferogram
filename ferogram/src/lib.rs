// Copyright (c) Ankit Chaubey <ankitchaubey.dev@gmail.com>
// SPDX-License-Identifier: MIT OR Apache-2.0
//
// ferogram: async Telegram MTProto client in Rust
// https://github.com/ankit-chaubey/ferogram
//
// Based on layer: https://github.com/ankit-chaubey/layer
// Follows official Telegram client behaviour (tdesktop, TDLib).
//
// If you use or modify this code, keep this notice at the top of your file
// and include the LICENSE-MIT or LICENSE-APACHE file from this repository:
// https://github.com/ankit-chaubey/ferogram

#![cfg_attr(docsrs, feature(doc_cfg))]
#![doc(html_root_url = "https://docs.rs/ferogram/0.1.0")]
//! # ferogram
//!
//! Async Telegram client built on MTProto.
//!
//! ## Features
//! - User login (phone + code + 2FA SRP) and bot token login
//! - Peer access-hash caching: API calls always carry correct access hashes
//! - `FLOOD_WAIT` auto-retry with configurable policy
//! - Typed async update stream: `NewMessage`, `MessageEdited`, `MessageDeleted`,
//!   `CallbackQuery`, `InlineQuery`, `InlineSend`, `Raw`
//! - Send / edit / delete / forward / pin messages
//! - Search messages (per-chat and global)
//! - Mark as read, delete dialogs, clear mentions
//! - Join chat / accept invite links
//! - Chat action (typing, uploading, …)
//! - `get_me()`: fetch own User info
//! - Paginated dialog and message iterators
//! - DC migration, session persistence, reconnect

#![deny(unsafe_code)]

pub mod builder;
mod errors;
pub mod media;
pub mod parsers;
pub mod participants;
pub mod pts;
mod restart;
mod retry;
mod session;
mod transport;
mod two_factor_auth;
pub mod update;

pub mod dc_pool;
pub mod inline_iter;
pub mod keyboard;
pub mod search;
pub mod session_backend;
pub mod socks5;
pub mod transport_intermediate;
pub mod transport_obfuscated;
pub mod types;
pub mod typing_guard;

#[macro_use]
pub mod macros;
pub mod peer_ref;
pub mod reactions;

#[cfg(test)]
mod pts_tests;

pub mod dc_migration;
pub mod proxy;

pub use builder::{BuilderError, ClientBuilder};
pub use errors::{InvocationError, LoginToken, PasswordToken, RpcError, SignInError};
pub use keyboard::{Button, InlineKeyboard, ReplyKeyboard};
pub use media::{Document, DownloadIter, Downloadable, Photo, Sticker, UploadedFile};
pub use participants::{Participant, ProfilePhotoIter};
pub use peer_ref::PeerRef;
pub use proxy::{MtProxyConfig, parse_proxy_link};
pub use restart::{ConnectionRestartPolicy, FixedInterval, NeverRestart};
use retry::RetryLoop;
pub use retry::{AutoSleep, NoRetries, RetryContext, RetryPolicy};
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
pub use update::Update;
pub use update::{ChatActionUpdate, UserStatusUpdate};

/// Re-export of `ferogram_tl_types`: generated TL constructors, functions, and enums.
/// Users can write `use ferogram::tl` instead of adding a separate `ferogram-tl-types` dep.
pub use ferogram_tl_types as tl;

use std::collections::HashMap;
use std::collections::VecDeque;
use std::num::NonZeroU32;
use std::ops::ControlFlow;
use std::sync::Arc;
use std::time::Duration;

use ferogram_mtproto::{EncryptedSession, Session, authentication as auth};
use ferogram_tl_types::{Cursor, Deserializable, RemoteCall};
use session::PersistedSession;
use socket2::TcpKeepalive;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio::sync::{Mutex, RwLock, mpsc, oneshot};
use tokio::time::sleep;
use tokio_util::sync::CancellationToken;

const ID_RPC_RESULT: u32 = 0xf35c6d01;
const ID_RPC_ERROR: u32 = 0x2144ca19;
const ID_MSG_CONTAINER: u32 = 0x73f1f8dc;
const ID_GZIP_PACKED: u32 = 0x3072cfa1;
const ID_PONG: u32 = 0x347773c5;
const ID_MSGS_ACK: u32 = 0x62d6b459;
const ID_BAD_SERVER_SALT: u32 = 0xedab447b;
const ID_NEW_SESSION: u32 = 0x9ec20908;
const ID_BAD_MSG_NOTIFY: u32 = 0xa7eff811;
// FutureSalts arrives as a bare frame (not inside rpc_result)
const ID_FUTURE_SALTS: u32 = 0xae500895;
// server confirms our message was received; we must ack its answer_msg_id
const ID_MSG_DETAILED_INFO: u32 = 0x276d3ec6;
const ID_MSG_NEW_DETAIL_INFO: u32 = 0x809db6df;
// server asks us to re-send a specific message
const ID_MSG_RESEND_REQ: u32 = 0x7d861a08;
const ID_UPDATES: u32 = 0x74ae4240;
const ID_UPDATE_SHORT: u32 = 0x78d4dec1;
const ID_UPDATES_COMBINED: u32 = 0x725b04c3;
const ID_UPDATE_SHORT_MSG: u32 = 0x313bc7f8;
const ID_UPDATE_SHORT_CHAT_MSG: u32 = 0x4d6deea5;
const ID_UPDATE_SHORT_SENT_MSG: u32 = 0x9015e101;
const ID_UPDATES_TOO_LONG: u32 = 0xe317af7e;

/// How often to send a keepalive ping.
/// 60 s matches  and Telegram Desktop long-poll cadence.
/// At 1 M connections this is 4× less keepalive traffic than 15 s.
const PING_DELAY_SECS: u64 = 60;

/// Tell Telegram to close the connection if it hears nothing for this many
/// seconds.  Must be > PING_DELAY_SECS so a single missed ping doesn't drop us.
/// 75 s = 60 s interval + 15 s slack, matching .
const NO_PING_DISCONNECT: i32 = 75;

/// Initial backoff before the first reconnect attempt.
const RECONNECT_BASE_MS: u64 = 500;

/// Maximum backoff between reconnect attempts.
/// 5 s cap instead of 30 s: on mobile, network outages are brief and a 30 s
/// sleep means the bot stays dead for up to 30 s after the network returns.
/// Official Telegram mobile clients use a short cap for the same reason.
const RECONNECT_MAX_SECS: u64 = 5;

/// TCP socket-level keepalive: start probes after this many seconds of idle.
const TCP_KEEPALIVE_IDLE_SECS: u64 = 10;
/// Interval between TCP keepalive probes.
const TCP_KEEPALIVE_INTERVAL_SECS: u64 = 5;
/// Number of failed probes before the OS declares the connection dead.
const TCP_KEEPALIVE_PROBES: u32 = 3;

/// Caches access hashes for users and channels so every API call carries the
/// correct hash without re-resolving peers.
///
/// All fields are `pub` so that `save_session` / `connect` can read/write them
/// directly, and so that advanced callers can inspect the cache.
#[derive(Default)]
pub struct PeerCache {
    /// user_id -> access_hash
    pub users: HashMap<i64, i64>,
    /// channel_id -> access_hash
    pub channels: HashMap<i64, i64>,
}

impl PeerCache {
    fn cache_user(&mut self, user: &tl::enums::User) {
        if let tl::enums::User::User(u) = user
            && let Some(hash) = u.access_hash
        {
            self.users.insert(u.id, hash);
        }
    }

    fn cache_chat(&mut self, chat: &tl::enums::Chat) {
        match chat {
            tl::enums::Chat::Channel(c) => {
                if let Some(hash) = c.access_hash {
                    self.channels.insert(c.id, hash);
                }
            }
            tl::enums::Chat::ChannelForbidden(c) => {
                self.channels.insert(c.id, c.access_hash);
            }
            _ => {}
        }
    }

    fn cache_users(&mut self, users: &[tl::enums::User]) {
        for u in users {
            self.cache_user(u);
        }
    }

    fn cache_chats(&mut self, chats: &[tl::enums::Chat]) {
        for c in chats {
            self.cache_chat(c);
        }
    }

    fn user_input_peer(&self, user_id: i64) -> tl::enums::InputPeer {
        if user_id == 0 {
            return tl::enums::InputPeer::PeerSelf;
        }
        let hash = self.users.get(&user_id).copied().unwrap_or_else(|| {
            tracing::warn!("[ferogram] PeerCache: no access_hash for user {user_id}, using 0: may cause USER_ID_INVALID");
            0
        });
        tl::enums::InputPeer::User(tl::types::InputPeerUser {
            user_id,
            access_hash: hash,
        })
    }

    fn channel_input_peer(&self, channel_id: i64) -> tl::enums::InputPeer {
        let hash = self.channels.get(&channel_id).copied().unwrap_or_else(|| {
            tracing::warn!("[ferogram] PeerCache: no access_hash for channel {channel_id}, using 0: may cause CHANNEL_INVALID");
            0
        });
        tl::enums::InputPeer::Channel(tl::types::InputPeerChannel {
            channel_id,
            access_hash: hash,
        })
    }

    fn peer_to_input(&self, peer: &tl::enums::Peer) -> tl::enums::InputPeer {
        match peer {
            tl::enums::Peer::User(u) => self.user_input_peer(u.user_id),
            tl::enums::Peer::Chat(c) => {
                tl::enums::InputPeer::Chat(tl::types::InputPeerChat { chat_id: c.chat_id })
            }
            tl::enums::Peer::Channel(c) => self.channel_input_peer(c.channel_id),
        }
    }
}

/// Builder for composing outgoing messages.
///
/// ```rust,no_run
/// use ferogram::InputMessage;
///
/// // plain text
/// let msg = InputMessage::text("Hello!");
///
/// // markdown
/// let msg = InputMessage::markdown("**bold** and _italic_");
///
/// // HTML
/// let msg = InputMessage::html("<b>bold</b> and <i>italic</i>");
///
/// // with options
/// let msg = InputMessage::markdown("**Hello**")
///     .silent(true)
///     .reply_to(Some(42));
/// ```
#[derive(Clone, Default)]
pub struct InputMessage {
    pub text: String,
    pub reply_to: Option<i32>,
    pub silent: bool,
    pub background: bool,
    pub clear_draft: bool,
    pub no_webpage: bool,
    /// Show media above the caption instead of below (Telegram ≥ 10.3).\
    pub invert_media: bool,
    /// Schedule to send when the user goes online (`schedule_date = 0x7FFFFFFE`).\
    pub schedule_once_online: bool,
    pub entities: Option<Vec<tl::enums::MessageEntity>>,
    pub reply_markup: Option<tl::enums::ReplyMarkup>,
    pub schedule_date: Option<i32>,
    /// Attached media to send alongside the message.
    /// Use [`InputMessage::copy_media`] to attach media copied from an existing message.
    pub media: Option<tl::enums::InputMedia>,
}

impl InputMessage {
    /// Create a message with the given text.
    pub fn text(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            ..Default::default()
        }
    }

    /// Create a message by parsing Telegram-flavoured markdown.
    ///
    /// The markdown is stripped and the resulting plain text + entities are
    /// set on the message. Supports `**bold**`, `_italic_`, `` `code` ``,
    /// `[text](url)`, `||spoiler||`, `~~strike~~`, `![text](tg://emoji?id=...)`,
    /// and backslash escapes.
    ///
    /// ```rust,no_run
    /// use ferogram::InputMessage;
    ///
    /// let msg = InputMessage::markdown("**Hello** _world_!");
    /// ```
    pub fn markdown(text: impl AsRef<str>) -> Self {
        let (plain, ents) = crate::parsers::parse_markdown(text.as_ref());
        Self {
            text: plain,
            entities: if ents.is_empty() { None } else { Some(ents) },
            ..Default::default()
        }
    }

    /// Create a message by parsing Telegram-compatible HTML.
    ///
    /// Supports `<b>`, `<i>`, `<u>`, `<s>`, `<code>`, `<pre>`,
    /// `<tg-spoiler>`, `<a href="...">`, `<tg-emoji emoji-id="...">`.
    ///
    /// ```rust,no_run
    /// use ferogram::InputMessage;
    ///
    /// let msg = InputMessage::html("<b>Hello</b> <i>world</i>!");
    /// ```
    pub fn html(text: impl AsRef<str>) -> Self {
        let (plain, ents) = crate::parsers::parse_html(text.as_ref());
        Self {
            text: plain,
            entities: if ents.is_empty() { None } else { Some(ents) },
            ..Default::default()
        }
    }

    /// Set the message text.
    pub fn set_text(mut self, text: impl Into<String>) -> Self {
        self.text = text.into();
        self
    }

    /// Reply to a specific message ID.
    pub fn reply_to(mut self, id: Option<i32>) -> Self {
        self.reply_to = id;
        self
    }

    /// Send silently (no notification sound).
    pub fn silent(mut self, v: bool) -> Self {
        self.silent = v;
        self
    }

    /// Send in background.
    pub fn background(mut self, v: bool) -> Self {
        self.background = v;
        self
    }

    /// Clear the draft after sending.
    pub fn clear_draft(mut self, v: bool) -> Self {
        self.clear_draft = v;
        self
    }

    /// Disable link preview.
    pub fn no_webpage(mut self, v: bool) -> Self {
        self.no_webpage = v;
        self
    }

    /// Show media above the caption rather than below (requires Telegram ≥ 10.3).
    pub fn invert_media(mut self, v: bool) -> Self {
        self.invert_media = v;
        self
    }

    /// Schedule the message to be sent when the recipient comes online.
    ///
    /// Mutually exclusive with `schedule_date`: calling this last wins.
    /// Uses the Telegram magic value `0x7FFFFFFE`.
    pub fn schedule_once_online(mut self) -> Self {
        self.schedule_once_online = true;
        self.schedule_date = None;
        self
    }

    /// Attach formatting entities (bold, italic, code, links, etc).
    pub fn entities(mut self, e: Vec<tl::enums::MessageEntity>) -> Self {
        self.entities = Some(e);
        self
    }

    /// Attach a reply markup (inline or reply keyboard).
    pub fn reply_markup(mut self, rm: tl::enums::ReplyMarkup) -> Self {
        self.reply_markup = Some(rm);
        self
    }

    /// Attach an [`crate::keyboard::InlineKeyboard`].
    ///
    /// ```rust,no_run
    /// use ferogram::{InputMessage, keyboard::{InlineKeyboard, Button}};
    ///
    /// let msg = InputMessage::text("Pick one:")
    /// .keyboard(InlineKeyboard::new()
    ///     .row([Button::callback("A", b"a"), Button::callback("B", b"b")]));
    /// ```
    pub fn keyboard(mut self, kb: impl Into<tl::enums::ReplyMarkup>) -> Self {
        self.reply_markup = Some(kb.into());
        self
    }

    /// Schedule the message for a future Unix timestamp.
    pub fn schedule_date(mut self, ts: Option<i32>) -> Self {
        self.schedule_date = ts;
        self
    }

    /// Attach media copied from an existing message.
    ///
    /// Pass the `InputMedia` obtained from [`crate::media::Photo`],
    /// [`crate::media::Document`], or directly from a raw `MessageMedia`.
    ///
    /// When a `media` is set, the message is sent via `messages.SendMedia`
    /// instead of `messages.SendMessage`.
    ///
    /// ```rust,no_run
    /// # use ferogram::{InputMessage, tl};
    /// # fn example(media: tl::enums::InputMedia) {
    /// let msg = InputMessage::text("Here is the file again")
    /// .copy_media(media);
    /// # }
    /// ```
    pub fn copy_media(mut self, media: tl::enums::InputMedia) -> Self {
        self.media = Some(media);
        self
    }

    /// Remove any previously attached media.
    pub fn clear_media(mut self) -> Self {
        self.media = None;
        self
    }

    fn reply_header(&self) -> Option<tl::enums::InputReplyTo> {
        self.reply_to.map(|id| {
            tl::enums::InputReplyTo::Message(tl::types::InputReplyToMessage {
                reply_to_msg_id: id,
                top_msg_id: None,
                reply_to_peer_id: None,
                quote_text: None,
                quote_entities: None,
                quote_offset: None,
                monoforum_peer_id: None,
                todo_item_id: None,
                poll_option: None,
            })
        })
    }
}

impl From<&str> for InputMessage {
    fn from(s: &str) -> Self {
        Self::text(s)
    }
}

impl From<String> for InputMessage {
    fn from(s: String) -> Self {
        Self::text(s)
    }
}

/// Which MTProto transport framing to use for all connections.
///
/// | Variant | Init bytes | Notes |
/// |---------|-----------|-------|
/// | `Abridged` | `0xef` | Smallest overhead |
/// | `Intermediate` | `0xeeeeeeee` | Better proxy compat |
/// | `Full` | none | Adds seqno + CRC32 |
/// | `Obfuscated` | random 64B | Bypasses DPI / MTProxy: **default** |
/// | `PaddedIntermediate` | random 64B (`0xDDDDDDDD` tag) | Obfuscated padded intermediate required for `0xDD` MTProxy secrets |
/// | `FakeTls` | TLS 1.3 ClientHello | Most DPI-resistant; required for `0xEE` MTProxy secrets |
#[derive(Clone, Debug)]
pub enum TransportKind {
    /// MTProto [Abridged] transport: length prefix is 1 or 4 bytes.
    ///
    /// [Abridged]: https://core.telegram.org/mtproto/mtproto-transports#abridged
    Abridged,
    /// MTProto [Intermediate] transport: 4-byte LE length prefix.
    ///
    /// [Intermediate]: https://core.telegram.org/mtproto/mtproto-transports#intermediate
    Intermediate,
    /// MTProto [Full] transport: 4-byte length + seqno + CRC32.
    ///
    /// [Full]: https://core.telegram.org/mtproto/mtproto-transports#full
    Full,
    /// [Obfuscated2] transport: AES-256-CTR over Abridged framing.
    /// Required for MTProxy and networks with deep-packet inspection.
    /// **Default**: works on all networks, bypasses DPI, negligible CPU cost.
    ///
    /// `secret` is the 16-byte MTProxy secret, or `None` for keyless obfuscation.
    ///
    /// [Obfuscated2]: https://core.telegram.org/mtproto/mtproto-transports#obfuscated-2
    Obfuscated { secret: Option<[u8; 16]> },
    /// Obfuscated PaddedIntermediate transport (`0xDDDDDDDD` tag in nonce).
    ///
    /// Same AES-256-CTR obfuscation as `Obfuscated`, but uses Intermediate
    /// framing and appends 0–15 random padding bytes to each frame so that
    /// all frames are not 4-byte multiples.  Required for `0xDD` MTProxy secrets.
    PaddedIntermediate { secret: Option<[u8; 16]> },
    /// FakeTLS transport (`0xEE` prefix in MTProxy secret).
    ///
    /// Wraps all MTProto data in fake TLS 1.3 records.  The ClientHello
    /// embeds an HMAC-SHA256 digest of the secret so the MTProxy server
    /// can validate ownership without decrypting real TLS.  Most DPI-resistant
    /// mode; required for `0xEE` MTProxy secrets.
    FakeTls { secret: [u8; 16], domain: String },
}

impl Default for TransportKind {
    fn default() -> Self {
        // Obfuscated (keyless) is the best all-round choice:
        //  - bypasses DPI / ISP blocks that filter plain MTProto
        //  - negligible CPU overhead (AES-256-CTR via hardware AES-NI)
        //  - works on all networks without MTProxy configuration
        //  - drops in as a replacement for Abridged with zero API changes
        TransportKind::Obfuscated { secret: None }
    }
}

/// A token that can be used to gracefully shut down a [`Client`].
///
/// Obtained from [`Client::connect`]: call [`ShutdownToken::cancel`] to begin
/// graceful shutdown. All pending requests will finish and the reader task will
/// exit cleanly.
///
/// # Example
/// ```rust,no_run
/// # async fn f() -> Result<(), Box<dyn std::error::Error>> {
/// use ferogram::{Client, Config, ShutdownToken};
///
/// let (client, shutdown) = Client::connect(Config::default()).await?;
///
/// // In a signal handler or background task:
/// // shutdown.cancel();
/// # Ok(()) }
/// ```
pub type ShutdownToken = CancellationToken;

/// Configuration for [`Client::connect`].
#[derive(Clone)]
pub struct Config {
    pub api_id: i32,
    pub api_hash: String,
    pub dc_addr: Option<String>,
    pub retry_policy: Arc<dyn RetryPolicy>,
    /// Optional SOCKS5 proxy: every Telegram connection is tunnelled through it.
    pub socks5: Option<crate::socks5::Socks5Config>,
    /// Optional MTProxy: if set, all TCP connections go to the proxy host:port
    /// instead of the Telegram DC address.  The `transport` field is overridden
    /// by `mtproxy.transport` automatically.
    pub mtproxy: Option<crate::proxy::MtProxyConfig>,
    /// Allow IPv6 DC addresses when populating the DC table (default: false).
    pub allow_ipv6: bool,
    /// Which MTProto transport framing to use (default: Abridged).
    pub transport: TransportKind,
    /// Session persistence backend (default: binary file `"ferogram.session"`).
    pub session_backend: Arc<dyn crate::session_backend::SessionBackend>,
    /// If `true`, replay missed updates via `updates.getDifference` immediately
    /// after connecting.
    /// Default: `false`.
    pub catch_up: bool,
    pub restart_policy: Arc<dyn ConnectionRestartPolicy>,
    /// Device model reported in `InitConnection` (default: `"Linux"`).
    pub device_model: String,
    /// System/OS version reported in `InitConnection` (default: `"1.0"`).
    pub system_version: String,
    /// App version reported in `InitConnection` (default: crate version).
    pub app_version: String,
    /// System language code reported in `InitConnection` (default: `"en"`).
    pub system_lang_code: String,
    /// Language pack name reported in `InitConnection` (default: `""`).
    pub lang_pack: String,
    /// Language code reported in `InitConnection` (default: `"en"`).
    pub lang_code: String,
}

impl Config {
    /// Convenience builder: use a portable base64 string session.
    ///
    /// Pass the string exported from a previous `client.export_session_string()` call,
    /// or an empty string to start fresh (the string session will be populated after auth).
    ///
    /// # Example
    /// ```rust,no_run
    /// let cfg = Config {
    /// api_id:   12345,
    /// api_hash: "abc".into(),
    /// catch_up: true,
    /// ..Config::with_string_session(std::env::var("SESSION").unwrap_or_default())
    /// };
    /// ```
    pub fn with_string_session(s: impl Into<String>) -> Self {
        Config {
            session_backend: Arc::new(crate::session_backend::StringSessionBackend::new(s)),
            ..Config::default()
        }
    }

    /// Set an MTProxy from a `https://t.me/proxy?...` or `tg://proxy?...` link.
    ///
    /// Empty string is a no-op; proxy stays unset. Invalid link panics.
    /// Transport is selected from the secret prefix:
    /// plain hex = Obfuscated, `dd` prefix = PaddedIntermediate, `ee` prefix = FakeTLS.
    ///
    /// # Example
    /// ```rust,no_run
    /// use ferogram::Config;
    /// const PROXY: &str = "https://t.me/proxy?server=HOST&port=443&secret=dd...";
    ///
    /// let cfg = Config {
    ///     api_id:   12345,
    ///     api_hash: "abc".into(),
    ///     ..Config::default().proxy_link(PROXY)
    /// };
    /// ```
    pub fn proxy_link(mut self, url: &str) -> Self {
        if url.is_empty() {
            return self;
        }
        let cfg = crate::proxy::parse_proxy_link(url)
            .unwrap_or_else(|| panic!("invalid MTProxy link: {url:?}"));
        self.mtproxy = Some(cfg);
        self
    }

    /// Set an MTProxy from raw fields: `host`, `port`, and `secret` (hex or base64).
    ///
    /// Secret decoding: 32+ hex chars are parsed as hex bytes, anything else as URL-safe base64.
    /// Transport is selected from the secret prefix, same as `proxy_link`.
    ///
    /// # Example
    /// ```rust,no_run
    /// use ferogram::Config;
    ///
    /// let cfg = Config {
    ///     api_id:   12345,
    ///     api_hash: "abc".into(),
    ///     // dd prefix = PaddedIntermediate, ee prefix = FakeTLS, plain = Obfuscated
    ///     ..Config::default().proxy("proxy.example.com", 443, "ee0000000000000000000000000000000000706578616d706c652e636f6d")
    /// };
    /// ```
    pub fn proxy(self, host: impl Into<String>, port: u16, secret: &str) -> Self {
        let host = host.into();
        let url = format!("tg://proxy?server={host}&port={port}&secret={secret}");
        self.proxy_link(&url)
    }

    /// Set a SOCKS5 proxy (no authentication).
    ///
    /// # Example
    /// ```rust,no_run
    /// use ferogram::Config;
    ///
    /// let cfg = Config {
    ///     api_id:   12345,
    ///     api_hash: "abc".into(),
    ///     ..Config::default().socks5("127.0.0.1:1080")
    /// };
    /// ```
    pub fn socks5(mut self, addr: impl Into<String>) -> Self {
        self.socks5 = Some(crate::socks5::Socks5Config::new(addr));
        self
    }

    /// Set a SOCKS5 proxy with username/password authentication.
    ///
    /// # Example
    /// ```rust,no_run
    /// use ferogram::Config;
    ///
    /// let cfg = Config {
    ///     api_id:   12345,
    ///     api_hash: "abc".into(),
    ///     ..Config::default().socks5_auth("proxy.example.com:1080", "user", "pass")
    /// };
    /// ```
    pub fn socks5_auth(
        mut self,
        addr: impl Into<String>,
        username: impl Into<String>,
        password: impl Into<String>,
    ) -> Self {
        self.socks5 = Some(crate::socks5::Socks5Config::with_auth(
            addr, username, password,
        ));
        self
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            api_id: 0,
            api_hash: String::new(),
            dc_addr: None,
            retry_policy: Arc::new(AutoSleep::default()),
            socks5: None,
            mtproxy: None,
            allow_ipv6: false,
            transport: TransportKind::Obfuscated { secret: None },
            session_backend: Arc::new(crate::session_backend::BinaryFileBackend::new(
                "ferogram.session",
            )),
            catch_up: false,
            restart_policy: Arc::new(NeverRestart),
            device_model: "Linux".to_string(),
            system_version: "1.0".to_string(),
            app_version: env!("CARGO_PKG_VERSION").to_string(),
            system_lang_code: "en".to_string(),
            lang_pack: String::new(),
            lang_code: "en".to_string(),
        }
    }
}

// UpdateStream
// UpdateStream lives here; next_raw() added.

/// Asynchronous stream of [`Update`]s.
pub struct UpdateStream {
    rx: mpsc::UnboundedReceiver<update::Update>,
}

impl UpdateStream {
    /// Wait for the next update. Returns `None` when the client has disconnected.
    pub async fn next(&mut self) -> Option<update::Update> {
        self.rx.recv().await
    }

    /// Wait for the next **raw** (unrecognised) update frame, skipping all
    /// typed high-level variants. Useful for handling constructor IDs that
    /// `ferogram` does not yet wrap: dispatch on `constructor_id` yourself.
    ///
    /// Returns `None` when the client has disconnected.
    pub async fn next_raw(&mut self) -> Option<update::RawUpdate> {
        loop {
            match self.rx.recv().await? {
                update::Update::Raw(r) => return Some(r),
                _ => continue,
            }
        }
    }
}

// Dialog

/// A Telegram dialog (chat, user, channel).
#[derive(Debug, Clone)]
pub struct Dialog {
    pub raw: tl::enums::Dialog,
    pub message: Option<tl::enums::Message>,
    pub entity: Option<tl::enums::User>,
    pub chat: Option<tl::enums::Chat>,
}

impl Dialog {
    /// The dialog's display title.
    pub fn title(&self) -> String {
        if let Some(tl::enums::User::User(u)) = &self.entity {
            let first = u.first_name.as_deref().unwrap_or("");
            let last = u.last_name.as_deref().unwrap_or("");
            let name = format!("{first} {last}").trim().to_string();
            if !name.is_empty() {
                return name;
            }
        }
        if let Some(chat) = &self.chat {
            return match chat {
                tl::enums::Chat::Chat(c) => c.title.clone(),
                tl::enums::Chat::Forbidden(c) => c.title.clone(),
                tl::enums::Chat::Channel(c) => c.title.clone(),
                tl::enums::Chat::ChannelForbidden(c) => c.title.clone(),
                tl::enums::Chat::Empty(_) => "(empty)".into(),
            };
        }
        "(Unknown)".to_string()
    }

    /// Peer of this dialog.
    pub fn peer(&self) -> Option<&tl::enums::Peer> {
        match &self.raw {
            tl::enums::Dialog::Dialog(d) => Some(&d.peer),
            tl::enums::Dialog::Folder(_) => None,
        }
    }

    /// Unread message count.
    pub fn unread_count(&self) -> i32 {
        match &self.raw {
            tl::enums::Dialog::Dialog(d) => d.unread_count,
            _ => 0,
        }
    }

    /// ID of the top message.
    pub fn top_message(&self) -> i32 {
        match &self.raw {
            tl::enums::Dialog::Dialog(d) => d.top_message,
            _ => 0,
        }
    }
}

// ClientInner

struct ClientInner {
    /// Crypto/state for the connection: EncryptedSession, salts, acks, etc.
    /// Held only for CPU-bound packing : never while awaiting TCP I/O.
    writer: Mutex<ConnectionWriter>,
    /// The TCP send half. Separate from `writer` so the reader task can lock
    /// `writer` for pending_ack / state while a caller awaits `write_all`.
    /// This split eliminates the burst-deadlock at 10+ concurrent RPCs.
    write_half: Mutex<OwnedWriteHalf>,
    /// Pending RPC replies, keyed by MTProto msg_id.
    /// RPC callers insert a oneshot::Sender here before sending; the reader
    /// task routes incoming rpc_result frames to the matching sender.
    #[allow(clippy::type_complexity)]
    pending: Arc<Mutex<HashMap<i64, oneshot::Sender<Result<Vec<u8>, InvocationError>>>>>,
    /// Channel used to hand a new (OwnedReadHalf, FrameKind, auth_key, session_id)
    /// to the reader task after a reconnect.
    reconnect_tx: mpsc::UnboundedSender<(OwnedReadHalf, FrameKind, [u8; 256], i64)>,
    /// Send `()` here to wake the reader's reconnect backoff loop immediately.
    /// Used by [`Client::signal_network_restored`].
    network_hint_tx: mpsc::UnboundedSender<()>,
    /// Cancelled to signal graceful shutdown to the reader task.
    #[allow(dead_code)]
    shutdown_token: CancellationToken,
    /// Whether to replay missed updates via getDifference on connect.
    #[allow(dead_code)]
    catch_up: bool,
    restart_policy: Arc<dyn ConnectionRestartPolicy>,
    home_dc_id: Mutex<i32>,
    dc_options: Mutex<HashMap<i32, DcEntry>>,
    /// Media-only DC options (ipv6/media_only/cdn filtered separately from API DCs).
    media_dc_options: Mutex<HashMap<i32, DcEntry>>,
    pub peer_cache: RwLock<PeerCache>,
    pub pts_state: Mutex<pts::PtsState>,
    /// Buffer for updates received during a possible-gap window.
    pub possible_gap: Mutex<pts::PossibleGapBuffer>,
    api_id: i32,
    api_hash: String,
    device_model: String,
    system_version: String,
    app_version: String,
    system_lang_code: String,
    lang_pack: String,
    lang_code: String,
    retry_policy: Arc<dyn RetryPolicy>,
    socks5: Option<crate::socks5::Socks5Config>,
    mtproxy: Option<crate::proxy::MtProxyConfig>,
    allow_ipv6: bool,
    transport: TransportKind,
    session_backend: Arc<dyn crate::session_backend::SessionBackend>,
    dc_pool: Mutex<dc_pool::DcPool>,
    update_tx: mpsc::Sender<update::Update>,
    /// Whether this client is signed in as a bot (set in `bot_sign_in`).
    /// Used by `get_channel_difference` to pick the correct diff limit:
    /// bots get 100_000 (BOT_CHANNEL_DIFF_LIMIT), users get 100 (USER_CHANNEL_DIFF_LIMIT).
    pub is_bot: std::sync::atomic::AtomicBool,
    /// Guards against calling `stream_updates()` more than once.
    stream_active: std::sync::atomic::AtomicBool,
    /// Prevents spawning more than one proactive GetFutureSalts at a time.
    /// Without this guard every bad_server_salt spawns a new task, which causes
    /// an exponential storm when many messages are queued with a stale salt.
    salt_request_in_flight: std::sync::atomic::AtomicBool,
    /// Prevents two concurrent fresh-DH handshakes racing each other.
    /// A double-DH results in one key being unregistered on Telegram's servers,
    /// causing AUTH_KEY_UNREGISTERED immediately after reconnect.
    dh_in_progress: std::sync::atomic::AtomicBool,
}

/// The main Telegram client. Cheap to clone: internally Arc-wrapped.
#[derive(Clone)]
pub struct Client {
    pub(crate) inner: Arc<ClientInner>,
    _update_rx: Arc<Mutex<mpsc::Receiver<update::Update>>>,
}

impl Client {
    /// Return a fluent [`ClientBuilder`] for constructing and connecting a client.
    ///
    /// # Example
    /// ```rust,no_run
    /// # use ferogram::Client;
    /// # #[tokio::main] async fn main() -> anyhow::Result<()> {
    /// let (client, _shutdown) = Client::builder()
    /// .api_id(12345)
    /// .api_hash("abc123")
    /// .session("my.session")
    /// .catch_up(true)
    /// .connect().await?;
    /// # Ok(()) }
    /// ```
    pub fn builder() -> crate::builder::ClientBuilder {
        crate::builder::ClientBuilder::default()
    }

    // Connect

    pub async fn connect(config: Config) -> Result<(Self, ShutdownToken), InvocationError> {
        // Validate required config fields up-front with clear error messages.
        if config.api_id == 0 {
            return Err(InvocationError::Deserialize(
                "api_id must be non-zero".into(),
            ));
        }
        if config.api_hash.is_empty() {
            return Err(InvocationError::Deserialize(
                "api_hash must not be empty".into(),
            ));
        }

        // Capacity: 2048 updates. If the consumer falls behind, excess updates
        // are dropped with a warning rather than growing RAM without bound.
        let (update_tx, update_rx) = mpsc::channel(2048);

        // Load or fresh-connect
        let socks5 = config.socks5.clone();
        let mtproxy = config.mtproxy.clone();
        let transport = config.transport.clone();

        let (conn, home_dc_id, dc_opts, media_dc_opts, loaded_session) =
            match config.session_backend.load().map_err(InvocationError::Io)? {
                Some(s) => {
                    if let Some(dc) = s.dcs.iter().find(|d| d.dc_id == s.home_dc_id) {
                        if let Some(key) = dc.auth_key {
                            tracing::info!("[ferogram] Loading session (DC{}) …", s.home_dc_id);
                            match Connection::connect_with_key(
                                &dc.addr,
                                key,
                                dc.first_salt,
                                dc.time_offset,
                                socks5.as_ref(),
                                &transport,
                                s.home_dc_id as i16,
                            )
                            .await
                            {
                                Ok(c) => {
                                    let mut opts = session::default_dc_addresses()
                                        .into_iter()
                                        .map(|(id, addr)| {
                                            (
                                                id,
                                                DcEntry {
                                                    dc_id: id,
                                                    addr,
                                                    auth_key: None,
                                                    first_salt: 0,
                                                    time_offset: 0,
                                                    flags: DcFlags::NONE,
                                                },
                                            )
                                        })
                                        .collect::<HashMap<_, _>>();
                                    let mut media_opts: HashMap<i32, DcEntry> = HashMap::new();
                                    for d in &s.dcs {
                                        if d.flags.contains(DcFlags::MEDIA_ONLY)
                                            || d.flags.contains(DcFlags::CDN)
                                        {
                                            media_opts.insert(d.dc_id, d.clone());
                                        } else {
                                            opts.insert(d.dc_id, d.clone());
                                        }
                                    }
                                    (c, s.home_dc_id, opts, media_opts, Some(s))
                                }
                                Err(e) => {
                                    // never call fresh_connect on a TCP blip during
                                    // startup: that would silently destroy the saved session
                                    // by switching to DC2 with a fresh key.  Return the error
                                    // so the caller gets a clear failure and can retry or
                                    // prompt for re-auth without corrupting the session file.
                                    tracing::warn!(
                                        "[ferogram] Session connect failed ({e}): \
                                         returning error (delete session file to reset)"
                                    );
                                    return Err(e);
                                }
                            }
                        } else {
                            let (c, dc, opts) =
                                Self::fresh_connect(socks5.as_ref(), mtproxy.as_ref(), &transport)
                                    .await?;
                            (c, dc, opts, HashMap::new(), None)
                        }
                    } else {
                        let (c, dc, opts) =
                            Self::fresh_connect(socks5.as_ref(), mtproxy.as_ref(), &transport)
                                .await?;
                        (c, dc, opts, HashMap::new(), None)
                    }
                }
                None => {
                    let (c, dc, opts) =
                        Self::fresh_connect(socks5.as_ref(), mtproxy.as_ref(), &transport).await?;
                    (c, dc, opts, HashMap::new(), None)
                }
            };

        // Build DC pool
        let pool = dc_pool::DcPool::new(home_dc_id, &dc_opts.values().cloned().collect::<Vec<_>>());

        // Split the TCP stream immediately.
        // The writer (write half + EncryptedSession) stays in ClientInner.
        // The read half goes to the reader task which we spawn right now so
        // that RPC calls during init_connection work correctly.
        let (writer, write_half, read_half, frame_kind) = conn.into_writer();
        let auth_key = writer.enc.auth_key_bytes();
        let session_id = writer.enc.session_id();

        #[allow(clippy::type_complexity)]
        let pending: Arc<
            Mutex<HashMap<i64, oneshot::Sender<Result<Vec<u8>, InvocationError>>>>,
        > = Arc::new(Mutex::new(HashMap::new()));

        // Channel the reconnect logic uses to hand a new read half to the reader task.
        let (reconnect_tx, reconnect_rx) =
            mpsc::unbounded_channel::<(OwnedReadHalf, FrameKind, [u8; 256], i64)>();

        // Channel for external "network restored" hints: lets Android/iOS callbacks
        // skip the reconnect backoff and attempt immediately.
        let (network_hint_tx, network_hint_rx) = mpsc::unbounded_channel::<()>();

        // Graceful shutdown token: cancel this to stop the reader task cleanly.
        let shutdown_token = CancellationToken::new();
        let catch_up = config.catch_up;
        let restart_policy = config.restart_policy;

        let inner = Arc::new(ClientInner {
            writer: Mutex::new(writer),
            write_half: Mutex::new(write_half),
            pending: pending.clone(),
            reconnect_tx,
            network_hint_tx,
            shutdown_token: shutdown_token.clone(),
            catch_up,
            restart_policy,
            home_dc_id: Mutex::new(home_dc_id),
            dc_options: Mutex::new(dc_opts),
            media_dc_options: Mutex::new(media_dc_opts),
            peer_cache: RwLock::new(PeerCache::default()),
            pts_state: Mutex::new(pts::PtsState::default()),
            possible_gap: Mutex::new(pts::PossibleGapBuffer::new()),
            api_id: config.api_id,
            api_hash: config.api_hash,
            device_model: config.device_model,
            system_version: config.system_version,
            app_version: config.app_version,
            system_lang_code: config.system_lang_code,
            lang_pack: config.lang_pack,
            lang_code: config.lang_code,
            retry_policy: config.retry_policy,
            socks5: config.socks5,
            mtproxy: config.mtproxy,
            allow_ipv6: config.allow_ipv6,
            transport: config.transport,
            session_backend: config.session_backend,
            dc_pool: Mutex::new(pool),
            update_tx,
            is_bot: std::sync::atomic::AtomicBool::new(false),
            stream_active: std::sync::atomic::AtomicBool::new(false),
            salt_request_in_flight: std::sync::atomic::AtomicBool::new(false),
            dh_in_progress: std::sync::atomic::AtomicBool::new(false),
        });

        let client = Self {
            inner,
            _update_rx: Arc::new(Mutex::new(update_rx)),
        };

        // Spawn the reader task immediately so that RPC calls during
        // init_connection can receive their responses.
        {
            let client_r = client.clone();
            let shutdown_r = shutdown_token.clone();
            tokio::spawn(async move {
                client_r
                    .run_reader_task(
                        read_half,
                        frame_kind,
                        auth_key,
                        session_id,
                        reconnect_rx,
                        network_hint_rx,
                        shutdown_r,
                    )
                    .await;
            });
        }

        // Only clear the auth key on definitive bad-key signals from Telegram.
        // Network errors (EOF mid-session, ConnectionReset, Rpc(-404)) mean the
        // server rejected our key. Any other error (I/O, etc.) is left intact
        // no RPC timeout exists anymore, so there is no "timed out = stale key" case.
        if let Err(e) = client.init_connection().await {
            let key_is_stale = match &e {
                InvocationError::Rpc(r) if r.code == -404 => true,
                // -429 = TRANSPORT_FLOOD (rate limit). The auth key is valid
                // Telegram is just throttling us. Do NOT do fresh DH here; it
                // would race with the reader task's error handler and produce two
                // concurrent DH handshakes whose keys clobber each other, leading
                // to AUTH_KEY_UNREGISTERED on every post-reconnect RPC call.
                InvocationError::Rpc(r) if r.code == -429 => false,
                InvocationError::Io(io)
                    if io.kind() == std::io::ErrorKind::UnexpectedEof
                        || io.kind() == std::io::ErrorKind::ConnectionReset =>
                {
                    true
                }
                _ => false,
            };

            // Concurrency guard: only one fresh-DH handshake at a time.
            // If the reader task already started DH (e.g. it also got a -404
            // from the same burst), skip this code path and let that one finish.
            let dh_allowed = key_is_stale
                && client
                    .inner
                    .dh_in_progress
                    .compare_exchange(
                        false,
                        true,
                        std::sync::atomic::Ordering::SeqCst,
                        std::sync::atomic::Ordering::SeqCst,
                    )
                    .is_ok();

            if dh_allowed {
                tracing::warn!("[ferogram] init_connection: definitive bad-key ({e}), fresh DH …");
                {
                    let home_dc_id = *client.inner.home_dc_id.lock().await;
                    let mut opts = client.inner.dc_options.lock().await;
                    if let Some(entry) = opts.get_mut(&home_dc_id)
                        && entry.auth_key.is_some()
                    {
                        tracing::warn!("[ferogram] Clearing stale auth key for DC{home_dc_id}");
                        entry.auth_key = None;
                        entry.first_salt = 0;
                        entry.time_offset = 0;
                    }
                }
                client.save_session().await.ok();
                client.inner.pending.lock().await.clear();

                let socks5_r = client.inner.socks5.clone();
                let mtproxy_r = client.inner.mtproxy.clone();
                let transport_r = client.inner.transport.clone();

                // reconnect to the HOME DC with fresh DH, not DC2.
                // fresh_connect() was hardcoded to DC2 and wiped all learned DC state,
                // which is why sessions on DC3/DC4/DC5 were corrupted on every -404.
                let home_dc_id_r = *client.inner.home_dc_id.lock().await;
                let addr_r = {
                    let opts = client.inner.dc_options.lock().await;
                    opts.get(&home_dc_id_r)
                        .map(|e| e.addr.clone())
                        .unwrap_or_else(|| {
                            crate::dc_migration::fallback_dc_addr(home_dc_id_r).to_string()
                        })
                };
                let new_conn = Connection::connect_raw(
                    &addr_r,
                    socks5_r.as_ref(),
                    mtproxy_r.as_ref(),
                    &transport_r,
                    home_dc_id_r as i16,
                )
                .await?;

                // Split first so we can read the new key/salt from the writer.
                let (new_writer, new_wh, new_read, new_fk) = new_conn.into_writer();
                // Update ONLY the home DC entry: all other DC keys are preserved.
                {
                    let mut opts_guard = client.inner.dc_options.lock().await;
                    if let Some(entry) = opts_guard.get_mut(&home_dc_id_r) {
                        entry.auth_key = Some(new_writer.auth_key_bytes());
                        entry.first_salt = new_writer.first_salt();
                        entry.time_offset = new_writer.time_offset();
                    }
                }
                // home_dc_id stays unchanged: we reconnected to the same DC.
                let new_ak = new_writer.enc.auth_key_bytes();
                let new_sid = new_writer.enc.session_id();
                *client.inner.writer.lock().await = new_writer;
                *client.inner.write_half.lock().await = new_wh;
                let _ = client
                    .inner
                    .reconnect_tx
                    .send((new_read, new_fk, new_ak, new_sid));
                tokio::task::yield_now().await;

                // Brief pause so the new key propagates to all of Telegram's
                // app servers before we send getDifference (same reason
                // does a yield after fresh DH before any RPCs).
                tokio::time::sleep(std::time::Duration::from_secs(2)).await;

                client.init_connection().await?;
                client
                    .inner
                    .dh_in_progress
                    .store(false, std::sync::atomic::Ordering::SeqCst);
                // Persist the new auth key so next startup loads the correct key.
                client.save_session().await.ok();

                tracing::warn!(
                    "[ferogram] Session invalidated and reset. \
                     Call is_authorized() and re-authenticate if needed."
                );
            } else {
                return Err(e);
            }
        }

        // Restore peer access-hash cache from session
        if let Some(ref s) = loaded_session
            && !s.peers.is_empty()
        {
            let mut cache = client.inner.peer_cache.write().await;
            for p in &s.peers {
                if p.is_channel {
                    cache.channels.entry(p.id).or_insert(p.access_hash);
                } else {
                    cache.users.entry(p.id).or_insert(p.access_hash);
                }
            }
            tracing::debug!(
                "[ferogram] Peer cache restored: {} users, {} channels",
                cache.users.len(),
                cache.channels.len()
            );
        }

        // Restore update state / catch-up
        //
        // Two modes:
        // catch_up=false -> always call sync_pts_state() so we start from
        //                the current server state (ignore saved pts).
        // catch_up=true  -> if we have a saved pts > 0, restore it and let
        //                get_difference() fetch what we missed.  Only fall
        //                back to sync_pts_state() when there is no saved
        //                state (first boot, or fresh session).
        let has_saved_state = loaded_session
            .as_ref()
            .is_some_and(|s| s.updates_state.is_initialised());

        if catch_up && has_saved_state {
            let snap = &loaded_session.as_ref().unwrap().updates_state;
            let mut state = client.inner.pts_state.lock().await;
            state.pts = snap.pts;
            state.qts = snap.qts;
            state.date = snap.date;
            state.seq = snap.seq;
            for &(cid, cpts) in &snap.channels {
                state.channel_pts.insert(cid, cpts);
            }
            tracing::info!(
                "[ferogram] Update state restored: pts={}, qts={}, seq={}, {} channels",
                state.pts,
                state.qts,
                state.seq,
                state.channel_pts.len()
            );
            drop(state);

            // Capture channel list before spawn: get_difference() resets
            // PtsState via from_server_state (channel_pts preserved now, but
            // we need the IDs to drive per-channel catch-up regardless).
            let channel_ids: Vec<i64> = snap.channels.iter().map(|&(cid, _)| cid).collect();

            // Now spawn the catch-up diff: pts is the *old* value, so
            // getDifference will return exactly what we missed.
            let c = client.clone();
            let utx = client.inner.update_tx.clone();
            tokio::spawn(async move {
                match c.get_difference().await {
                    Ok(missed) => {
                        tracing::info!(
                            "[ferogram] catch_up: {} global updates replayed",
                            missed.len()
                        );
                        for u in missed {
                            if utx.try_send(attach_client_to_update(u, &c)).is_err() {
                                tracing::warn!(
                                    "[ferogram] update channel full: dropping catch-up update"
                                );
                                break;
                            }
                        }
                    }
                    Err(e) => tracing::warn!("[ferogram] catch_up getDifference: {e}"),
                }

                // Limit concurrency to avoid FLOOD_WAIT from spawning one task
                // per channel with no cap (a session with 500 channels would
                // fire 500 simultaneous API calls).
                if !channel_ids.is_empty() {
                    tracing::info!(
                        "[ferogram] catch_up: per-channel diff for {} channels",
                        channel_ids.len()
                    );
                    let sem = std::sync::Arc::new(tokio::sync::Semaphore::new(10));
                    for channel_id in channel_ids {
                        let c2 = c.clone();
                        let utx2 = utx.clone();
                        let permit = sem.clone().acquire_owned().await.unwrap();
                        tokio::spawn(async move {
                            let _permit = permit; // released when task completes
                            match c2.get_channel_difference(channel_id).await {
                                Ok(updates) => {
                                    if !updates.is_empty() {
                                        tracing::debug!(
                                            "[ferogram] catch_up channel {channel_id}: {} updates",
                                            updates.len()
                                        );
                                    }
                                    for u in updates {
                                        if utx2.try_send(u).is_err() {
                                            tracing::warn!(
                                                "[ferogram] update channel full: dropping channel diff update"
                                            );
                                            break;
                                        }
                                    }
                                }
                                Err(e) => {
                                    tracing::warn!("[ferogram] catch_up channel {channel_id}: {e}")
                                }
                            }
                        });
                    }
                }
            });
        } else {
            // No saved state or catch_up disabled: sync from server.
            let _ = client.sync_pts_state().await;
        }

        Ok((client, shutdown_token))
    }

    async fn fresh_connect(
        socks5: Option<&crate::socks5::Socks5Config>,
        mtproxy: Option<&crate::proxy::MtProxyConfig>,
        transport: &TransportKind,
    ) -> Result<(Connection, i32, HashMap<i32, DcEntry>), InvocationError> {
        tracing::debug!("[ferogram] Fresh connect to DC2 …");
        let conn = Connection::connect_raw(
            crate::dc_migration::fallback_dc_addr(2),
            socks5,
            mtproxy,
            transport,
            2i16,
        )
        .await?;
        let opts = session::default_dc_addresses()
            .into_iter()
            .map(|(id, addr)| {
                (
                    id,
                    DcEntry {
                        dc_id: id,
                        addr,
                        auth_key: None,
                        first_salt: 0,
                        time_offset: 0,
                        flags: DcFlags::NONE,
                    },
                )
            })
            .collect();
        Ok((conn, 2, opts))
    }

    // Session

    /// Build a [`PersistedSession`] snapshot from current client state.
    ///
    /// Single source of truth used by both [`save_session`] and
    /// [`export_session_string`]: any serialisation change only needs
    /// to be made here.
    async fn build_persisted_session(&self) -> PersistedSession {
        use session::{CachedPeer, UpdatesStateSnap};

        let writer_guard = self.inner.writer.lock().await;
        let home_dc_id = *self.inner.home_dc_id.lock().await;
        let dc_options = self.inner.dc_options.lock().await;

        let mut dcs: Vec<DcEntry> = dc_options
            .values()
            .map(|e| DcEntry {
                dc_id: e.dc_id,
                addr: e.addr.clone(),
                auth_key: if e.dc_id == home_dc_id {
                    Some(writer_guard.auth_key_bytes())
                } else {
                    e.auth_key
                },
                first_salt: if e.dc_id == home_dc_id {
                    writer_guard.first_salt()
                } else {
                    e.first_salt
                },
                time_offset: if e.dc_id == home_dc_id {
                    writer_guard.time_offset()
                } else {
                    e.time_offset
                },
                flags: e.flags,
            })
            .collect();
        // Also persist media DCs so they survive restart.
        {
            let media_opts = self.inner.media_dc_options.lock().await;
            for e in media_opts.values() {
                dcs.push(e.clone());
            }
        }
        self.inner.dc_pool.lock().await.collect_keys(&mut dcs);

        let pts_snap = {
            let s = self.inner.pts_state.lock().await;
            UpdatesStateSnap {
                pts: s.pts,
                qts: s.qts,
                date: s.date,
                seq: s.seq,
                channels: s.channel_pts.iter().map(|(&k, &v)| (k, v)).collect(),
            }
        };

        let peers: Vec<CachedPeer> = {
            let cache = self.inner.peer_cache.read().await;
            let mut v = Vec::with_capacity(cache.users.len() + cache.channels.len());
            for (&id, &hash) in &cache.users {
                v.push(CachedPeer {
                    id,
                    access_hash: hash,
                    is_channel: false,
                });
            }
            for (&id, &hash) in &cache.channels {
                v.push(CachedPeer {
                    id,
                    access_hash: hash,
                    is_channel: true,
                });
            }
            v
        };

        PersistedSession {
            home_dc_id,
            dcs,
            updates_state: pts_snap,
            peers,
        }
    }

    /// Persist the current session to the configured [`SessionBackend`].
    pub async fn save_session(&self) -> Result<(), InvocationError> {
        let session = self.build_persisted_session().await;
        self.inner
            .session_backend
            .save(&session)
            .map_err(InvocationError::Io)?;
        tracing::debug!("[ferogram] Session saved ✓");
        Ok(())
    }

    /// Export the current session as a portable URL-safe base64 string.
    ///
    /// The returned string encodes the auth key, DC, update state, and peer
    /// cache. Store it in an environment variable or secret manager and pass
    /// it back via [`Config::with_string_session`] to restore the session
    /// without re-authenticating.
    pub async fn export_session_string(&self) -> Result<String, InvocationError> {
        Ok(self.build_persisted_session().await.to_string())
    }

    /// Return the media-only DC address for the given DC id, if known.
    ///
    /// Media DCs (`media_only = true` in `DcOption`) are preferred for file
    /// uploads and downloads because they are not subject to the API rate
    /// limits applied to the main DC connection.
    pub async fn media_dc_addr(&self, dc_id: i32) -> Option<String> {
        self.inner
            .media_dc_options
            .lock()
            .await
            .get(&dc_id)
            .map(|e| e.addr.clone())
    }

    /// Return the best media DC address for the current home DC (falls back to
    /// any known media DC if no home-DC media entry exists).
    pub async fn best_media_dc_addr(&self) -> Option<(i32, String)> {
        let home = *self.inner.home_dc_id.lock().await;
        let media = self.inner.media_dc_options.lock().await;
        media
            .get(&home)
            .map(|e| (home, e.addr.clone()))
            .or_else(|| media.iter().next().map(|(&id, e)| (id, e.addr.clone())))
    }

    /// Returns `true` if the client is already authorized.
    pub async fn is_authorized(&self) -> Result<bool, InvocationError> {
        match self.invoke(&tl::functions::updates::GetState {}).await {
            Ok(_) => Ok(true),
            Err(e)
                if e.is("AUTH_KEY_UNREGISTERED")
                    || matches!(&e, InvocationError::Rpc(r) if r.code == 401) =>
            {
                Ok(false)
            }
            Err(e) => Err(e),
        }
    }

    /// Sign in as a bot.
    pub async fn bot_sign_in(&self, token: &str) -> Result<String, InvocationError> {
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
        tracing::info!("[ferogram] Bot signed in ✓  ({name})");
        self.inner
            .is_bot
            .store(true, std::sync::atomic::Ordering::Relaxed);
        Ok(name)
    }

    /// Request a login code for a user account.
    pub async fn request_login_code(&self, phone: &str) -> Result<LoginToken, InvocationError> {
        use tl::enums::auth::SentCode;

        let req = self.make_send_code_req(phone);
        let body = self.rpc_call_raw(&req).await?;

        let mut cur = Cursor::from_slice(&body);
        let hash = match tl::enums::auth::SentCode::deserialize(&mut cur)? {
            SentCode::SentCode(s) => s.phone_code_hash,
            SentCode::Success(_) => {
                return Err(InvocationError::Deserialize("unexpected Success".into()));
            }
            SentCode::PaymentRequired(_) => {
                return Err(InvocationError::Deserialize(
                    "payment required to send code".into(),
                ));
            }
        };
        tracing::info!("[ferogram] Login code sent");
        Ok(LoginToken {
            phone: phone.to_string(),
            phone_code_hash: hash,
        })
    }

    /// Complete sign-in with the code sent to the phone.
    pub async fn sign_in(&self, token: &LoginToken, code: &str) -> Result<String, SignInError> {
        let req = tl::functions::auth::SignIn {
            phone_number: token.phone.clone(),
            phone_code_hash: token.phone_code_hash.clone(),
            phone_code: Some(code.trim().to_string()),
            email_verification: None,
        };

        let body = match self.rpc_call_raw(&req).await {
            Ok(b) => b,
            Err(e) if e.is("SESSION_PASSWORD_NEEDED") => {
                let t = self.get_password_info().await.map_err(SignInError::Other)?;
                return Err(SignInError::PasswordRequired(Box::new(t)));
            }
            Err(e) if e.is("PHONE_CODE_*") => return Err(SignInError::InvalidCode),
            Err(e) => return Err(SignInError::Other(e)),
        };

        let mut cur = Cursor::from_slice(&body);
        match tl::enums::auth::Authorization::deserialize(&mut cur)
            .map_err(|e| SignInError::Other(e.into()))?
        {
            tl::enums::auth::Authorization::Authorization(a) => {
                self.cache_user(&a.user).await;
                let name = Self::extract_user_name(&a.user);
                tracing::info!("[ferogram] Signed in ✓  Welcome, {name}!");
                Ok(name)
            }
            tl::enums::auth::Authorization::SignUpRequired(_) => Err(SignInError::SignUpRequired),
        }
    }

    /// Complete 2FA login.
    pub async fn check_password(
        &self,
        token: PasswordToken,
        password: impl AsRef<[u8]>,
    ) -> Result<String, InvocationError> {
        let pw = token.password;
        let algo = pw
            .current_algo
            .ok_or_else(|| InvocationError::Deserialize("no current_algo".into()))?;
        let (salt1, salt2, p, g) = Self::extract_password_params(&algo)?;
        let g_b = pw
            .srp_b
            .ok_or_else(|| InvocationError::Deserialize("no srp_b".into()))?;
        let a = pw.secure_random;
        let srp_id = pw
            .srp_id
            .ok_or_else(|| InvocationError::Deserialize("no srp_id".into()))?;

        let (m1, g_a) =
            two_factor_auth::calculate_2fa(salt1, salt2, p, g, &g_b, &a, password.as_ref());
        let req = tl::functions::auth::CheckPassword {
            password: tl::enums::InputCheckPasswordSrp::InputCheckPasswordSrp(
                tl::types::InputCheckPasswordSrp {
                    srp_id,
                    a: g_a.to_vec(),
                    m1: m1.to_vec(),
                },
            ),
        };

        let body = self.rpc_call_raw(&req).await?;
        let mut cur = Cursor::from_slice(&body);
        match tl::enums::auth::Authorization::deserialize(&mut cur)? {
            tl::enums::auth::Authorization::Authorization(a) => {
                self.cache_user(&a.user).await;
                let name = Self::extract_user_name(&a.user);
                tracing::info!("[ferogram] 2FA ✓  Welcome, {name}!");
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
                tracing::info!("[ferogram] Signed out ✓");
                Ok(true)
            }
            Err(e) if e.is("AUTH_KEY_UNREGISTERED") => Ok(false),
            Err(e) => Err(e),
        }
    }

    // Get self

    // Get users

    /// Fetch user info by ID. Returns `None` for each ID that is not found.
    ///
    /// Used internally by [`update::IncomingMessage::sender_user`].
    pub async fn get_users_by_id(
        &self,
        ids: &[i64],
    ) -> Result<Vec<Option<crate::types::User>>, InvocationError> {
        let cache = self.inner.peer_cache.read().await;
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
        let body = self.rpc_call_raw(&req).await?;
        let mut cur = Cursor::from_slice(&body);
        let users = Vec::<tl::enums::User>::deserialize(&mut cur)?;
        self.cache_users_slice(&users).await;
        Ok(users
            .into_iter()
            .map(crate::types::User::from_raw)
            .collect())
    }

    /// Fetch information about the logged-in user.
    pub async fn get_me(&self) -> Result<tl::types::User, InvocationError> {
        let req = tl::functions::users::GetUsers {
            id: vec![tl::enums::InputUser::UserSelf],
        };
        let body = self.rpc_call_raw(&req).await?;
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

    // Updates

    /// Return an [`UpdateStream`] that yields incoming [`Update`]s.
    ///
    /// The reader task (started inside `connect()`) sends all updates to
    /// `inner.update_tx`. This method proxies those updates into a fresh
    /// caller-owned channel: typically called once per bot/app loop.
    pub fn stream_updates(&self) -> UpdateStream {
        // Guard: only one UpdateStream is supported per Client clone group.
        // A second call would compete with the first for updates, causing
        // non-deterministic splitting. Panic early with a clear message.
        if self
            .inner
            .stream_active
            .swap(true, std::sync::atomic::Ordering::SeqCst)
        {
            panic!(
                "stream_updates() called twice on the same Client: only one UpdateStream is supported per client"
            );
        }
        let (caller_tx, rx) = mpsc::unbounded_channel::<update::Update>();
        let internal_rx = self._update_rx.clone();
        tokio::spawn(async move {
            let mut guard = internal_rx.lock().await;
            while let Some(upd) = guard.recv().await {
                if caller_tx.send(upd).is_err() {
                    break;
                }
            }
        });
        UpdateStream { rx }
    }

    // Network hint

    /// Signal that network connectivity has been restored.
    ///
    /// Call this from platform network-change callbacks: Android's
    /// `ConnectivityManager`, iOS `NWPathMonitor`, or any other OS hook
    /// to make the client attempt an immediate reconnect instead of waiting
    /// for the exponential backoff timer to expire.
    ///
    /// Safe to call at any time: if the connection is healthy the hint is
    /// silently ignored by the reader task; if it is in a backoff loop it
    /// wakes up and tries again right away.
    pub fn signal_network_restored(&self) {
        let _ = self.inner.network_hint_tx.send(());
    }

    // Reader task
    // Decrypts frames without holding any lock, then routes:
    // rpc_result  -> pending map (oneshot to waiting RPC caller)
    // update      -> update_tx  (delivered to stream_updates consumers)
    // bad_server_salt -> updates writer salt
    //
    // On error: drains pending with Io errors (so AutoSleep retries callers),
    // then loops with exponential backoff until reconnect succeeds.
    // network_hint_rx lets external callers (Android/iOS) skip the backoff.
    //
    // DC migration / reconnect: the new read half arrives via new_conn_rx.
    // The select! between recv_frame_owned and new_conn_rx.recv() make sure we
    // switch to the new connection immediately, without waiting for the next
    // frame on the old (now stale) connection.

    // Reader task supervisor
    //
    // run_reader_task is the outer supervisor. It wraps reader_loop in a
    // restart loop so that if reader_loop ever exits for any reason other than
    // a clean shutdown request, it is automatically reconnected and restarted.
    //
    // This mirrors what Telegram Desktop does: the network thread is considered
    // infrastructure and is never allowed to die permanently.
    //
    // On unexpected exit: drain pending RPCs with ConnectionReset, backoff-reconnect
    // (500ms to 5s cap), spawn init_connection as a background task (same pattern
    // as do_reconnect_loop), hand the init oneshot to the restarted reader_loop.
    #[allow(clippy::too_many_arguments)]
    async fn run_reader_task(
        &self,
        read_half: OwnedReadHalf,
        frame_kind: FrameKind,
        auth_key: [u8; 256],
        session_id: i64,
        mut new_conn_rx: mpsc::UnboundedReceiver<(OwnedReadHalf, FrameKind, [u8; 256], i64)>,
        mut network_hint_rx: mpsc::UnboundedReceiver<()>,
        shutdown_token: CancellationToken,
    ) {
        let mut rh = read_half;
        let mut fk = frame_kind;
        let mut ak = auth_key;
        let mut sid = session_id;
        // On first start no init is needed (connect() already called it).
        // On restarts we pass the spawned init task so reader_loop handles it.
        let mut restart_init_rx: Option<oneshot::Receiver<Result<(), InvocationError>>> = None;
        let mut restart_count: u32 = 0;

        loop {
            tokio::select! {
                // Clean shutdown
                _ = shutdown_token.cancelled() => {
                    tracing::info!("[ferogram] Reader task: shutdown requested, exiting cleanly.");
                    let mut pending = self.inner.pending.lock().await;
                    for (_, tx) in pending.drain() {
                        let _ = tx.send(Err(InvocationError::Dropped));
                    }
                    return;
                }

                // reader_loop
                _ = self.reader_loop(
                        rh, fk, ak, sid,
                        restart_init_rx.take(),
                        &mut new_conn_rx, &mut network_hint_rx,
                    ) => {}
            }

            // If we reach here, reader_loop returned without a shutdown signal.
            // This should never happen in normal operation: treat it as a fault.
            if shutdown_token.is_cancelled() {
                tracing::debug!("[ferogram] Reader task: exiting after loop (shutdown).");
                return;
            }

            restart_count += 1;
            tracing::error!(
                "[ferogram] Reader loop exited unexpectedly (restart #{restart_count}):                  supervisor reconnecting …"
            );

            {
                let mut pending = self.inner.pending.lock().await;
                for (_, tx) in pending.drain() {
                    let _ = tx.send(Err(InvocationError::Io(std::io::Error::new(
                        std::io::ErrorKind::ConnectionReset,
                        "reader task restarted",
                    ))));
                }
            }
            // drain sent_bodies alongside pending to prevent unbounded growth.
            self.inner.writer.lock().await.sent_bodies.clear();

            let mut delay_ms = RECONNECT_BASE_MS;
            let new_conn = loop {
                tracing::debug!("[ferogram] Supervisor: reconnecting in {delay_ms} ms …");
                tokio::select! {
                    _ = shutdown_token.cancelled() => {
                        tracing::debug!("[ferogram] Supervisor: shutdown during reconnect, exiting.");
                        return;
                    }
                    _ = sleep(Duration::from_millis(delay_ms)) => {}
                }

                // do_reconnect ignores both params (_old_auth_key, _old_frame_kind)
                // it re-reads everything from ClientInner. rh/fk/ak/sid were moved
                // into reader_loop, so we pass dummies here; fresh values come back
                // from the Ok result and replace them below.
                let dummy_ak = [0u8; 256];
                let dummy_fk = FrameKind::Abridged;
                match self.do_reconnect(&dummy_ak, &dummy_fk).await {
                    Ok(conn) => break conn,
                    Err(e) => {
                        tracing::warn!("[ferogram] Supervisor: reconnect failed ({e})");
                        let next = (delay_ms * 2).min(RECONNECT_MAX_SECS * 1_000);
                        delay_ms = jitter_delay(next).as_millis() as u64;
                    }
                }
            };

            let (new_rh, new_fk, new_ak, new_sid) = new_conn;
            rh = new_rh;
            fk = new_fk;
            ak = new_ak;
            sid = new_sid;

            // be running to route the RPC response, or we deadlock).
            let (init_tx, init_rx) = oneshot::channel();
            let c = self.clone();
            let utx = self.inner.update_tx.clone();
            tokio::spawn(async move {
                // Respect FLOOD_WAIT (same as do_reconnect_loop).
                let result = loop {
                    match c.init_connection().await {
                        Ok(()) => break Ok(()),
                        Err(InvocationError::Rpc(ref r)) if r.flood_wait_seconds().is_some() => {
                            let secs = r.flood_wait_seconds().unwrap();
                            tracing::warn!(
                                "[ferogram] Supervisor init_connection FLOOD_WAIT_{secs}: waiting"
                            );
                            sleep(Duration::from_secs(secs + 1)).await;
                        }
                        Err(e) => break Err(e),
                    }
                };
                if result.is_ok() {
                    // After fresh DH, wait 2 s for key propagation before getDifference.
                    if c.inner
                        .dh_in_progress
                        .load(std::sync::atomic::Ordering::SeqCst)
                    {
                        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                    }
                    let missed = match c.get_difference().await {
                        Ok(updates) => updates,
                        Err(ref e)
                            if matches!(e,
                            InvocationError::Rpc(r) if r.code == 401) =>
                        {
                            tracing::warn!(
                                "[ferogram] getDifference AUTH_KEY_UNREGISTERED after \
                                 fresh DH: falling back to sync_pts_state"
                            );
                            let _ = c.sync_pts_state().await;
                            vec![]
                        }
                        Err(e) => {
                            tracing::warn!("[ferogram] getDifference failed after reconnect: {e}");
                            vec![]
                        }
                    };
                    for u in missed {
                        if utx.try_send(u).is_err() {
                            tracing::warn!(
                                "[ferogram] update channel full: dropping catch-up update"
                            );
                            break;
                        }
                    }
                }
                let _ = init_tx.send(result);
            });
            restart_init_rx = Some(init_rx);

            tracing::debug!(
                "[ferogram] Supervisor: restarting reader loop (restart #{restart_count}) …"
            );
            // Loop back -> reader_loop restarts with the fresh connection.
        }
    }

    #[allow(clippy::too_many_arguments)]
    async fn reader_loop(
        &self,
        mut rh: OwnedReadHalf,
        mut fk: FrameKind,
        mut ak: [u8; 256],
        mut sid: i64,
        // When Some, the supervisor has already spawned init_connection on our
        // behalf (supervisor restart path). On first start this is None.
        initial_init_rx: Option<oneshot::Receiver<Result<(), InvocationError>>>,
        new_conn_rx: &mut mpsc::UnboundedReceiver<(OwnedReadHalf, FrameKind, [u8; 256], i64)>,
        network_hint_rx: &mut mpsc::UnboundedReceiver<()>,
    ) {
        // Tracks an in-flight init_connection task spawned after every reconnect.
        // The reader loop must keep routing frames while we wait so the RPC
        // response can reach its oneshot sender (otherwise -> 30 s self-deadlock).
        // If init fails we re-enter the reconnect loop immediately.
        let mut init_rx: Option<oneshot::Receiver<Result<(), InvocationError>>> = initial_init_rx;
        // How many consecutive init_connection failures have occurred on the
        // *current* auth key.  We retry with the same key up to 2 times before
        // assuming the key is stale and clearing it for a fresh DH handshake.
        // This prevents a transient 30 s timeout from nuking a valid session.
        let mut init_fail_count: u32 = 0;

        let mut gap_tick = tokio::time::interval(std::time::Duration::from_millis(1500));
        gap_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

        let mut restart_interval = self.inner.restart_policy.restart_interval().map(|d| {
            let mut i = tokio::time::interval(d);
            i.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            i
        });
        if let Some(ref mut i) = restart_interval {
            i.tick().await;
        }

        loop {
            tokio::select! {
                // Drive possible-gap deadline every 1.5 s: if updates were buffered
                // waiting for a pts gap fill and no new update arrives, this fires
                // getDifference after the 1-second window expires.
                _ = gap_tick.tick() => {
                    // get_difference() is now atomic (check-and-set inside a single
                    // lock acquisition), so there is no need to guard against a
                    // concurrent in-flight call here : get_difference() will bail
                    // safely on its own.  Just check has_global() + deadline.
                    if self.inner.possible_gap.lock().await.has_global() {
                        let gap_expired = self.inner.possible_gap.lock().await.global_deadline_elapsed();
                        if gap_expired {
                            let c = self.clone();
                            tokio::spawn(async move {
                                if let Err(e) = c.check_update_deadline().await {
                                    tracing::warn!("[ferogram] gap tick getDifference: {e}");
                                }
                            });
                        }
                    }
                }
                _ = async {
                    if let Some(ref mut i) = restart_interval { i.tick().await; }
                    else { std::future::pending::<()>().await; }
                } => {
                    tracing::info!("[ferogram] scheduled restart: reconnecting");
                    let _ = self.inner.write_half.lock().await.shutdown().await;
                    let _ = self.inner.network_hint_tx.send(());
                }
                // Normal frame (or application-level keepalive timeout)
                outcome = recv_frame_with_keepalive(&mut rh, &fk, self, &ak) => {
                    match outcome {
                        FrameOutcome::Frame(mut raw) => {
                            let msg = match EncryptedSession::decrypt_frame(&ak, sid, &mut raw) {
                                Ok(m)  => m,
                                Err(e) => {
                                    // A decrypt failure (e.g. Crypto(InvalidBuffer) from a
                                    // 4-byte transport error that slipped through) means our
                                    // auth key is stale or the framing is broken. Treat it as
                                    // fatal: same path as FrameOutcome::Error: so pending RPCs
                                    // unblock immediately instead of hanging for 30 s.
                                    tracing::warn!("[ferogram] Decrypt error: {e:?}: failing pending waiters and reconnecting");
                                    drop(init_rx.take());
                                    {
                                        let mut pending = self.inner.pending.lock().await;
                                        let msg = format!("decrypt error: {e}");
                                        for (_, tx) in pending.drain() {
                                            let _ = tx.send(Err(InvocationError::Io(
                                                std::io::Error::new(
                                                    std::io::ErrorKind::InvalidData,
                                                    msg.clone(),
                                                )
                                            )));
                                        }
                                    }
                                    self.inner.writer.lock().await.sent_bodies.clear();
                                    match self.do_reconnect_loop(
                                        RECONNECT_BASE_MS, &mut rh, &mut fk, &mut ak, &mut sid,
                                        network_hint_rx,
                                    ).await {
                                        Some(rx) => { init_rx = Some(rx); }
                                        None     => return,
                                    }
                                    continue;
                                }
                            };
                            //  discards the frame-level salt entirely
                            // (it's not the "server salt" we should use: that only comes
                            // from new_session_created, bad_server_salt, or future_salts).
                            // Overwriting enc.salt here would clobber the managed salt pool.
                            self.route_frame(msg.body, msg.msg_id).await;

                            //: Acks are NOT flushed here standalone.
                            // They accumulate in pending_ack and are bundled into the next
                            // outgoing request container
                            // avoiding an extra standalone frame (and extra RTT exposure).
                        }

                        FrameOutcome::Error(e) => {
                            tracing::warn!("[ferogram] Reader: connection error: {e}");
                            drop(init_rx.take()); // discard any in-flight init

                            // Detect definitive auth-key rejection.  Telegram signals
                            // this with a -404 transport error (now surfaced as Rpc(-404)
                            // by recv_frame_read) or an immediate EOF/RST.  In that case
                            // we clear the saved key so do_reconnect_loop falls through to
                            // connect_raw (fresh DH) rather than reconnecting with the same
                            // expired key and getting -404 forever.
                            //
                            // -429 = TRANSPORT_FLOOD (rate limit). The key is fine: do NOT
                            // clear it. Clearing on -429 causes a double-DH race with the
                            // startup path, producing AUTH_KEY_UNREGISTERED post-reconnect.
                            let key_is_stale = match &e {
                                InvocationError::Rpc(r) if r.code == -404 => true,
                                InvocationError::Rpc(r) if r.code == -429 => false,
                                InvocationError::Io(io)
                                    if io.kind() == std::io::ErrorKind::UnexpectedEof
                                    || io.kind() == std::io::ErrorKind::ConnectionReset => true,
                                _ => false,
                            };
                            // Only clear the key if no DH is already in progress.
                            // The startup init_connection path may have already claimed
                            // dh_in_progress; honour that to avoid a double-DH race.
                            let clear_key = key_is_stale
                                && self.inner.dh_in_progress
                                    .compare_exchange(false, true,
                                        std::sync::atomic::Ordering::SeqCst,
                                        std::sync::atomic::Ordering::SeqCst)
                                    .is_ok();
                            if clear_key {
                                let home_dc_id = *self.inner.home_dc_id.lock().await;
                                let mut opts = self.inner.dc_options.lock().await;
                                if let Some(entry) = opts.get_mut(&home_dc_id) {
                                    tracing::warn!(
                                        "[ferogram] Stale auth key on DC{home_dc_id} ({e}) \
                                        : clearing for fresh DH"
                                    );
                                    entry.auth_key = None;
                                }
                            }

                            // Fail all in-flight RPCs immediately so AutoSleep
                            // retries them as soon as we reconnect.
                            {
                                let mut pending = self.inner.pending.lock().await;
                                let msg = e.to_string();
                                for (_, tx) in pending.drain() {
                                    let _ = tx.send(Err(InvocationError::Io(
                                        std::io::Error::new(
                                            std::io::ErrorKind::ConnectionReset, msg.clone()))));
                                }
                            }
                            // drain sent_bodies so it doesn't grow unbounded under loss.
                            self.inner.writer.lock().await.sent_bodies.clear();

                            // Skip backoff when the key is stale: no point waiting before
                            // fresh DH: the server told us directly to renegotiate.
                            let reconnect_delay = if clear_key { 0 } else { RECONNECT_BASE_MS };
                            match self.do_reconnect_loop(
                                reconnect_delay, &mut rh, &mut fk, &mut ak, &mut sid,
                                network_hint_rx,
                            ).await {
                                Some(rx) => {
                                    // DH (if any) is complete; release the guard so a future
                                    // stale-key event can claim it again.
                                    self.inner.dh_in_progress
                                        .store(false, std::sync::atomic::Ordering::SeqCst);
                                    init_rx = Some(rx);
                                }
                                None => {
                                    self.inner.dh_in_progress
                                        .store(false, std::sync::atomic::Ordering::SeqCst);
                                    return; // shutdown requested
                                }
                            }
                        }

                        FrameOutcome::Keepalive => {
                            // Drive possible-gap deadline: if updates were buffered
                            // waiting for a gap fill and no new update has arrived
                            // to re-trigger check_and_fill_gap, this fires getDifference.
                            let c = self.clone();
                            tokio::spawn(async move {
                                if let Err(e) = c.check_update_deadline().await {
                                    tracing::warn!("[ferogram] check_update_deadline: {e}");
                                }
                            });
                        }
                    }
                }

                // DC migration / deliberate reconnect
                maybe = new_conn_rx.recv() => {
                    if let Some((new_rh, new_fk, new_ak, new_sid)) = maybe {
                        rh = new_rh; fk = new_fk; ak = new_ak; sid = new_sid;
                        tracing::debug!("[ferogram] Reader: switched to new connection.");
                    } else {
                        break; // reconnect_tx dropped -> client is shutting down
                    }
                }


                // init_connection result (polled only when Some)
                init_result = async { init_rx.as_mut().unwrap().await }, if init_rx.is_some() => {
                    init_rx = None;
                    match init_result {
                        Ok(Ok(())) => {
                            init_fail_count = 0;
                            // do NOT save_session here.
                            // We do NOT save the session here on a plain TCP reconnect.
                            // reconnect: only when a genuinely new auth key is
                            // generated (fresh DH).  Writing here was the mechanism
                            // by which bugs S1 and S2 corrupted the on-disk session:
                            // if fresh DH ran with the wrong DC, the bad key was
                            // then immediately flushed to disk.  Without the write
                            // there is nothing to corrupt.
                            tracing::info!("[ferogram] Reconnected to Telegram ✓: session live, replaying missed updates …");
                        }

                        Ok(Err(e)) => {
                            // TCP connected but init RPC failed.
                            // Only clear auth key on definitive bad-key signals from Telegram.
                            // -429 = TRANSPORT_FLOOD: key is valid, just throttled: do NOT clear.
                            let key_is_stale = match &e {
                                InvocationError::Rpc(r) if r.code == -404 => true,
                                InvocationError::Rpc(r) if r.code == -429 => false,
                                InvocationError::Io(io) if io.kind() == std::io::ErrorKind::UnexpectedEof
                                    || io.kind() == std::io::ErrorKind::ConnectionReset => true,
                                _ => false,
                            };
                            // Use compare_exchange so we don't stomp on another in-progress DH.
                            let dh_claimed = key_is_stale
                                && self.inner.dh_in_progress
                                    .compare_exchange(false, true,
                                        std::sync::atomic::Ordering::SeqCst,
                                        std::sync::atomic::Ordering::SeqCst)
                                    .is_ok();

                            if dh_claimed {
                                tracing::warn!(
                                    "[ferogram] init_connection: definitive bad-key ({e}) \
                                    : clearing auth key for fresh DH …"
                                );
                                init_fail_count = 0;
                                let home_dc_id = *self.inner.home_dc_id.lock().await;
                                let mut opts = self.inner.dc_options.lock().await;
                                if let Some(entry) = opts.get_mut(&home_dc_id) {
                                    entry.auth_key = None;
                                }
                                // dh_in_progress is released by do_reconnect_loop's caller.
                            } else {
                                init_fail_count += 1;
                                tracing::warn!(
                                    "[ferogram] init_connection failed (attempt {init_fail_count}, {e}) \
                                    : retrying with same key …"
                                );
                            }
                            {
                                let mut pending = self.inner.pending.lock().await;
                                let msg = e.to_string();
                                for (_, tx) in pending.drain() {
                                    let _ = tx.send(Err(InvocationError::Io(
                                        std::io::Error::new(
                                            std::io::ErrorKind::ConnectionReset, msg.clone()))));
                                }
                            }
                            match self.do_reconnect_loop(
                                0, &mut rh, &mut fk, &mut ak, &mut sid, network_hint_rx,
                            ).await {
                                Some(rx) => { init_rx = Some(rx); }
                                None     => return,
                            }
                        }

                        Err(_) => {
                            // init task was dropped (shouldn't normally happen).
                            tracing::warn!("[ferogram] init_connection task dropped unexpectedly, reconnecting …");
                            match self.do_reconnect_loop(
                                RECONNECT_BASE_MS, &mut rh, &mut fk, &mut ak, &mut sid,
                                network_hint_rx,
                            ).await {
                                Some(rx) => { init_rx = Some(rx); }
                                None     => return,
                            }
                        }
                    }
                }
            }
        }
    }

    /// Route a decrypted MTProto frame body to either a pending RPC caller or update_tx.
    async fn route_frame(&self, body: Vec<u8>, msg_id: i64) {
        if body.len() < 4 {
            return;
        }
        let cid = u32::from_le_bytes(body[..4].try_into().unwrap());

        match cid {
            ID_RPC_RESULT => {
                if body.len() < 12 {
                    return;
                }
                let req_msg_id = i64::from_le_bytes(body[4..12].try_into().unwrap());
                let inner = body[12..].to_vec();
                // ack the rpc_result container message
                self.inner.writer.lock().await.pending_ack.push(msg_id);
                let result = unwrap_envelope(inner);
                if let Some(tx) = self.inner.pending.lock().await.remove(&req_msg_id) {
                    // request resolved: remove from sent_bodies and container_map
                    self.inner
                        .writer
                        .lock()
                        .await
                        .sent_bodies
                        .remove(&req_msg_id);
                    // Remove any container entry that pointed at this request.
                    self.inner
                        .writer
                        .lock()
                        .await
                        .container_map
                        .retain(|_, inner| *inner != req_msg_id);
                    let to_send = match result {
                        Ok(EnvelopeResult::Payload(p)) => Ok(p),
                        Ok(EnvelopeResult::RawUpdates(bodies)) => {
                            // route through dispatch_updates so pts/seq is
                            // properly tracked. Previously updates were sent directly
                            // to update_tx, skipping pts tracking -> false gap ->
                            // getDifference -> duplicate deliveries.
                            let c = self.clone();
                            tokio::spawn(async move {
                                for body in bodies {
                                    c.dispatch_updates(&body).await;
                                }
                            });
                            Ok(vec![])
                        }
                        Ok(EnvelopeResult::Pts(pts, pts_count)) => {
                            // updateShortSentMessage: advance pts without emitting any Update.
                            let c = self.clone();
                            tokio::spawn(async move {
                                match c.check_and_fill_gap(pts, pts_count, None).await {
                                    Ok(replayed) => {
                                        // replayed is normally empty (no gap); emit if getDifference ran
                                        for u in replayed {
                                            let _ = c.inner.update_tx.try_send(u);
                                        }
                                    }
                                    Err(e) => tracing::warn!(
                                        "[ferogram] updateShortSentMessage pts advance: {e}"
                                    ),
                                }
                            });
                            Ok(vec![])
                        }
                        Ok(EnvelopeResult::None) => Ok(vec![]),
                        Err(e) => {
                            tracing::debug!(
                                "[ferogram] rpc_result deserialize failure for msg_id={req_msg_id}: {e}"
                            );
                            Err(e)
                        }
                    };
                    let _ = tx.send(to_send);
                }
            }
            ID_RPC_ERROR => {
                tracing::warn!("[ferogram] Unexpected top-level rpc_error (no pending target)");
            }
            ID_MSG_CONTAINER => {
                if body.len() < 8 {
                    return;
                }
                let count = u32::from_le_bytes(body[4..8].try_into().unwrap()) as usize;
                let mut pos = 8usize;
                for _ in 0..count {
                    if pos + 16 > body.len() {
                        break;
                    }
                    // Extract inner msg_id for correct ack tracking
                    let inner_msg_id = i64::from_le_bytes(body[pos..pos + 8].try_into().unwrap());
                    let inner_len =
                        u32::from_le_bytes(body[pos + 12..pos + 16].try_into().unwrap()) as usize;
                    pos += 16;
                    if pos + inner_len > body.len() {
                        break;
                    }
                    let inner = body[pos..pos + inner_len].to_vec();
                    pos += inner_len;
                    Box::pin(self.route_frame(inner, inner_msg_id)).await;
                }
            }
            ID_GZIP_PACKED => {
                let bytes = tl_read_bytes(&body[4..]).unwrap_or_default();
                if let Ok(inflated) = gz_inflate(&bytes) {
                    // pass same outer msg_id: gzip has no msg_id of its own
                    Box::pin(self.route_frame(inflated, msg_id)).await;
                }
            }
            ID_BAD_SERVER_SALT => {
                // bad_server_salt#edab447b bad_msg_id:long bad_msg_seqno:int error_code:int new_server_salt:long
                // body[0..4]   = constructor
                // body[4..12]  = bad_msg_id       (long,  8 bytes)
                // body[12..16] = bad_msg_seqno     (int,   4 bytes)
                // body[16..20] = error_code        (int,   4 bytes)  ← NOT the salt!
                // body[20..28] = new_server_salt   (long,  8 bytes)  ← actual salt
                if body.len() >= 28 {
                    let bad_msg_id = i64::from_le_bytes(body[4..12].try_into().unwrap());
                    let new_salt = i64::from_le_bytes(body[20..28].try_into().unwrap());

                    // clear the salt pool and insert new_server_salt
                    // with valid_until=i32::MAX, then updates the active session salt.
                    {
                        let mut w = self.inner.writer.lock().await;
                        w.salts.clear();
                        w.salts.push(FutureSalt {
                            valid_since: 0,
                            valid_until: i32::MAX,
                            salt: new_salt,
                        });
                        w.enc.salt = new_salt;
                    }
                    tracing::debug!(
                        "[ferogram] bad_server_salt: bad_msg_id={bad_msg_id} new_salt={new_salt:#x}"
                    );

                    // Re-transmit the original request under the new salt.
                    // if bad_msg_id is not in sent_bodies directly, check
                    // container_map: the server may have sent the notification for
                    // the outer container msg_id rather than the inner request msg_id.
                    {
                        let mut w = self.inner.writer.lock().await;

                        // Resolve: if bad_msg_id points to a container, get the inner id.
                        let resolved_id = if w.sent_bodies.contains_key(&bad_msg_id) {
                            bad_msg_id
                        } else if let Some(&inner_id) = w.container_map.get(&bad_msg_id) {
                            w.container_map.remove(&bad_msg_id);
                            inner_id
                        } else {
                            bad_msg_id // will fall through to else-branch below
                        };

                        if let Some(orig_body) = w.sent_bodies.remove(&resolved_id) {
                            let (wire, new_msg_id) = w.enc.pack_body_with_msg_id(&orig_body, true);
                            let fk = w.frame_kind.clone();
                            // Intentionally NOT re-inserting into sent_bodies: a second
                            // bad_server_salt for new_msg_id finds nothing -> stops chain.
                            drop(w);
                            let mut pending = self.inner.pending.lock().await;
                            if let Some(tx) = pending.remove(&resolved_id) {
                                pending.insert(new_msg_id, tx);
                                drop(pending);
                                if let Err(e) = send_frame_write(
                                    &mut *self.inner.write_half.lock().await,
                                    &wire,
                                    &fk,
                                )
                                .await
                                {
                                    tracing::warn!(
                                        "[ferogram] bad_server_salt re-send failed: {e}"
                                    );
                                } else {
                                    tracing::debug!(
                                        "[ferogram] bad_server_salt re-sent \
                                         {resolved_id}→{new_msg_id}"
                                    );
                                }
                            }
                        } else {
                            // Not in sent_bodies (re-sent message rejected again, or unknown).
                            // Fail the pending caller so it doesn't hang.
                            drop(w);
                            if let Some(tx) = self.inner.pending.lock().await.remove(&bad_msg_id) {
                                let _ = tx.send(Err(InvocationError::Io(std::io::Error::new(
                                    std::io::ErrorKind::InvalidData,
                                    "bad_server_salt on re-sent message; caller should retry",
                                ))));
                            }
                        }
                    }

                    // Reactive refresh after bad_server_salt: reuses the extracted helper.
                    self.spawn_salt_fetch_if_needed();
                }
            }
            ID_PONG => {
                // Pong is the server's reply to Ping: NOT inside rpc_result.
                // pong#347773c5  msg_id:long  ping_id:long
                // body[4..12] = msg_id of the original Ping -> key in pending map
                //
                // pong has odd seq_no (content-related), must ack it.
                if body.len() >= 20 {
                    let ping_msg_id = i64::from_le_bytes(body[4..12].try_into().unwrap());
                    // Ack the pong frame itself (outer msg_id, not the ping msg_id).
                    self.inner.writer.lock().await.pending_ack.push(msg_id);
                    if let Some(tx) = self.inner.pending.lock().await.remove(&ping_msg_id) {
                        let mut w = self.inner.writer.lock().await;
                        w.sent_bodies.remove(&ping_msg_id);
                        w.container_map.retain(|_, inner| *inner != ping_msg_id);
                        drop(w);
                        let _ = tx.send(Ok(body));
                    }
                }
            }
            // FutureSalts: maintain the full server-provided salt pool.
            ID_FUTURE_SALTS => {
                // future_salts#ae500895
                // [0..4]   constructor
                // [4..12]  req_msg_id (long)
                // [12..16] now (int) : server's current Unix time
                // [16..20] vector constructor 0x1cb5c415
                // [20..24] count (int)
                // per entry (bare FutureSalt, no constructor):
                // [+0..+4]  valid_since (int)
                // [+4..+8]  valid_until (int)
                // [+8..+16] salt (long)
                // first entry starts at byte 24
                //
                // FutureSalts has odd seq_no, must ack it.
                self.inner.writer.lock().await.pending_ack.push(msg_id);

                if body.len() >= 24 {
                    let req_msg_id = i64::from_le_bytes(body[4..12].try_into().unwrap());
                    let server_now = i32::from_le_bytes(body[12..16].try_into().unwrap());
                    let count = u32::from_le_bytes(body[20..24].try_into().unwrap()) as usize;

                    // Parse ALL returned salts ( stores the full Vec).
                    // Each FutureSalt entry is 16 bytes starting at offset 24.
                    let mut new_salts: Vec<FutureSalt> = Vec::with_capacity(count.clamp(0, 4096));
                    for i in 0..count {
                        let base = 24 + i * 16;
                        if base + 16 > body.len() {
                            break;
                        }
                        // Wire format: valid_since(4) | salt(8) | valid_until(4)
                        // NOTE: The TL schema lists valid_until before salt, but the actual
                        // wire encoding puts salt first. Confirmed empirically: reading
                        // valid_until at [+4..+8] produces Oct-2019 timestamps impossible
                        // for future salts; at [+12..+16] produces correct future dates.
                        new_salts.push(FutureSalt {
                            valid_since: i32::from_le_bytes(
                                body[base..base + 4].try_into().unwrap(),
                            ),
                            salt: i64::from_le_bytes(body[base + 4..base + 12].try_into().unwrap()),
                            valid_until: i32::from_le_bytes(
                                body[base + 12..base + 16].try_into().unwrap(),
                            ),
                        });
                    }

                    if !new_salts.is_empty() {
                        // Sort newest-last (mirrors  sort_by_key(|s| -s.valid_since)
                        // which in ascending order puts highest valid_since at the end).
                        new_salts.sort_by_key(|s| s.valid_since);
                        let mut w = self.inner.writer.lock().await;
                        w.salts = new_salts;
                        w.start_salt_time = Some((server_now, std::time::Instant::now()));

                        // Pick the best currently-usable salt.
                        // A salt is usable after valid_since + SALT_USE_DELAY (60 s).
                        // Walk newest-to-oldest (end of vec to start) and pick the
                        // first one whose use-delay window has already opened.
                        let use_salt = w
                            .salts
                            .iter()
                            .rev()
                            .find(|s| s.valid_since + SALT_USE_DELAY <= server_now)
                            .or_else(|| w.salts.first())
                            .map(|s| s.salt);
                        if let Some(salt) = use_salt {
                            w.enc.salt = salt;
                            tracing::debug!(
                                "[ferogram] FutureSalts: stored {} salts, \
                                 active salt={salt:#x}",
                                w.salts.len()
                            );
                        }
                    }

                    if let Some(tx) = self.inner.pending.lock().await.remove(&req_msg_id) {
                        let mut w = self.inner.writer.lock().await;
                        w.sent_bodies.remove(&req_msg_id);
                        w.container_map.retain(|_, inner| *inner != req_msg_id);
                        drop(w);
                        let _ = tx.send(Ok(body));
                    }
                }
            }
            ID_NEW_SESSION => {
                // new_session_created#9ec20908 first_msg_id:long unique_id:long server_salt:long
                // body[4..12]  = first_msg_id
                // body[12..20] = unique_id
                // body[20..28] = server_salt
                if body.len() >= 28 {
                    let server_salt = i64::from_le_bytes(body[20..28].try_into().unwrap());
                    let mut w = self.inner.writer.lock().await;
                    // new_session_created has odd seq_no -> must ack.
                    w.pending_ack.push(msg_id);
                    //  clears the salt pool and inserts the fresh
                    // server_salt with valid_until=i32::MAX (permanently valid).
                    w.salts.clear();
                    w.salts.push(FutureSalt {
                        valid_since: 0,
                        valid_until: i32::MAX,
                        salt: server_salt,
                    });
                    w.enc.salt = server_salt;
                    tracing::debug!(
                        "[ferogram] new_session_created: salt pool reset to {server_salt:#x}"
                    );
                }
            }
            // +: bad_msg_notification
            ID_BAD_MSG_NOTIFY => {
                // bad_msg_notification#a7eff811 bad_msg_id:long bad_msg_seqno:int error_code:int
                if body.len() < 20 {
                    return;
                }
                let bad_msg_id = i64::from_le_bytes(body[4..12].try_into().unwrap());
                let error_code = u32::from_le_bytes(body[16..20].try_into().unwrap());

                //  description strings for each code
                let description = match error_code {
                    16 => "msg_id too low",
                    17 => "msg_id too high",
                    18 => "incorrect two lower order msg_id bits (bug)",
                    19 => "container msg_id is same as previously received (bug)",
                    20 => "message too old",
                    32 => "msg_seqno too low",
                    33 => "msg_seqno too high",
                    34 => "even msg_seqno expected (bug)",
                    35 => "odd msg_seqno expected (bug)",
                    48 => "incorrect server salt",
                    64 => "invalid container (bug)",
                    _ => "unknown bad_msg code",
                };

                // codes 16/17/48 are retryable; 32/33 are non-fatal seq corrections; rest are fatal.
                let retryable = matches!(error_code, 16 | 17 | 48);
                let fatal = !retryable && !matches!(error_code, 32 | 33);

                if fatal {
                    tracing::error!(
                        "[ferogram] bad_msg_notification (fatal): bad_msg_id={bad_msg_id} \
                         code={error_code}: {description}"
                    );
                } else {
                    tracing::warn!(
                        "[ferogram] bad_msg_notification: bad_msg_id={bad_msg_id} \
                         code={error_code}: {description}"
                    );
                }

                // Phase 1: hold writer only for enc-state mutations + packing.
                // The lock is dropped BEFORE we touch `pending`, eliminating the
                // writer→pending lock-order deadlock that existed before this fix.
                let resend: Option<(Vec<u8>, i64, i64, FrameKind)> = {
                    let mut w = self.inner.writer.lock().await;

                    // correct clock skew on codes 16/17.
                    if error_code == 16 || error_code == 17 {
                        w.enc.correct_time_offset(msg_id);
                    }
                    // correct seq_no on codes 32/33
                    if error_code == 32 || error_code == 33 {
                        w.enc.correct_seq_no(error_code);
                    }

                    if retryable {
                        // if bad_msg_id is not in sent_bodies directly, check
                        // container_map: the server sends the notification for the
                        // outer container msg_id when a whole container was bad.
                        let resolved_id = if w.sent_bodies.contains_key(&bad_msg_id) {
                            bad_msg_id
                        } else if let Some(&inner_id) = w.container_map.get(&bad_msg_id) {
                            w.container_map.remove(&bad_msg_id);
                            inner_id
                        } else {
                            bad_msg_id
                        };

                        if let Some(orig_body) = w.sent_bodies.remove(&resolved_id) {
                            let (wire, new_msg_id) = w.enc.pack_body_with_msg_id(&orig_body, true);
                            let fk = w.frame_kind.clone();
                            w.sent_bodies.insert(new_msg_id, orig_body);
                            // resolved_id is the inner msg_id we move in pending
                            Some((wire, resolved_id, new_msg_id, fk))
                        } else {
                            None
                        }
                    } else {
                        // Non-retryable: clean up so maps don't grow unbounded.
                        w.sent_bodies.remove(&bad_msg_id);
                        if let Some(&inner_id) = w.container_map.get(&bad_msg_id) {
                            w.sent_bodies.remove(&inner_id);
                            w.container_map.remove(&bad_msg_id);
                        }
                        None
                    }
                }; // ← writer lock released here

                match resend {
                    Some((wire, old_msg_id, new_msg_id, fk)) => {
                        // Phase 2: re-key pending (no writer lock held).
                        let has_waiter = {
                            let mut pending = self.inner.pending.lock().await;
                            if let Some(tx) = pending.remove(&old_msg_id) {
                                pending.insert(new_msg_id, tx);
                                true
                            } else {
                                false
                            }
                        };
                        if has_waiter {
                            // Phase 3: TCP send : no writer lock needed.
                            if let Err(e) = send_frame_write(
                                &mut *self.inner.write_half.lock().await,
                                &wire,
                                &fk,
                            )
                            .await
                            {
                                tracing::warn!("[ferogram] re-send failed: {e}");
                                self.inner
                                    .writer
                                    .lock()
                                    .await
                                    .sent_bodies
                                    .remove(&new_msg_id);
                            } else {
                                tracing::debug!("[ferogram] re-sent {old_msg_id}→{new_msg_id}");
                            }
                        } else {
                            self.inner
                                .writer
                                .lock()
                                .await
                                .sent_bodies
                                .remove(&new_msg_id);
                        }
                    }
                    None => {
                        // Not re-sending: surface error to the waiter so caller can retry.
                        if let Some(tx) = self.inner.pending.lock().await.remove(&bad_msg_id) {
                            let _ = tx.send(Err(InvocationError::Deserialize(format!(
                                "bad_msg_notification code={error_code} ({description})"
                            ))));
                        }
                    }
                }
            }
            // MsgDetailedInfo -> ack the answer_msg_id
            ID_MSG_DETAILED_INFO => {
                // msg_detailed_info#276d3ec6 msg_id:long answer_msg_id:long bytes:int status:int
                // body[4..12]  = msg_id (original request)
                // body[12..20] = answer_msg_id (what to ack)
                if body.len() >= 20 {
                    let answer_msg_id = i64::from_le_bytes(body[12..20].try_into().unwrap());
                    self.inner
                        .writer
                        .lock()
                        .await
                        .pending_ack
                        .push(answer_msg_id);
                    tracing::trace!(
                        "[ferogram] MsgDetailedInfo: queued ack for answer_msg_id={answer_msg_id}"
                    );
                }
            }
            ID_MSG_NEW_DETAIL_INFO => {
                // msg_new_detailed_info#809db6df answer_msg_id:long bytes:int status:int
                // body[4..12] = answer_msg_id
                if body.len() >= 12 {
                    let answer_msg_id = i64::from_le_bytes(body[4..12].try_into().unwrap());
                    self.inner
                        .writer
                        .lock()
                        .await
                        .pending_ack
                        .push(answer_msg_id);
                    tracing::trace!(
                        "[ferogram] MsgNewDetailedInfo: queued ack for {answer_msg_id}"
                    );
                }
            }
            // MsgResendReq -> re-send the requested msg_ids
            ID_MSG_RESEND_REQ => {
                // msg_resend_req#7d861a08 msg_ids:Vector<long>
                // body[4..8]   = 0x1cb5c415 (Vector constructor)
                // body[8..12]  = count
                // body[12..]   = msg_ids
                if body.len() >= 12 {
                    let count = u32::from_le_bytes(body[8..12].try_into().unwrap()) as usize;
                    let mut resends: Vec<(Vec<u8>, i64, i64)> = Vec::new();
                    {
                        let mut w = self.inner.writer.lock().await;
                        let fk = w.frame_kind.clone();
                        for i in 0..count {
                            let off = 12 + i * 8;
                            if off + 8 > body.len() {
                                break;
                            }
                            let resend_id =
                                i64::from_le_bytes(body[off..off + 8].try_into().unwrap());
                            if let Some(orig_body) = w.sent_bodies.remove(&resend_id) {
                                let (wire, new_id) = w.enc.pack_body_with_msg_id(&orig_body, true);
                                let mut pending = self.inner.pending.lock().await;
                                if let Some(tx) = pending.remove(&resend_id) {
                                    pending.insert(new_id, tx);
                                }
                                drop(pending);
                                w.sent_bodies.insert(new_id, orig_body);
                                resends.push((wire, resend_id, new_id));
                            }
                        }
                        let _ = fk; // fk captured above, writer lock drops here
                    }
                    // TCP sends outside writer lock
                    let fk = self.inner.writer.lock().await.frame_kind.clone();
                    for (wire, resend_id, new_id) in resends {
                        send_frame_write(&mut *self.inner.write_half.lock().await, &wire, &fk)
                            .await
                            .ok();
                        tracing::debug!("[ferogram] MsgResendReq: resent {resend_id} -> {new_id}");
                    }
                }
            }
            // log DestroySession outcomes
            0xe22045fc => {
                tracing::warn!(
                    "[ferogram] destroy_session_ok received: session terminated by server"
                );
            }
            0x62d350c9 => {
                tracing::warn!(
                    "[ferogram] destroy_session_none received: session was already gone"
                );
            }
            ID_UPDATES
            | ID_UPDATE_SHORT
            | ID_UPDATES_COMBINED
            | ID_UPDATE_SHORT_MSG
            | ID_UPDATE_SHORT_CHAT_MSG
            | ID_UPDATE_SHORT_SENT_MSG
            | ID_UPDATES_TOO_LONG => {
                // ack update frames too
                self.inner.writer.lock().await.pending_ack.push(msg_id);
                // Bug #1 fix: route through pts/qts/seq gap-checkers
                self.dispatch_updates(&body).await;
            }
            _ => {}
        }
    }

    // sort updates by pts-count key before dispatching
    // make seq check synchronous and gating

    /// Extract the pts-sort key for a single update: `pts - pts_count`.
    ///
    ///sorts every update batch by this key before processing.
    /// Without the sort, a container arriving as [pts=5, pts=3, pts=4] produces
    /// a false gap on the first item (expected 3, got 5) and spuriously fires
    /// getDifference even though the filling updates are present in the same batch.
    fn update_sort_key(upd: &tl::enums::Update) -> i32 {
        use tl::enums::Update::*;
        match upd {
            NewMessage(u) => u.pts - u.pts_count,
            EditMessage(u) => u.pts - u.pts_count,
            DeleteMessages(u) => u.pts - u.pts_count,
            ReadHistoryInbox(u) => u.pts - u.pts_count,
            ReadHistoryOutbox(u) => u.pts - u.pts_count,
            NewChannelMessage(u) => u.pts - u.pts_count,
            EditChannelMessage(u) => u.pts - u.pts_count,
            DeleteChannelMessages(u) => u.pts - u.pts_count,
            _ => 0,
        }
    }

    // Bug #1: pts-aware update dispatch

    /// Parse an incoming update container and route each update through the
    /// pts/qts/seq gap-checkers before forwarding to `update_tx`.
    async fn dispatch_updates(&self, body: &[u8]) {
        if body.len() < 4 {
            return;
        }
        let cid = u32::from_le_bytes(body[..4].try_into().unwrap());

        // updatesTooLong: we must call getDifference to recover missed updates.
        if cid == 0xe317af7e_u32 {
            tracing::warn!("[ferogram] updatesTooLong: getDifference");
            let c = self.clone();
            let utx = self.inner.update_tx.clone();
            tokio::spawn(async move {
                match c.get_difference().await {
                    Ok(updates) => {
                        for u in updates {
                            if utx.try_send(u).is_err() {
                                tracing::warn!("[ferogram] update channel full: dropping update");
                                break;
                            }
                        }
                    }
                    Err(e) => tracing::warn!("[ferogram] getDifference after updatesTooLong: {e}"),
                }
            });
            return;
        }

        // updateShortMessage (0x313bc7f8) and updateShortChatMessage (0x4d6deea5)
        // carry pts/pts_count but the old code forwarded them directly to update_tx WITHOUT
        // calling check_and_fill_gap. That left the internal pts counter frozen, so the
        // next updateNewMessage (e.g. the bot's own reply) triggered a false gap ->
        // getDifference -> re-delivery of already-processed messages -> duplicate replies.
        //
        // Fix: deserialize pts/pts_count from the compact struct, build the high-level
        // Update, then route through check_and_fill_gap exactly like every other pts update.
        if cid == 0x313bc7f8 {
            // updateShortMessage
            let mut cur = Cursor::from_slice(&body[4..]);
            let m = match tl::types::UpdateShortMessage::deserialize(&mut cur) {
                Ok(m) => m,
                Err(e) => {
                    tracing::debug!("[ferogram] updateShortMessage deserialize error: {e}");
                    return;
                }
            };
            let pts = m.pts;
            let pts_count = m.pts_count;
            let upd = update::Update::NewMessage(update::make_short_dm(m));
            let c = self.clone();
            let utx = self.inner.update_tx.clone();
            tokio::spawn(async move {
                match c
                    .check_and_fill_gap(pts, pts_count, Some(attach_client_to_update(upd, &c)))
                    .await
                {
                    Ok(updates) => {
                        for u in updates {
                            if utx.try_send(u).is_err() {
                                tracing::warn!("[ferogram] update channel full: dropping update");
                            }
                        }
                    }
                    Err(e) => tracing::warn!("[ferogram] updateShortMessage gap fill: {e}"),
                }
            });
            return;
        }
        if cid == 0x4d6deea5 {
            // updateShortChatMessage
            let mut cur = Cursor::from_slice(&body[4..]);
            let m = match tl::types::UpdateShortChatMessage::deserialize(&mut cur) {
                Ok(m) => m,
                Err(e) => {
                    tracing::debug!("[ferogram] updateShortChatMessage deserialize error: {e}");
                    return;
                }
            };
            let pts = m.pts;
            let pts_count = m.pts_count;
            let upd = update::Update::NewMessage(update::make_short_chat(m));
            let c = self.clone();
            let utx = self.inner.update_tx.clone();
            tokio::spawn(async move {
                match c
                    .check_and_fill_gap(pts, pts_count, Some(attach_client_to_update(upd, &c)))
                    .await
                {
                    Ok(updates) => {
                        for u in updates {
                            if utx.try_send(u).is_err() {
                                tracing::warn!("[ferogram] update channel full: dropping update");
                            }
                        }
                    }
                    Err(e) => tracing::warn!("[ferogram] updateShortChatMessage gap fill: {e}"),
                }
            });
            return;
        }

        // updateShortSentMessage push: advance pts without emitting an Update.
        // Telegram can also PUSH updateShortSentMessage (not just in RPC responses).
        // Same fix: extract pts and route through check_and_fill_gap.
        if cid == ID_UPDATE_SHORT_SENT_MSG {
            let mut cur = Cursor::from_slice(&body[4..]);
            match tl::types::UpdateShortSentMessage::deserialize(&mut cur) {
                Ok(m) => {
                    let pts = m.pts;
                    let pts_count = m.pts_count;
                    tracing::debug!(
                        "[ferogram] updateShortSentMessage (push): pts={pts} pts_count={pts_count}: advancing pts"
                    );
                    let c = self.clone();
                    let utx = self.inner.update_tx.clone();
                    tokio::spawn(async move {
                        match c.check_and_fill_gap(pts, pts_count, None).await {
                            Ok(replayed) => {
                                for u in replayed {
                                    if utx.try_send(u).is_err() {
                                        tracing::warn!(
                                            "[ferogram] update channel full: dropping update"
                                        );
                                    }
                                }
                            }
                            Err(e) => tracing::warn!(
                                "[ferogram] updateShortSentMessage push pts advance: {e}"
                            ),
                        }
                    });
                }
                Err(e) => {
                    tracing::debug!("[ferogram] updateShortSentMessage push deserialize error: {e}")
                }
            }
            return;
        }

        // Seq check must be synchronous and act as a gate for the whole
        // container.  The old approach spawned a task concurrently with dispatching
        // the individual updates, meaning seq could be advanced over an unclean batch.
        // seq must only advance after the full update loop runs with no
        // unresolved gaps.  We mirror this: check seq first, drop the container if
        // it's a gap or duplicate, and advance seq AFTER dispatching all updates.
        use crate::pts::PtsCheckResult;
        use ferogram_tl_types::{Cursor, Deserializable};

        // Parse the container ONCE and capture seq_info, users, chats, and the
        // bare update list together.  The old code parsed twice (once for seq_info,
        // once for raw updates) and both times discarded users/chats, so the
        // PeerCache was never populated from incoming update containers: hence
        // the "no access_hash for user X, using 0" warnings.
        struct ParsedContainer {
            seq_info: Option<(i32, i32)>,
            users: Vec<tl::enums::User>,
            chats: Vec<tl::enums::Chat>,
            updates: Vec<tl::enums::Update>,
        }

        let mut cur = Cursor::from_slice(body);
        let parsed: ParsedContainer = match cid {
            0x74ae4240 => {
                // updates#74ae4240
                match tl::enums::Updates::deserialize(&mut cur) {
                    Ok(tl::enums::Updates::Updates(u)) => ParsedContainer {
                        seq_info: Some((u.seq, u.seq)),
                        users: u.users,
                        chats: u.chats,
                        updates: u.updates,
                    },
                    _ => ParsedContainer {
                        seq_info: None,
                        users: vec![],
                        chats: vec![],
                        updates: vec![],
                    },
                }
            }
            0x725b04c3 => {
                // updatesCombined#725b04c3
                match tl::enums::Updates::deserialize(&mut cur) {
                    Ok(tl::enums::Updates::Combined(u)) => ParsedContainer {
                        seq_info: Some((u.seq, u.seq_start)),
                        users: u.users,
                        chats: u.chats,
                        updates: u.updates,
                    },
                    _ => ParsedContainer {
                        seq_info: None,
                        users: vec![],
                        chats: vec![],
                        updates: vec![],
                    },
                }
            }
            0x78d4dec1 => {
                // updateShort: no users/chats/seq
                match tl::types::UpdateShort::deserialize(&mut Cursor::from_slice(body)) {
                    Ok(u) => ParsedContainer {
                        seq_info: None,
                        users: vec![],
                        chats: vec![],
                        updates: vec![u.update],
                    },
                    Err(_) => ParsedContainer {
                        seq_info: None,
                        users: vec![],
                        chats: vec![],
                        updates: vec![],
                    },
                }
            }
            _ => ParsedContainer {
                seq_info: None,
                users: vec![],
                chats: vec![],
                updates: vec![],
            },
        };

        // Feed users/chats into the PeerCache so access_hash lookups work.
        if !parsed.users.is_empty() || !parsed.chats.is_empty() {
            self.cache_users_and_chats(&parsed.users, &parsed.chats)
                .await;
        }

        // synchronous seq gate: check before processing any updates.
        if let Some((seq, seq_start)) = parsed.seq_info
            && seq != 0
        {
            let result = self.inner.pts_state.lock().await.check_seq(seq, seq_start);
            match result {
                PtsCheckResult::Ok => {
                    // Good: will advance seq after the batch below.
                }
                PtsCheckResult::Duplicate => {
                    // Already handled this container: drop it silently.
                    tracing::debug!(
                        "[ferogram] seq duplicate (seq={seq}, seq_start={seq_start}): dropping container"
                    );
                    return;
                }
                PtsCheckResult::Gap { expected, got } => {
                    // Real seq gap: fire getDifference and drop the container.
                    // getDifference will deliver the missed updates.
                    tracing::warn!(
                        "[ferogram] seq gap: expected {expected}, got {got}: getDifference"
                    );
                    let c = self.clone();
                    let utx = self.inner.update_tx.clone();
                    tokio::spawn(async move {
                        match c.get_difference().await {
                            Ok(updates) => {
                                for u in updates {
                                    if utx.try_send(u).is_err() {
                                        tracing::warn!(
                                            "[ferogram] update channel full: dropping seq gap update"
                                        );
                                        break;
                                    }
                                }
                            }
                            Err(e) => tracing::warn!("[ferogram] seq gap fill: {e}"),
                        }
                    });
                    return; // drop this container; diff will supply updates
                }
            }
        }

        let mut raw: Vec<tl::enums::Update> = parsed.updates;

        // sort by (pts - pts_count) before dispatching:
        // updates.sort_by_key(update_sort_key).  Without this, an out-of-order batch
        // like [pts=5, pts=3, pts=4] falsely detects a gap on the first update and
        // fires getDifference even though the filling updates are in the same container.
        raw.sort_by_key(Self::update_sort_key);

        for upd in raw {
            self.dispatch_single_update(upd).await;
        }

        // advance seq AFTER the full batch has been dispatched: mirrors
        // ' post-loop seq advance that only fires when !have_unresolved_gaps.
        // (In our spawn-per-update model we can't track unresolved gaps inline, but
        // advancing here at minimum prevents premature seq advancement before the
        // container's pts checks have even been spawned.)
        if let Some((seq, _)) = parsed.seq_info
            && seq != 0
        {
            self.inner.pts_state.lock().await.advance_seq(seq);
        }
    }

    /// Route one bare `tl::enums::Update` through the pts/qts gap-checker,
    /// then emit surviving updates to `update_tx`.
    async fn dispatch_single_update(&self, upd: tl::enums::Update) {
        // Two-phase: inspect pts fields via reference first (all Copy), then
        // convert to high-level Update (consumes upd). Avoids borrow-then-move.
        enum Kind {
            GlobalPts {
                pts: i32,
                pts_count: i32,
                carry: bool,
            },
            ChannelPts {
                channel_id: i64,
                pts: i32,
                pts_count: i32,
                carry: bool,
            },
            Qts {
                qts: i32,
            },
            Passthrough,
        }

        fn ch_from_msg(msg: &tl::enums::Message) -> i64 {
            if let tl::enums::Message::Message(m) = msg
                && let tl::enums::Peer::Channel(c) = &m.peer_id
            {
                return c.channel_id;
            }
            0
        }

        let kind = {
            use tl::enums::Update::*;
            match &upd {
                NewMessage(u) => Kind::GlobalPts {
                    pts: u.pts,
                    pts_count: u.pts_count,
                    carry: true,
                },
                EditMessage(u) => Kind::GlobalPts {
                    pts: u.pts,
                    pts_count: u.pts_count,
                    carry: true,
                },
                DeleteMessages(u) => Kind::GlobalPts {
                    pts: u.pts,
                    pts_count: u.pts_count,
                    carry: true,
                },
                ReadHistoryInbox(u) => Kind::GlobalPts {
                    pts: u.pts,
                    pts_count: u.pts_count,
                    carry: false,
                },
                ReadHistoryOutbox(u) => Kind::GlobalPts {
                    pts: u.pts,
                    pts_count: u.pts_count,
                    carry: false,
                },
                NewChannelMessage(u) => Kind::ChannelPts {
                    channel_id: ch_from_msg(&u.message),
                    pts: u.pts,
                    pts_count: u.pts_count,
                    carry: true,
                },
                EditChannelMessage(u) => Kind::ChannelPts {
                    channel_id: ch_from_msg(&u.message),
                    pts: u.pts,
                    pts_count: u.pts_count,
                    carry: true,
                },
                DeleteChannelMessages(u) => Kind::ChannelPts {
                    channel_id: u.channel_id,
                    pts: u.pts,
                    pts_count: u.pts_count,
                    carry: true,
                },
                NewEncryptedMessage(u) => Kind::Qts { qts: u.qts },
                _ => Kind::Passthrough,
            }
        };

        let high = update::from_single_update_pub(upd);

        let to_send: Vec<update::Update> = match kind {
            Kind::GlobalPts {
                pts,
                pts_count,
                carry,
            } => {
                let first = if carry { high.into_iter().next() } else { None };
                // DEADLOCK FIX: never await an RPC inside the reader task.
                // Spawn gap-fill as a separate task; it can receive the RPC
                // response because the reader loop continues running.
                let c = self.clone();
                let utx = self.inner.update_tx.clone();
                tokio::spawn(async move {
                    match c.check_and_fill_gap(pts, pts_count, first).await {
                        Ok(v) => {
                            for u in v {
                                let u = attach_client_to_update(u, &c);
                                if utx.try_send(u).is_err() {
                                    tracing::warn!(
                                        "[ferogram] update channel full: dropping update"
                                    );
                                    break;
                                }
                            }
                        }
                        Err(e) => tracing::warn!("[ferogram] pts gap: {e}"),
                    }
                });
                vec![]
            }
            Kind::ChannelPts {
                channel_id,
                pts,
                pts_count,
                carry,
            } => {
                let first = if carry { high.into_iter().next() } else { None };
                if channel_id != 0 {
                    // DEADLOCK FIX: spawn; same reasoning as GlobalPts above.
                    let c = self.clone();
                    let utx = self.inner.update_tx.clone();
                    tokio::spawn(async move {
                        match c
                            .check_and_fill_channel_gap(channel_id, pts, pts_count, first)
                            .await
                        {
                            Ok(v) => {
                                for u in v {
                                    let u = attach_client_to_update(u, &c);
                                    if utx.try_send(u).is_err() {
                                        tracing::warn!(
                                            "[ferogram] update channel full: dropping update"
                                        );
                                        break;
                                    }
                                }
                            }
                            Err(e) => tracing::warn!("[ferogram] ch pts gap: {e}"),
                        }
                    });
                    vec![]
                } else {
                    first.into_iter().collect()
                }
            }
            Kind::Qts { qts } => {
                // DEADLOCK FIX: spawn; same reasoning as above.
                let c = self.clone();
                tokio::spawn(async move {
                    if let Err(e) = c.check_and_fill_qts_gap(qts, 1).await {
                        tracing::warn!("[ferogram] qts gap: {e}");
                    }
                });
                vec![]
            }
            Kind::Passthrough => high
                .into_iter()
                .map(|u| match u {
                    update::Update::NewMessage(msg) => {
                        update::Update::NewMessage(msg.with_client(self.clone()))
                    }
                    update::Update::MessageEdited(msg) => {
                        update::Update::MessageEdited(msg.with_client(self.clone()))
                    }
                    other => other,
                })
                .collect(),
        };

        for u in to_send {
            if self.inner.update_tx.try_send(u).is_err() {
                tracing::warn!("[ferogram] update channel full: dropping update");
            }
        }
    }

    /// Loops with exponential backoff until a TCP+DH reconnect succeeds, then
    /// spawns `init_connection` in a background task and returns a oneshot
    /// receiver for its result.
    ///
    /// - `initial_delay_ms = RECONNECT_BASE_MS` for a fresh disconnect.
    /// - `initial_delay_ms = 0` when TCP already worked but init failed: we
    ///   want to retry init immediately rather than waiting another full backoff.
    ///
    /// Returns `None` if the shutdown token fires (caller should exit).
    async fn do_reconnect_loop(
        &self,
        initial_delay_ms: u64,
        rh: &mut OwnedReadHalf,
        fk: &mut FrameKind,
        ak: &mut [u8; 256],
        sid: &mut i64,
        network_hint_rx: &mut mpsc::UnboundedReceiver<()>,
    ) -> Option<oneshot::Receiver<Result<(), InvocationError>>> {
        let mut delay_ms = if initial_delay_ms == 0 {
            // Caller explicitly requests an immediate first attempt (e.g. init
            // failed but TCP is up: no reason to wait before the next try).
            0
        } else {
            initial_delay_ms.max(RECONNECT_BASE_MS)
        };
        loop {
            tracing::debug!("[ferogram] Reconnecting in {delay_ms} ms …");
            tokio::select! {
                _ = sleep(Duration::from_millis(delay_ms)) => {}
                hint = network_hint_rx.recv() => {
                    hint?; // shutdown
                    tracing::debug!("[ferogram] Network hint -> skipping backoff, reconnecting now");
                }
            }

            match self.do_reconnect(ak, fk).await {
                Ok((new_rh, new_fk, new_ak, new_sid)) => {
                    *rh = new_rh;
                    *fk = new_fk;
                    *ak = new_ak;
                    *sid = new_sid;
                    tracing::debug!("[ferogram] TCP reconnected ✓: initialising session …");

                    // Spawn init_connection. MUST NOT be awaited inline: the
                    // reader loop must resume so it can route the RPC response.
                    // We give back a oneshot so the reader can act on failure.
                    let (init_tx, init_rx) = oneshot::channel();
                    let c = self.clone();
                    let utx = self.inner.update_tx.clone();
                    tokio::spawn(async move {
                        // Respect FLOOD_WAIT before sending the result back.
                        // Without this, a FLOOD_WAIT from Telegram during init
                        // would immediately re-trigger another reconnect attempt,
                        // which would itself hit FLOOD_WAIT: a ban spiral.
                        let result = loop {
                            match c.init_connection().await {
                                Ok(()) => break Ok(()),
                                Err(InvocationError::Rpc(ref r))
                                    if r.flood_wait_seconds().is_some() =>
                                {
                                    let secs = r.flood_wait_seconds().unwrap();
                                    tracing::warn!(
                                        "[ferogram] init_connection FLOOD_WAIT_{secs}:                                          waiting before retry"
                                    );
                                    sleep(Duration::from_secs(secs + 1)).await;
                                    // loop and retry init_connection
                                }
                                Err(e) => break Err(e),
                            }
                        };
                        if result.is_ok() {
                            // Replay any updates missed during the outage.
                            // After fresh DH the new key may not have propagated to
                            // all of Telegram's app servers yet, so getDifference can
                            // return AUTH_KEY_UNREGISTERED (401).  A 2 s pause lets the
                            // key replicate before we send any RPCs (same reason
                            // yields after fresh DH).  Without this, post-reconnect RPC
                            // calls silently fail and the bot stops responding.
                            if c.inner
                                .dh_in_progress
                                .load(std::sync::atomic::Ordering::SeqCst)
                            {
                                tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                            }
                            let missed = match c.get_difference().await {
                                Ok(updates) => updates,
                                Err(ref e)
                                    if matches!(e,
                                    InvocationError::Rpc(r) if r.code == 401) =>
                                {
                                    tracing::warn!(
                                        "[ferogram] getDifference AUTH_KEY_UNREGISTERED after \
                                         fresh DH: falling back to sync_pts_state"
                                    );
                                    let _ = c.sync_pts_state().await;
                                    vec![]
                                }
                                Err(e) => {
                                    tracing::warn!(
                                        "[ferogram] getDifference failed after reconnect: {e}"
                                    );
                                    vec![]
                                }
                            };
                            for u in missed {
                                if utx.try_send(attach_client_to_update(u, &c)).is_err() {
                                    tracing::warn!(
                                        "[ferogram] update channel full: dropping catch-up update"
                                    );
                                    break;
                                }
                            }
                        }
                        let _ = init_tx.send(result);
                    });
                    return Some(init_rx);
                }
                Err(e) => {
                    tracing::warn!("[ferogram] Reconnect attempt failed: {e}");
                    // Cap at max, then apply ±20 % jitter to avoid thundering herd.
                    // Ensure the delay always advances by at least RECONNECT_BASE_MS
                    // so a 0 initial delay on the first attempt doesn't spin-loop.
                    let next = delay_ms
                        .saturating_mul(2)
                        .clamp(RECONNECT_BASE_MS, RECONNECT_MAX_SECS * 1_000);
                    delay_ms = jitter_delay(next).as_millis() as u64;
                }
            }
        }
    }

    /// Reconnect to the home DC, replace the writer, and return the new read half.
    async fn do_reconnect(
        &self,
        _old_auth_key: &[u8; 256],
        _old_frame_kind: &FrameKind,
    ) -> Result<(OwnedReadHalf, FrameKind, [u8; 256], i64), InvocationError> {
        let home_dc_id = *self.inner.home_dc_id.lock().await;
        let (addr, saved_key, first_salt, time_offset) = {
            let opts = self.inner.dc_options.lock().await;
            match opts.get(&home_dc_id) {
                Some(e) => (e.addr.clone(), e.auth_key, e.first_salt, e.time_offset),
                None => (
                    crate::dc_migration::fallback_dc_addr(home_dc_id).to_string(),
                    None,
                    0,
                    0,
                ),
            }
        };
        let socks5 = self.inner.socks5.clone();
        let mtproxy = self.inner.mtproxy.clone();
        let transport = self.inner.transport.clone();

        let new_conn = if let Some(key) = saved_key {
            tracing::debug!("[ferogram] Reconnecting to DC{home_dc_id} with saved key …");
            match Connection::connect_with_key(
                &addr,
                key,
                first_salt,
                time_offset,
                socks5.as_ref(),
                &transport,
                home_dc_id as i16,
            )
            .await
            {
                Ok(c) => c,
                Err(e) => {
                    return Err(e);
                }
            }
        } else {
            Connection::connect_raw(
                &addr,
                socks5.as_ref(),
                mtproxy.as_ref(),
                &transport,
                home_dc_id as i16,
            )
            .await?
        };

        let (new_writer, new_wh, new_read, new_fk) = new_conn.into_writer();
        let new_ak = new_writer.enc.auth_key_bytes();
        let new_sid = new_writer.enc.session_id();
        *self.inner.writer.lock().await = new_writer;
        *self.inner.write_half.lock().await = new_wh;

        // The new writer is fresh (new EncryptedSession) but
        // salt_request_in_flight lives on self.inner and is never reset
        // automatically.  If a GetFutureSalts was in flight when the
        // disconnect happened the flag stays `true` forever, preventing any
        // future proactive salt refreshes.  Reset it here so the first
        // bad_server_salt after reconnect can spawn a new request.
        // because the entire Sender is recreated.
        self.inner
            .salt_request_in_flight
            .store(false, std::sync::atomic::Ordering::SeqCst);

        // Persist the new auth key so subsequent reconnects reuse it instead of
        // repeating fresh DH.  (Cleared keys cause a fresh-DH loop: clear -> DH →
        // key not saved -> next disconnect clears nothing -> but dc_options still
        // None -> DH again -> AUTH_KEY_UNREGISTERED on getDifference forever.)
        {
            let mut opts = self.inner.dc_options.lock().await;
            if let Some(entry) = opts.get_mut(&home_dc_id) {
                entry.auth_key = Some(new_ak);
            }
        }

        // NOTE: init_connection() is intentionally NOT called here.
        //
        // do_reconnect() is always called from inside the reader loop's select!,
        // which means the reader task is blocked while this function runs.
        // init_connection() sends an RPC and awaits the response: but only the
        // reader task can route that response back to the pending caller.
        // Calling it here creates a self-deadlock that times out after 30 s.
        //
        // Instead, callers are responsible for spawning init_connection() in a
        // separate task AFTER the reader loop has resumed and can process frames.

        Ok((new_read, new_fk, new_ak, new_sid))
    }

    // Messaging

    /// Send a text message. Use `"me"` for Saved Messages.
    pub async fn send_message(
        &self,
        peer: &str,
        text: &str,
    ) -> Result<update::IncomingMessage, InvocationError> {
        let p = self.resolve_peer(peer).await?;
        self.send_message_to_peer(p, text).await
    }

    /// Send a message to a peer (plain text shorthand).
    ///
    /// Accepts anything that converts to [`PeerRef`]: a `&str` username,
    /// an `i64` ID, or an already-resolved `tl::enums::Peer`.
    pub async fn send_message_to_peer(
        &self,
        peer: impl Into<PeerRef>,
        text: &str,
    ) -> Result<update::IncomingMessage, InvocationError> {
        self.send_message_to_peer_ex(peer, &InputMessage::text(text))
            .await
    }

    /// Send a message with full [`InputMessage`] options.
    ///
    /// Accepts anything that converts to [`PeerRef`].
    /// Returns the sent message as an [`update::IncomingMessage`].
    pub async fn send_message_to_peer_ex(
        &self,
        peer: impl Into<PeerRef>,
        msg: &InputMessage,
    ) -> Result<update::IncomingMessage, InvocationError> {
        let peer = peer.into().resolve(self).await?;
        let input_peer = self.inner.peer_cache.read().await.peer_to_input(&peer);
        let schedule = if msg.schedule_once_online {
            Some(0x7FFF_FFFEi32)
        } else {
            msg.schedule_date
        };

        // if media is attached, route through SendMedia instead of SendMessage.
        if let Some(media) = &msg.media {
            let req = tl::functions::messages::SendMedia {
                silent: msg.silent,
                background: msg.background,
                clear_draft: msg.clear_draft,
                noforwards: false,
                update_stickersets_order: false,
                invert_media: msg.invert_media,
                allow_paid_floodskip: false,
                peer: input_peer,
                reply_to: msg.reply_header(),
                media: media.clone(),
                message: msg.text.clone(),
                random_id: random_i64(),
                reply_markup: msg.reply_markup.clone(),
                entities: msg.entities.clone(),
                schedule_date: schedule,
                schedule_repeat_period: None,
                send_as: None,
                quick_reply_shortcut: None,
                effect: None,
                allow_paid_stars: None,
                suggested_post: None,
            };
            let body = self.rpc_call_raw_pub(&req).await?;
            return Ok(self.extract_sent_message(&body, msg, &peer));
        }

        let req = tl::functions::messages::SendMessage {
            no_webpage: msg.no_webpage,
            silent: msg.silent,
            background: msg.background,
            clear_draft: msg.clear_draft,
            noforwards: false,
            update_stickersets_order: false,
            invert_media: msg.invert_media,
            allow_paid_floodskip: false,
            peer: input_peer,
            reply_to: msg.reply_header(),
            message: msg.text.clone(),
            random_id: random_i64(),
            reply_markup: msg.reply_markup.clone(),
            entities: msg.entities.clone(),
            schedule_date: schedule,
            schedule_repeat_period: None,
            send_as: None,
            quick_reply_shortcut: None,
            effect: None,
            allow_paid_stars: None,
            suggested_post: None,
        };
        let body = self.rpc_call_raw(&req).await?;
        Ok(self.extract_sent_message(&body, msg, &peer))
    }

    /// Parse the Updates blob returned by SendMessage / SendMedia and extract the
    /// sent message. Falls back to a synthetic stub if the response is opaque
    /// (e.g. `updateShortSentMessage` which doesn't include the full message).
    fn extract_sent_message(
        &self,
        body: &[u8],
        input: &InputMessage,
        peer: &tl::enums::Peer,
    ) -> update::IncomingMessage {
        if body.len() < 4 {
            return self.synthetic_sent(input, peer, 0, 0);
        }
        let cid = u32::from_le_bytes(body[..4].try_into().unwrap());

        // updates#74ae4240 / updatesCombined#725b04c3: full Updates container
        if cid == 0x74ae4240 || cid == 0x725b04c3 {
            let mut cur = Cursor::from_slice(body);
            if let Ok(tl::enums::Updates::Updates(u)) = tl::enums::Updates::deserialize(&mut cur) {
                for upd in &u.updates {
                    if let tl::enums::Update::NewMessage(nm) = upd {
                        return update::IncomingMessage::from_raw(nm.message.clone())
                            .with_client(self.clone());
                    }
                    if let tl::enums::Update::NewChannelMessage(nm) = upd {
                        return update::IncomingMessage::from_raw(nm.message.clone())
                            .with_client(self.clone());
                    }
                }
            }
            if let Ok(tl::enums::Updates::Combined(u)) =
                tl::enums::Updates::deserialize(&mut Cursor::from_slice(body))
            {
                for upd in &u.updates {
                    if let tl::enums::Update::NewMessage(nm) = upd {
                        return update::IncomingMessage::from_raw(nm.message.clone())
                            .with_client(self.clone());
                    }
                    if let tl::enums::Update::NewChannelMessage(nm) = upd {
                        return update::IncomingMessage::from_raw(nm.message.clone())
                            .with_client(self.clone());
                    }
                }
            }
        }

        // updateShortSentMessage#9015e101: server returns id/pts/date/media/entities
        // but not the full message body. Reconstruct from what we know.
        if cid == 0x9015e101 {
            let mut cur = Cursor::from_slice(&body[4..]);
            if let Ok(sent) = tl::types::UpdateShortSentMessage::deserialize(&mut cur) {
                return self.synthetic_sent_from_short(sent, input, peer);
            }
        }

        // updateShortMessage#313bc7f8 (DM to another user: we get a short form)
        if cid == 0x313bc7f8 {
            let mut cur = Cursor::from_slice(&body[4..]);
            if let Ok(m) = tl::types::UpdateShortMessage::deserialize(&mut cur) {
                let msg = tl::types::Message {
                    out: m.out,
                    mentioned: m.mentioned,
                    media_unread: m.media_unread,
                    silent: m.silent,
                    post: false,
                    from_scheduled: false,
                    legacy: false,
                    edit_hide: false,
                    pinned: false,
                    noforwards: false,
                    invert_media: false,
                    offline: false,
                    video_processing_pending: false,
                    paid_suggested_post_stars: false,
                    paid_suggested_post_ton: false,
                    id: m.id,
                    from_id: Some(tl::enums::Peer::User(tl::types::PeerUser {
                        user_id: m.user_id,
                    })),
                    peer_id: tl::enums::Peer::User(tl::types::PeerUser { user_id: m.user_id }),
                    saved_peer_id: None,
                    fwd_from: m.fwd_from,
                    via_bot_id: m.via_bot_id,
                    via_business_bot_id: None,
                    reply_to: m.reply_to,
                    date: m.date,
                    message: m.message,
                    media: None,
                    reply_markup: None,
                    entities: m.entities,
                    views: None,
                    forwards: None,
                    replies: None,
                    edit_date: None,
                    post_author: None,
                    grouped_id: None,
                    reactions: None,
                    restriction_reason: None,
                    ttl_period: None,
                    quick_reply_shortcut_id: None,
                    effect: None,
                    factcheck: None,
                    report_delivery_until_date: None,
                    paid_message_stars: None,
                    suggested_post: None,
                    from_rank: None,
                    from_boosts_applied: None,
                    schedule_repeat_period: None,
                    summary_from_language: None,
                };
                return update::IncomingMessage::from_raw(tl::enums::Message::Message(msg))
                    .with_client(self.clone());
            }
        }

        // Fallback: synthetic stub with no message ID known
        self.synthetic_sent(input, peer, 0, 0)
    }

    /// Construct a synthetic `IncomingMessage` from an `UpdateShortSentMessage`.
    fn synthetic_sent_from_short(
        &self,
        sent: tl::types::UpdateShortSentMessage,
        input: &InputMessage,
        peer: &tl::enums::Peer,
    ) -> update::IncomingMessage {
        let msg = tl::types::Message {
            out: sent.out,
            mentioned: false,
            media_unread: false,
            silent: input.silent,
            post: false,
            from_scheduled: false,
            legacy: false,
            edit_hide: false,
            pinned: false,
            noforwards: false,
            invert_media: input.invert_media,
            offline: false,
            video_processing_pending: false,
            paid_suggested_post_stars: false,
            paid_suggested_post_ton: false,
            id: sent.id,
            from_id: None,
            from_boosts_applied: None,
            from_rank: None,
            peer_id: peer.clone(),
            saved_peer_id: None,
            fwd_from: None,
            via_bot_id: None,
            via_business_bot_id: None,
            reply_to: input.reply_to.map(|id| {
                tl::enums::MessageReplyHeader::MessageReplyHeader(tl::types::MessageReplyHeader {
                    reply_to_scheduled: false,
                    forum_topic: false,
                    quote: false,
                    reply_to_msg_id: Some(id),
                    reply_to_peer_id: None,
                    reply_from: None,
                    reply_media: None,
                    reply_to_top_id: None,
                    quote_text: None,
                    quote_entities: None,
                    quote_offset: None,
                    todo_item_id: None,
                    poll_option: None,
                })
            }),
            date: sent.date,
            message: input.text.clone(),
            media: sent.media,
            reply_markup: input.reply_markup.clone(),
            entities: sent.entities,
            views: None,
            forwards: None,
            replies: None,
            edit_date: None,
            post_author: None,
            grouped_id: None,
            reactions: None,
            restriction_reason: None,
            ttl_period: sent.ttl_period,
            quick_reply_shortcut_id: None,
            effect: None,
            factcheck: None,
            report_delivery_until_date: None,
            paid_message_stars: None,
            suggested_post: None,
            schedule_repeat_period: None,
            summary_from_language: None,
        };
        update::IncomingMessage::from_raw(tl::enums::Message::Message(msg))
            .with_client(self.clone())
    }

    /// Synthetic stub used when Updates parsing yields no message.
    fn synthetic_sent(
        &self,
        input: &InputMessage,
        peer: &tl::enums::Peer,
        id: i32,
        date: i32,
    ) -> update::IncomingMessage {
        let msg = tl::types::Message {
            out: true,
            mentioned: false,
            media_unread: false,
            silent: input.silent,
            post: false,
            from_scheduled: false,
            legacy: false,
            edit_hide: false,
            pinned: false,
            noforwards: false,
            invert_media: input.invert_media,
            offline: false,
            video_processing_pending: false,
            paid_suggested_post_stars: false,
            paid_suggested_post_ton: false,
            id,
            from_id: None,
            from_boosts_applied: None,
            from_rank: None,
            peer_id: peer.clone(),
            saved_peer_id: None,
            fwd_from: None,
            via_bot_id: None,
            via_business_bot_id: None,
            reply_to: input.reply_to.map(|rid| {
                tl::enums::MessageReplyHeader::MessageReplyHeader(tl::types::MessageReplyHeader {
                    reply_to_scheduled: false,
                    forum_topic: false,
                    quote: false,
                    reply_to_msg_id: Some(rid),
                    reply_to_peer_id: None,
                    reply_from: None,
                    reply_media: None,
                    reply_to_top_id: None,
                    quote_text: None,
                    quote_entities: None,
                    quote_offset: None,
                    todo_item_id: None,
                    poll_option: None,
                })
            }),
            date,
            message: input.text.clone(),
            media: None,
            reply_markup: input.reply_markup.clone(),
            entities: input.entities.clone(),
            views: None,
            forwards: None,
            replies: None,
            edit_date: None,
            post_author: None,
            grouped_id: None,
            reactions: None,
            restriction_reason: None,
            ttl_period: None,
            quick_reply_shortcut_id: None,
            effect: None,
            factcheck: None,
            report_delivery_until_date: None,
            paid_message_stars: None,
            suggested_post: None,
            schedule_repeat_period: None,
            summary_from_language: None,
        };
        update::IncomingMessage::from_raw(tl::enums::Message::Message(msg))
            .with_client(self.clone())
    }

    /// Send directly to Saved Messages.
    pub async fn send_to_self(
        &self,
        text: &str,
    ) -> Result<update::IncomingMessage, InvocationError> {
        let req = tl::functions::messages::SendMessage {
            no_webpage: false,
            silent: false,
            background: false,
            clear_draft: false,
            noforwards: false,
            update_stickersets_order: false,
            invert_media: false,
            allow_paid_floodskip: false,
            peer: tl::enums::InputPeer::PeerSelf,
            reply_to: None,
            message: text.to_string(),
            random_id: random_i64(),
            reply_markup: None,
            entities: None,
            schedule_date: None,
            schedule_repeat_period: None,
            send_as: None,
            quick_reply_shortcut: None,
            effect: None,
            allow_paid_stars: None,
            suggested_post: None,
        };
        let body = self.rpc_call_raw(&req).await?;
        let self_peer = tl::enums::Peer::User(tl::types::PeerUser { user_id: 0 });
        Ok(self.extract_sent_message(&body, &InputMessage::text(text), &self_peer))
    }

    /// Edit an existing message.
    pub async fn edit_message(
        &self,
        peer: impl Into<PeerRef>,
        message_id: i32,
        new_text: &str,
    ) -> Result<(), InvocationError> {
        let peer = peer.into().resolve(self).await?;
        let input_peer = self.inner.peer_cache.read().await.peer_to_input(&peer);
        let req = tl::functions::messages::EditMessage {
            no_webpage: false,
            invert_media: false,
            peer: input_peer,
            id: message_id,
            message: Some(new_text.to_string()),
            media: None,
            reply_markup: None,
            entities: None,
            schedule_date: None,
            schedule_repeat_period: None,
            quick_reply_shortcut_id: None,
        };
        self.rpc_write(&req).await
    }

    /// Forward messages from `source` to `destination`.
    pub async fn forward_messages(
        &self,
        destination: impl Into<PeerRef>,
        message_ids: &[i32],
        source: impl Into<PeerRef>,
    ) -> Result<(), InvocationError> {
        let dest = destination.into().resolve(self).await?;
        let src = source.into().resolve(self).await?;
        let cache = self.inner.peer_cache.read().await;
        let to_peer = cache.peer_to_input(&dest);
        let from_peer = cache.peer_to_input(&src);
        drop(cache);

        let req = tl::functions::messages::ForwardMessages {
            silent: false,
            background: false,
            with_my_score: false,
            drop_author: false,
            drop_media_captions: false,
            noforwards: false,
            from_peer,
            id: message_ids.to_vec(),
            random_id: (0..message_ids.len()).map(|_| random_i64()).collect(),
            to_peer,
            top_msg_id: None,
            reply_to: None,
            schedule_date: None,
            schedule_repeat_period: None,
            send_as: None,
            quick_reply_shortcut: None,
            effect: None,
            video_timestamp: None,
            allow_paid_stars: None,
            allow_paid_floodskip: false,
            suggested_post: None,
        };
        self.rpc_write(&req).await
    }

    /// Forward messages and return the forwarded copies.
    ///
    /// Like [`forward_messages`] but parses the Updates response and returns
    /// the new messages in the destination chat, matching  behaviour.
    pub async fn forward_messages_returning(
        &self,
        destination: impl Into<PeerRef>,
        message_ids: &[i32],
        source: impl Into<PeerRef>,
    ) -> Result<Vec<update::IncomingMessage>, InvocationError> {
        let dest = destination.into().resolve(self).await?;
        let src = source.into().resolve(self).await?;
        let cache = self.inner.peer_cache.read().await;
        let to_peer = cache.peer_to_input(&dest);
        let from_peer = cache.peer_to_input(&src);
        drop(cache);

        let req = tl::functions::messages::ForwardMessages {
            silent: false,
            background: false,
            with_my_score: false,
            drop_author: false,
            drop_media_captions: false,
            noforwards: false,
            from_peer,
            id: message_ids.to_vec(),
            random_id: (0..message_ids.len()).map(|_| random_i64()).collect(),
            to_peer,
            top_msg_id: None,
            reply_to: None,
            schedule_date: None,
            schedule_repeat_period: None,
            send_as: None,
            quick_reply_shortcut: None,
            effect: None,
            video_timestamp: None,
            allow_paid_stars: None,
            allow_paid_floodskip: false,
            suggested_post: None,
        };
        let body = self.rpc_call_raw(&req).await?;
        // Parse the Updates container and collect NewMessage / NewChannelMessage updates.
        let mut out = Vec::new();
        if body.len() >= 4 {
            let cid = u32::from_le_bytes(body[..4].try_into().unwrap());
            if cid == 0x74ae4240 || cid == 0x725b04c3 {
                let mut cur = Cursor::from_slice(&body);
                let updates_opt = tl::enums::Updates::deserialize(&mut cur).ok();
                let raw_updates = match updates_opt {
                    Some(tl::enums::Updates::Updates(u)) => u.updates,
                    Some(tl::enums::Updates::Combined(u)) => u.updates,
                    _ => vec![],
                };
                for upd in raw_updates {
                    match upd {
                        tl::enums::Update::NewMessage(u) => {
                            out.push(
                                update::IncomingMessage::from_raw(u.message)
                                    .with_client(self.clone()),
                            );
                        }
                        tl::enums::Update::NewChannelMessage(u) => {
                            out.push(
                                update::IncomingMessage::from_raw(u.message)
                                    .with_client(self.clone()),
                            );
                        }
                        _ => {}
                    }
                }
            }
        }
        Ok(out)
    }

    /// Delete messages by ID.
    pub async fn delete_messages(
        &self,
        message_ids: Vec<i32>,
        revoke: bool,
    ) -> Result<(), InvocationError> {
        let req = tl::functions::messages::DeleteMessages {
            revoke,
            id: message_ids,
        };
        self.rpc_write(&req).await
    }

    /// Get messages by their IDs from a peer.
    pub async fn get_messages_by_id(
        &self,
        peer: impl Into<PeerRef>,
        ids: &[i32],
    ) -> Result<Vec<update::IncomingMessage>, InvocationError> {
        let peer = peer.into().resolve(self).await?;
        let input_peer = self.inner.peer_cache.read().await.peer_to_input(&peer);
        let id_list: Vec<tl::enums::InputMessage> = ids
            .iter()
            .map(|&id| tl::enums::InputMessage::Id(tl::types::InputMessageId { id }))
            .collect();
        let req = tl::functions::channels::GetMessages {
            channel: match &input_peer {
                tl::enums::InputPeer::Channel(c) => {
                    tl::enums::InputChannel::InputChannel(tl::types::InputChannel {
                        channel_id: c.channel_id,
                        access_hash: c.access_hash,
                    })
                }
                _ => return self.get_messages_user(input_peer, id_list).await,
            },
            id: id_list,
        };
        let body = self.rpc_call_raw(&req).await?;
        let mut cur = Cursor::from_slice(&body);
        let msgs = match tl::enums::messages::Messages::deserialize(&mut cur)? {
            tl::enums::messages::Messages::Messages(m) => m.messages,
            tl::enums::messages::Messages::Slice(m) => m.messages,
            tl::enums::messages::Messages::ChannelMessages(m) => m.messages,
            tl::enums::messages::Messages::NotModified(_) => vec![],
        };
        Ok(msgs
            .into_iter()
            .map(|m| update::IncomingMessage::from_raw(m).with_client(self.clone()))
            .collect())
    }

    async fn get_messages_user(
        &self,
        _peer: tl::enums::InputPeer,
        ids: Vec<tl::enums::InputMessage>,
    ) -> Result<Vec<update::IncomingMessage>, InvocationError> {
        let req = tl::functions::messages::GetMessages { id: ids };
        let body = self.rpc_call_raw(&req).await?;
        let mut cur = Cursor::from_slice(&body);
        let msgs = match tl::enums::messages::Messages::deserialize(&mut cur)? {
            tl::enums::messages::Messages::Messages(m) => m.messages,
            tl::enums::messages::Messages::Slice(m) => m.messages,
            tl::enums::messages::Messages::ChannelMessages(m) => m.messages,
            tl::enums::messages::Messages::NotModified(_) => vec![],
        };
        Ok(msgs
            .into_iter()
            .map(|m| update::IncomingMessage::from_raw(m).with_client(self.clone()))
            .collect())
    }

    /// Get the pinned message in a chat.
    pub async fn get_pinned_message(
        &self,
        peer: impl Into<PeerRef>,
    ) -> Result<Option<update::IncomingMessage>, InvocationError> {
        let peer = peer.into().resolve(self).await?;
        let input_peer = self.inner.peer_cache.read().await.peer_to_input(&peer);
        let req = tl::functions::messages::Search {
            peer: input_peer,
            q: String::new(),
            from_id: None,
            saved_peer_id: None,
            saved_reaction: None,
            top_msg_id: None,
            filter: tl::enums::MessagesFilter::InputMessagesFilterPinned,
            min_date: 0,
            max_date: 0,
            offset_id: 0,
            add_offset: 0,
            limit: 1,
            max_id: 0,
            min_id: 0,
            hash: 0,
        };
        let body = self.rpc_call_raw(&req).await?;
        let mut cur = Cursor::from_slice(&body);
        let msgs = match tl::enums::messages::Messages::deserialize(&mut cur)? {
            tl::enums::messages::Messages::Messages(m) => m.messages,
            tl::enums::messages::Messages::Slice(m) => m.messages,
            tl::enums::messages::Messages::ChannelMessages(m) => m.messages,
            tl::enums::messages::Messages::NotModified(_) => vec![],
        };
        Ok(msgs
            .into_iter()
            .next()
            .map(|m| update::IncomingMessage::from_raw(m).with_client(self.clone())))
    }

    /// Pin a message in a chat.
    pub async fn pin_message(
        &self,
        peer: impl Into<PeerRef>,
        message_id: i32,
        silent: bool,
        unpin: bool,
        pm_oneside: bool,
    ) -> Result<(), InvocationError> {
        let peer = peer.into().resolve(self).await?;
        let input_peer = self.inner.peer_cache.read().await.peer_to_input(&peer);
        let req = tl::functions::messages::UpdatePinnedMessage {
            silent,
            unpin,
            pm_oneside,
            peer: input_peer,
            id: message_id,
        };
        self.rpc_write(&req).await
    }

    /// Unpin a specific message.
    pub async fn unpin_message(
        &self,
        peer: impl Into<PeerRef>,
        message_id: i32,
    ) -> Result<(), InvocationError> {
        self.pin_message(peer, message_id, true, true, false).await
    }

    /// Fetch the message that `message` is replying to.
    ///
    /// Returns `None` if the message is not a reply, or if the original
    /// message could not be found (deleted / inaccessible).
    ///
    /// # Example
    /// ```rust,no_run
    /// # async fn f(client: ferogram::Client, msg: ferogram::update::IncomingMessage)
    /// #   -> Result<(), ferogram::InvocationError> {
    /// if let Some(replied) = client.get_reply_to_message(&msg).await? {
    /// println!("Replied to: {:?}", replied.text());
    /// }
    /// # Ok(()) }
    /// ```
    pub async fn get_reply_to_message(
        &self,
        message: &update::IncomingMessage,
    ) -> Result<Option<update::IncomingMessage>, InvocationError> {
        let reply_id = match message.reply_to_message_id() {
            Some(id) => id,
            None => return Ok(None),
        };
        let peer = match message.peer_id() {
            Some(p) => p.clone(),
            None => return Ok(None),
        };
        let input_peer = self.inner.peer_cache.read().await.peer_to_input(&peer);
        let id = vec![tl::enums::InputMessage::Id(tl::types::InputMessageId {
            id: reply_id,
        })];

        let result = match &input_peer {
            tl::enums::InputPeer::Channel(c) => {
                let req = tl::functions::channels::GetMessages {
                    channel: tl::enums::InputChannel::InputChannel(tl::types::InputChannel {
                        channel_id: c.channel_id,
                        access_hash: c.access_hash,
                    }),
                    id,
                };
                self.rpc_call_raw(&req).await?
            }
            _ => {
                let req = tl::functions::messages::GetMessages { id };
                self.rpc_call_raw(&req).await?
            }
        };

        let mut cur = Cursor::from_slice(&result);
        let msgs = match tl::enums::messages::Messages::deserialize(&mut cur)? {
            tl::enums::messages::Messages::Messages(m) => m.messages,
            tl::enums::messages::Messages::Slice(m) => m.messages,
            tl::enums::messages::Messages::ChannelMessages(m) => m.messages,
            tl::enums::messages::Messages::NotModified(_) => vec![],
        };
        Ok(msgs
            .into_iter()
            .next()
            .map(|m| update::IncomingMessage::from_raw(m).with_client(self.clone())))
    }

    /// Unpin all messages in a chat.
    pub async fn unpin_all_messages(
        &self,
        peer: impl Into<PeerRef>,
    ) -> Result<(), InvocationError> {
        let peer = peer.into().resolve(self).await?;
        let input_peer = self.inner.peer_cache.read().await.peer_to_input(&peer);
        let req = tl::functions::messages::UnpinAllMessages {
            peer: input_peer,
            top_msg_id: None,
            saved_peer_id: None,
        };
        self.rpc_write(&req).await
    }

    // Message search

    /// Search messages in a chat (simple form).
    /// For advanced filtering use [`Client::search`] -> [`SearchBuilder`].
    pub async fn search_messages(
        &self,
        peer: impl Into<PeerRef>,
        query: &str,
        limit: i32,
    ) -> Result<Vec<update::IncomingMessage>, InvocationError> {
        self.search(peer, query).limit(limit).fetch(self).await
    }

    /// Fluent search builder for in-chat message search.
    pub fn search(&self, peer: impl Into<PeerRef>, query: &str) -> SearchBuilder {
        SearchBuilder::new(peer.into(), query.to_string())
    }

    /// Search globally (simple form). For filtering use [`Client::search_global_builder`].
    pub async fn search_global(
        &self,
        query: &str,
        limit: i32,
    ) -> Result<Vec<update::IncomingMessage>, InvocationError> {
        self.search_global_builder(query)
            .limit(limit)
            .fetch(self)
            .await
    }

    /// Fluent builder for global cross-chat search.
    pub fn search_global_builder(&self, query: &str) -> GlobalSearchBuilder {
        GlobalSearchBuilder::new(query.to_string())
    }

    // Scheduled messages

    /// Retrieve all scheduled messages in a chat.
    ///
    /// Scheduled messages are messages set to be sent at a future time using
    /// [`InputMessage::schedule_date`].  Returns them newest-first.
    ///
    /// # Example
    /// ```rust,no_run
    /// # async fn f(client: ferogram::Client, peer: ferogram_tl_types::enums::Peer) -> Result<(), Box<dyn std::error::Error>> {
    /// let scheduled = client.get_scheduled_messages(peer).await?;
    /// for msg in &scheduled {
    /// println!("Scheduled: {:?} at {:?}", msg.text(), msg.date());
    /// }
    /// # Ok(()) }
    /// ```
    pub async fn get_scheduled_messages(
        &self,
        peer: impl Into<PeerRef>,
    ) -> Result<Vec<update::IncomingMessage>, InvocationError> {
        let peer = peer.into().resolve(self).await?;
        let input_peer = self.inner.peer_cache.read().await.peer_to_input(&peer);
        let req = tl::functions::messages::GetScheduledHistory {
            peer: input_peer,
            hash: 0,
        };
        let body = self.rpc_call_raw(&req).await?;
        let mut cur = Cursor::from_slice(&body);
        let msgs = match tl::enums::messages::Messages::deserialize(&mut cur)? {
            tl::enums::messages::Messages::Messages(m) => m.messages,
            tl::enums::messages::Messages::Slice(m) => m.messages,
            tl::enums::messages::Messages::ChannelMessages(m) => m.messages,
            tl::enums::messages::Messages::NotModified(_) => vec![],
        };
        Ok(msgs
            .into_iter()
            .map(|m| update::IncomingMessage::from_raw(m).with_client(self.clone()))
            .collect())
    }

    /// Delete one or more scheduled messages by their IDs.
    pub async fn delete_scheduled_messages(
        &self,
        peer: impl Into<PeerRef>,
        ids: Vec<i32>,
    ) -> Result<(), InvocationError> {
        let peer = peer.into().resolve(self).await?;
        let input_peer = self.inner.peer_cache.read().await.peer_to_input(&peer);
        let req = tl::functions::messages::DeleteScheduledMessages {
            peer: input_peer,
            id: ids,
        };
        self.rpc_write(&req).await
    }

    // Callback / Inline Queries

    /// Edit an inline message by its [`InputBotInlineMessageId`].
    ///
    /// Inline messages live on the bot's home DC, not necessarily the current
    /// connection's DC.  This method sends the edit RPC on the correct DC by
    /// using the DC ID encoded in `msg_id` (high 20 bits of the `dc_id` field).
    ///
    /// # Example
    /// ```rust,no_run
    /// # async fn f(
    /// #   client: ferogram::Client,
    /// #   id: ferogram_tl_types::enums::InputBotInlineMessageId,
    /// # ) -> Result<(), Box<dyn std::error::Error>> {
    /// client.edit_inline_message(id, "new text", None).await?;
    /// # Ok(()) }
    /// ```
    pub async fn edit_inline_message(
        &self,
        id: tl::enums::InputBotInlineMessageId,
        new_text: &str,
        reply_markup: Option<tl::enums::ReplyMarkup>,
    ) -> Result<bool, InvocationError> {
        let req = tl::functions::messages::EditInlineBotMessage {
            no_webpage: false,
            invert_media: false,
            id,
            message: Some(new_text.to_string()),
            media: None,
            reply_markup,
            entities: None,
        };
        let body = self.rpc_call_raw(&req).await?;
        // Bool#997275b5 = boolTrue; Bool#bc799737 = boolFalse
        Ok(body.len() >= 4 && u32::from_le_bytes(body[..4].try_into().unwrap()) == 0x997275b5)
    }

    /// Answer a callback query from an inline keyboard button press (bots only).
    pub async fn answer_callback_query(
        &self,
        query_id: i64,
        text: Option<&str>,
        alert: bool,
    ) -> Result<bool, InvocationError> {
        let req = tl::functions::messages::SetBotCallbackAnswer {
            alert,
            query_id,
            message: text.map(|s| s.to_string()),
            url: None,
            cache_time: 0,
        };
        let body = self.rpc_call_raw(&req).await?;
        Ok(body.len() >= 4 && u32::from_le_bytes(body[..4].try_into().unwrap()) == 0x997275b5)
    }

    pub async fn answer_inline_query(
        &self,
        query_id: i64,
        results: Vec<tl::enums::InputBotInlineResult>,
        cache_time: i32,
        is_personal: bool,
        next_offset: Option<String>,
    ) -> Result<bool, InvocationError> {
        let req = tl::functions::messages::SetInlineBotResults {
            gallery: false,
            private: is_personal,
            query_id,
            results,
            cache_time,
            next_offset,
            switch_pm: None,
            switch_webview: None,
        };
        let body = self.rpc_call_raw(&req).await?;
        Ok(body.len() >= 4 && u32::from_le_bytes(body[..4].try_into().unwrap()) == 0x997275b5)
    }

    // Dialogs

    /// Fetch up to `limit` dialogs, most recent first. Populates entity/message.
    pub async fn get_dialogs(&self, limit: i32) -> Result<Vec<Dialog>, InvocationError> {
        let req = tl::functions::messages::GetDialogs {
            exclude_pinned: false,
            folder_id: None,
            offset_date: 0,
            offset_id: 0,
            offset_peer: tl::enums::InputPeer::Empty,
            limit,
            hash: 0,
        };

        let body = self.rpc_call_raw(&req).await?;
        let mut cur = Cursor::from_slice(&body);
        let raw = match tl::enums::messages::Dialogs::deserialize(&mut cur)? {
            tl::enums::messages::Dialogs::Dialogs(d) => d,
            tl::enums::messages::Dialogs::Slice(d) => tl::types::messages::Dialogs {
                dialogs: d.dialogs,
                messages: d.messages,
                chats: d.chats,
                users: d.users,
            },
            tl::enums::messages::Dialogs::NotModified(_) => return Ok(vec![]),
        };

        // Build message map
        let msg_map: HashMap<i32, tl::enums::Message> = raw
            .messages
            .into_iter()
            .map(|m| {
                let id = match &m {
                    tl::enums::Message::Message(x) => x.id,
                    tl::enums::Message::Service(x) => x.id,
                    tl::enums::Message::Empty(x) => x.id,
                };
                (id, m)
            })
            .collect();

        // Build user map
        let user_map: HashMap<i64, tl::enums::User> = raw
            .users
            .into_iter()
            .filter_map(|u| {
                if let tl::enums::User::User(ref uu) = u {
                    Some((uu.id, u))
                } else {
                    None
                }
            })
            .collect();

        // Build chat map
        let chat_map: HashMap<i64, tl::enums::Chat> = raw
            .chats
            .into_iter()
            .map(|c| {
                let id = match &c {
                    tl::enums::Chat::Chat(x) => x.id,
                    tl::enums::Chat::Forbidden(x) => x.id,
                    tl::enums::Chat::Channel(x) => x.id,
                    tl::enums::Chat::ChannelForbidden(x) => x.id,
                    tl::enums::Chat::Empty(x) => x.id,
                };
                (id, c)
            })
            .collect();

        // Cache peers for future access_hash lookups
        {
            let u_list: Vec<tl::enums::User> = user_map.values().cloned().collect();
            let c_list: Vec<tl::enums::Chat> = chat_map.values().cloned().collect();
            self.cache_users_and_chats(&u_list, &c_list).await;
        }

        let result = raw
            .dialogs
            .into_iter()
            .map(|d| {
                let top_id = match &d {
                    tl::enums::Dialog::Dialog(x) => x.top_message,
                    _ => 0,
                };
                let peer = match &d {
                    tl::enums::Dialog::Dialog(x) => Some(&x.peer),
                    _ => None,
                };

                let message = msg_map.get(&top_id).cloned();
                let entity = peer.and_then(|p| match p {
                    tl::enums::Peer::User(u) => user_map.get(&u.user_id).cloned(),
                    _ => None,
                });
                let chat = peer.and_then(|p| match p {
                    tl::enums::Peer::Chat(c) => chat_map.get(&c.chat_id).cloned(),
                    tl::enums::Peer::Channel(c) => chat_map.get(&c.channel_id).cloned(),
                    _ => None,
                });

                Dialog {
                    raw: d,
                    message,
                    entity,
                    chat,
                }
            })
            .collect();

        Ok(result)
    }

    /// Internal helper: fetch dialogs with a custom GetDialogs request.
    #[allow(dead_code)]
    async fn get_dialogs_raw(
        &self,
        req: tl::functions::messages::GetDialogs,
    ) -> Result<Vec<Dialog>, InvocationError> {
        let body = self.rpc_call_raw(&req).await?;
        let mut cur = Cursor::from_slice(&body);
        let raw = match tl::enums::messages::Dialogs::deserialize(&mut cur)? {
            tl::enums::messages::Dialogs::Dialogs(d) => d,
            tl::enums::messages::Dialogs::Slice(d) => tl::types::messages::Dialogs {
                dialogs: d.dialogs,
                messages: d.messages,
                chats: d.chats,
                users: d.users,
            },
            tl::enums::messages::Dialogs::NotModified(_) => return Ok(vec![]),
        };

        let msg_map: HashMap<i32, tl::enums::Message> = raw
            .messages
            .into_iter()
            .map(|m| {
                let id = match &m {
                    tl::enums::Message::Message(x) => x.id,
                    tl::enums::Message::Service(x) => x.id,
                    tl::enums::Message::Empty(x) => x.id,
                };
                (id, m)
            })
            .collect();

        let user_map: HashMap<i64, tl::enums::User> = raw
            .users
            .into_iter()
            .filter_map(|u| {
                if let tl::enums::User::User(ref uu) = u {
                    Some((uu.id, u))
                } else {
                    None
                }
            })
            .collect();

        let chat_map: HashMap<i64, tl::enums::Chat> = raw
            .chats
            .into_iter()
            .map(|c| {
                let id = match &c {
                    tl::enums::Chat::Chat(x) => x.id,
                    tl::enums::Chat::Forbidden(x) => x.id,
                    tl::enums::Chat::Channel(x) => x.id,
                    tl::enums::Chat::ChannelForbidden(x) => x.id,
                    tl::enums::Chat::Empty(x) => x.id,
                };
                (id, c)
            })
            .collect();

        {
            let u_list: Vec<tl::enums::User> = user_map.values().cloned().collect();
            let c_list: Vec<tl::enums::Chat> = chat_map.values().cloned().collect();
            self.cache_users_and_chats(&u_list, &c_list).await;
        }

        let result = raw
            .dialogs
            .into_iter()
            .map(|d| {
                let top_id = match &d {
                    tl::enums::Dialog::Dialog(x) => x.top_message,
                    _ => 0,
                };
                let peer = match &d {
                    tl::enums::Dialog::Dialog(x) => Some(&x.peer),
                    _ => None,
                };

                let message = msg_map.get(&top_id).cloned();
                let entity = peer.and_then(|p| match p {
                    tl::enums::Peer::User(u) => user_map.get(&u.user_id).cloned(),
                    _ => None,
                });
                let chat = peer.and_then(|p| match p {
                    tl::enums::Peer::Chat(c) => chat_map.get(&c.chat_id).cloned(),
                    tl::enums::Peer::Channel(c) => chat_map.get(&c.channel_id).cloned(),
                    _ => None,
                });

                Dialog {
                    raw: d,
                    message,
                    entity,
                    chat,
                }
            })
            .collect();

        Ok(result)
    }

    /// Like `get_dialogs_raw` but also returns the total count from `messages.DialogsSlice`.
    async fn get_dialogs_raw_with_count(
        &self,
        req: tl::functions::messages::GetDialogs,
    ) -> Result<(Vec<Dialog>, Option<i32>), InvocationError> {
        let body = self.rpc_call_raw(&req).await?;
        let mut cur = Cursor::from_slice(&body);
        let (raw, count) = match tl::enums::messages::Dialogs::deserialize(&mut cur)? {
            tl::enums::messages::Dialogs::Dialogs(d) => (d, None),
            tl::enums::messages::Dialogs::Slice(d) => {
                let cnt = Some(d.count);
                (
                    tl::types::messages::Dialogs {
                        dialogs: d.dialogs,
                        messages: d.messages,
                        chats: d.chats,
                        users: d.users,
                    },
                    cnt,
                )
            }
            tl::enums::messages::Dialogs::NotModified(_) => return Ok((vec![], None)),
        };

        let msg_map: HashMap<i32, tl::enums::Message> = raw
            .messages
            .into_iter()
            .map(|m| {
                let id = match &m {
                    tl::enums::Message::Message(x) => x.id,
                    tl::enums::Message::Service(x) => x.id,
                    tl::enums::Message::Empty(x) => x.id,
                };
                (id, m)
            })
            .collect();

        let user_map: HashMap<i64, tl::enums::User> = raw
            .users
            .into_iter()
            .filter_map(|u| {
                if let tl::enums::User::User(ref uu) = u {
                    Some((uu.id, u))
                } else {
                    None
                }
            })
            .collect();

        let chat_map: HashMap<i64, tl::enums::Chat> = raw
            .chats
            .into_iter()
            .map(|c| {
                let id = match &c {
                    tl::enums::Chat::Chat(x) => x.id,
                    tl::enums::Chat::Forbidden(x) => x.id,
                    tl::enums::Chat::Channel(x) => x.id,
                    tl::enums::Chat::ChannelForbidden(x) => x.id,
                    tl::enums::Chat::Empty(x) => x.id,
                };
                (id, c)
            })
            .collect();

        {
            let u_list: Vec<tl::enums::User> = user_map.values().cloned().collect();
            let c_list: Vec<tl::enums::Chat> = chat_map.values().cloned().collect();
            self.cache_users_and_chats(&u_list, &c_list).await;
        }

        let result = raw
            .dialogs
            .into_iter()
            .map(|d| {
                let top_id = match &d {
                    tl::enums::Dialog::Dialog(x) => x.top_message,
                    _ => 0,
                };
                let peer = match &d {
                    tl::enums::Dialog::Dialog(x) => Some(&x.peer),
                    _ => None,
                };
                let message = msg_map.get(&top_id).cloned();
                let entity = peer.and_then(|p| match p {
                    tl::enums::Peer::User(u) => user_map.get(&u.user_id).cloned(),
                    _ => None,
                });
                let chat = peer.and_then(|p| match p {
                    tl::enums::Peer::Chat(c) => chat_map.get(&c.chat_id).cloned(),
                    tl::enums::Peer::Channel(c) => chat_map.get(&c.channel_id).cloned(),
                    _ => None,
                });
                Dialog {
                    raw: d,
                    message,
                    entity,
                    chat,
                }
            })
            .collect();

        Ok((result, count))
    }

    /// Like `get_messages` but also returns the total count from `messages.Slice`.
    async fn get_messages_with_count(
        &self,
        peer: tl::enums::InputPeer,
        limit: i32,
        offset_id: i32,
    ) -> Result<(Vec<update::IncomingMessage>, Option<i32>), InvocationError> {
        let req = tl::functions::messages::GetHistory {
            peer,
            offset_id,
            offset_date: 0,
            add_offset: 0,
            limit,
            max_id: 0,
            min_id: 0,
            hash: 0,
        };
        let body = self.rpc_call_raw(&req).await?;
        let mut cur = Cursor::from_slice(&body);
        let (msgs, count) = match tl::enums::messages::Messages::deserialize(&mut cur)? {
            tl::enums::messages::Messages::Messages(m) => (m.messages, None),
            tl::enums::messages::Messages::Slice(m) => {
                let cnt = Some(m.count);
                (m.messages, cnt)
            }
            tl::enums::messages::Messages::ChannelMessages(m) => (m.messages, Some(m.count)),
            tl::enums::messages::Messages::NotModified(_) => (vec![], None),
        };
        Ok((
            msgs.into_iter()
                .map(|m| update::IncomingMessage::from_raw(m).with_client(self.clone()))
                .collect(),
            count,
        ))
    }

    /// Download all bytes of a media attachment and save them to `path`.
    ///
    /// # Example
    /// ```rust,no_run
    /// # async fn f(client: ferogram::Client, msg: ferogram::update::IncomingMessage) -> Result<(), Box<dyn std::error::Error>> {
    /// if let Some(loc) = msg.download_location() {
    /// client.download_media_to_file(loc, "/tmp/file.jpg").await?;
    /// }
    /// # Ok(()) }
    /// ```
    pub async fn download_media_to_file(
        &self,
        location: tl::enums::InputFileLocation,
        path: impl AsRef<std::path::Path>,
    ) -> Result<(), InvocationError> {
        let bytes = self.download_media(location).await?;
        std::fs::write(path, &bytes).map_err(InvocationError::Io)?;
        Ok(())
    }

    pub async fn delete_dialog(&self, peer: impl Into<PeerRef>) -> Result<(), InvocationError> {
        let peer = peer.into().resolve(self).await?;
        let input_peer = self.inner.peer_cache.read().await.peer_to_input(&peer);
        let req = tl::functions::messages::DeleteHistory {
            just_clear: false,
            revoke: false,
            peer: input_peer,
            max_id: 0,
            min_date: None,
            max_date: None,
        };
        self.rpc_write(&req).await
    }

    /// Mark all messages in a chat as read.
    pub async fn mark_as_read(&self, peer: impl Into<PeerRef>) -> Result<(), InvocationError> {
        let peer = peer.into().resolve(self).await?;
        let input_peer = self.inner.peer_cache.read().await.peer_to_input(&peer);
        match &input_peer {
            tl::enums::InputPeer::Channel(c) => {
                let req = tl::functions::channels::ReadHistory {
                    channel: tl::enums::InputChannel::InputChannel(tl::types::InputChannel {
                        channel_id: c.channel_id,
                        access_hash: c.access_hash,
                    }),
                    max_id: 0,
                };
                self.rpc_call_raw(&req).await?;
            }
            _ => {
                let req = tl::functions::messages::ReadHistory {
                    peer: input_peer,
                    max_id: 0,
                };
                self.rpc_call_raw(&req).await?;
            }
        }
        Ok(())
    }

    /// Clear unread mention markers.
    pub async fn clear_mentions(&self, peer: impl Into<PeerRef>) -> Result<(), InvocationError> {
        let peer = peer.into().resolve(self).await?;
        let input_peer = self.inner.peer_cache.read().await.peer_to_input(&peer);
        let req = tl::functions::messages::ReadMentions {
            peer: input_peer,
            top_msg_id: None,
        };
        self.rpc_write(&req).await
    }

    // Chat actions (typing, etc)

    /// Send a chat action (typing indicator, uploading photo, etc).
    ///
    /// For "typing" use `tl::enums::SendMessageAction::Typing`.
    /// For forum topic support use [`send_chat_action_ex`](Self::send_chat_action_ex)
    /// or the [`typing_in_topic`](Self::typing_in_topic) helper.
    pub async fn send_chat_action(
        &self,
        peer: impl Into<PeerRef>,
        action: tl::enums::SendMessageAction,
    ) -> Result<(), InvocationError> {
        let peer = peer.into().resolve(self).await?;
        self.send_chat_action_ex(peer, action, None).await
    }

    // Join / invite links

    /// Join a public chat or channel by username/peer.
    pub async fn join_chat(&self, peer: impl Into<PeerRef>) -> Result<(), InvocationError> {
        let peer = peer.into().resolve(self).await?;
        let input_peer = self.inner.peer_cache.read().await.peer_to_input(&peer);
        match input_peer {
            tl::enums::InputPeer::Channel(c) => {
                let req = tl::functions::channels::JoinChannel {
                    channel: tl::enums::InputChannel::InputChannel(tl::types::InputChannel {
                        channel_id: c.channel_id,
                        access_hash: c.access_hash,
                    }),
                };
                self.rpc_call_raw(&req).await?;
            }
            tl::enums::InputPeer::Chat(c) => {
                let req = tl::functions::messages::AddChatUser {
                    chat_id: c.chat_id,
                    user_id: tl::enums::InputUser::UserSelf,
                    fwd_limit: 0,
                };
                self.rpc_call_raw(&req).await?;
            }
            _ => {
                return Err(InvocationError::Deserialize(
                    "cannot join this peer type".into(),
                ));
            }
        }
        Ok(())
    }

    /// Accept and join via an invite link.
    pub async fn accept_invite_link(&self, link: &str) -> Result<(), InvocationError> {
        let hash = Self::parse_invite_hash(link)
            .ok_or_else(|| InvocationError::Deserialize(format!("invalid invite link: {link}")))?;
        let req = tl::functions::messages::ImportChatInvite {
            hash: hash.to_string(),
        };
        self.rpc_write(&req).await
    }

    /// Extract hash from `https://t.me/+HASH` or `https://t.me/joinchat/HASH`.
    pub fn parse_invite_hash(link: &str) -> Option<&str> {
        if let Some(pos) = link.find("/+") {
            return Some(&link[pos + 2..]);
        }
        if let Some(pos) = link.find("/joinchat/") {
            return Some(&link[pos + 10..]);
        }
        None
    }

    // Message history (paginated)

    /// Fetch a page of messages from a peer's history.
    pub async fn get_messages(
        &self,
        peer: tl::enums::InputPeer,
        limit: i32,
        offset_id: i32,
    ) -> Result<Vec<update::IncomingMessage>, InvocationError> {
        let req = tl::functions::messages::GetHistory {
            peer,
            offset_id,
            offset_date: 0,
            add_offset: 0,
            limit,
            max_id: 0,
            min_id: 0,
            hash: 0,
        };
        let body = self.rpc_call_raw(&req).await?;
        let mut cur = Cursor::from_slice(&body);
        let msgs = match tl::enums::messages::Messages::deserialize(&mut cur)? {
            tl::enums::messages::Messages::Messages(m) => m.messages,
            tl::enums::messages::Messages::Slice(m) => m.messages,
            tl::enums::messages::Messages::ChannelMessages(m) => m.messages,
            tl::enums::messages::Messages::NotModified(_) => vec![],
        };
        Ok(msgs
            .into_iter()
            .map(|m| update::IncomingMessage::from_raw(m).with_client(self.clone()))
            .collect())
    }

    // Peer resolution

    /// Resolve a peer string to a [`tl::enums::Peer`].
    pub async fn resolve_peer(&self, peer: &str) -> Result<tl::enums::Peer, InvocationError> {
        match peer.trim() {
            "me" | "self" => Ok(tl::enums::Peer::User(tl::types::PeerUser { user_id: 0 })),
            username if username.starts_with('@') => self.resolve_username(&username[1..]).await,
            id_str => {
                if let Ok(id) = id_str.parse::<i64>() {
                    Ok(tl::enums::Peer::User(tl::types::PeerUser { user_id: id }))
                } else {
                    Err(InvocationError::Deserialize(format!(
                        "cannot resolve peer: {peer}"
                    )))
                }
            }
        }
    }

    /// Resolve a Telegram username to a [`tl::enums::Peer`] and cache the access hash.
    ///
    /// Also accepts usernames without the leading `@`.
    pub async fn resolve_username(
        &self,
        username: &str,
    ) -> Result<tl::enums::Peer, InvocationError> {
        let req = tl::functions::contacts::ResolveUsername {
            username: username.to_string(),
            referer: None,
        };
        let body = self.rpc_call_raw(&req).await?;
        let mut cur = Cursor::from_slice(&body);
        let tl::enums::contacts::ResolvedPeer::ResolvedPeer(resolved) =
            tl::enums::contacts::ResolvedPeer::deserialize(&mut cur)?;
        // Cache users and chats from the resolution
        self.cache_users_slice(&resolved.users).await;
        self.cache_chats_slice(&resolved.chats).await;
        Ok(resolved.peer)
    }

    // Raw invoke

    /// Invoke any TL function directly, handling flood-wait retries.
    /// Spawn a background `GetFutureSalts` if one is not already in flight.
    ///
    /// Called from `do_rpc_call` (proactive, pool size <= 1) and from the
    /// `bad_server_salt` handler (reactive, after salt pool reset).
    ///
    fn spawn_salt_fetch_if_needed(&self) {
        if self
            .inner
            .salt_request_in_flight
            .compare_exchange(
                false,
                true,
                std::sync::atomic::Ordering::SeqCst,
                std::sync::atomic::Ordering::SeqCst,
            )
            .is_err()
        {
            return; // already in flight
        }
        let inner = Arc::clone(&self.inner);
        tokio::spawn(async move {
            tracing::debug!("[ferogram] proactive GetFutureSalts spawned");
            let mut req_body = Vec::with_capacity(8);
            req_body.extend_from_slice(&0xb921bd04_u32.to_le_bytes()); // get_future_salts
            req_body.extend_from_slice(&64_i32.to_le_bytes()); // num
            let (wire, fk, fs_msg_id) = {
                let mut w = inner.writer.lock().await;
                let fk = w.frame_kind.clone();
                let (wire, id) = w.enc.pack_body_with_msg_id(&req_body, true);
                w.sent_bodies.insert(id, req_body);
                (wire, fk, id)
            };
            let (tx, rx) = tokio::sync::oneshot::channel();
            inner.pending.lock().await.insert(fs_msg_id, tx);
            let send_ok = {
                send_frame_write(&mut *inner.write_half.lock().await, &wire, &fk)
                    .await
                    .is_ok()
            };
            if !send_ok {
                inner.pending.lock().await.remove(&fs_msg_id);
                inner.writer.lock().await.sent_bodies.remove(&fs_msg_id);
                inner
                    .salt_request_in_flight
                    .store(false, std::sync::atomic::Ordering::SeqCst);
                return;
            }
            let _ = rx.await;
            inner
                .salt_request_in_flight
                .store(false, std::sync::atomic::Ordering::SeqCst);
        });
    }

    pub async fn invoke<R: RemoteCall>(&self, req: &R) -> Result<R::Return, InvocationError> {
        let body = self.rpc_call_raw(req).await?;
        let mut cur = Cursor::from_slice(&body);
        R::Return::deserialize(&mut cur).map_err(Into::into)
    }

    async fn rpc_call_raw<R: RemoteCall>(&self, req: &R) -> Result<Vec<u8>, InvocationError> {
        let mut rl = RetryLoop::new(Arc::clone(&self.inner.retry_policy));
        let mut auth_key_retries = 0u32;
        loop {
            match self.do_rpc_call(req).await {
                Ok(body) => return Ok(body),
                Err(e) if e.migrate_dc_id().is_some() => {
                    // Telegram is redirecting us to a different DC.
                    // Migrate transparently and retry: no error surfaces to caller.
                    self.migrate_to(e.migrate_dc_id().unwrap()).await?;
                }
                // AUTH_KEY_UNREGISTERED (401): the reader loop already handles fresh DH +
                // reconnect on its own.  We must NOT call shutdown()/network_hint here
                // that would race with the reader and destroy the new key it just built.
                // Instead, log the attempt, surface as ConnectionReset so RetryLoop backs
                // off, and let the reader finish the reconnect before the next retry fires.
                // After 3 attempts we give up and surface the real RPC error.
                Err(InvocationError::Rpc(ref r)) if r.code == 401 => {
                    auth_key_retries += 1;
                    if auth_key_retries > 3 {
                        return Err(InvocationError::Rpc(r.clone()));
                    }
                    tracing::warn!(
                        "[ferogram] AUTH_KEY_UNREGISTERED on invoke (attempt {}/3): waiting for reader reconnect",
                        auth_key_retries
                    );
                    rl.advance(InvocationError::Io(std::io::Error::new(
                        std::io::ErrorKind::ConnectionReset,
                        r.to_string(),
                    )))
                    .await?;
                }
                Err(e) => rl.advance(e).await?,
            }
        }
    }

    /// Send an RPC call and await the response via a oneshot channel.
    ///
    /// This is the core of the split-stream design:
    ///1. Pack the request and get its msg_id.
    ///2. Register a oneshot Sender in the pending map (BEFORE sending).
    ///3. Send the frame while holding the writer lock.
    ///4. Release the writer lock immediately: the reader task now runs freely.
    ///5. Await the oneshot Receiver; the reader task will fulfill it when
    ///   the matching rpc_result frame arrives.
    async fn do_rpc_call<R: RemoteCall>(&self, req: &R) -> Result<Vec<u8>, InvocationError> {
        let (tx, rx) = oneshot::channel();
        let wire = {
            let raw_body = req.to_bytes();
            // compress large outgoing bodies
            let body = maybe_gz_pack(&raw_body);

            let mut w = self.inner.writer.lock().await;

            // Proactive salt cycling on every send (: Encrypted::push() prelude).
            // Prunes expired salts, cycles enc.salt to newest usable entry,
            // and triggers a background GetFutureSalts when pool shrinks to 1.
            if w.advance_salt_if_needed() {
                drop(w); // release lock before spawning
                self.spawn_salt_fetch_if_needed();
                w = self.inner.writer.lock().await;
            }

            let fk = w.frame_kind.clone();

            // +: drain any pending acks; if non-empty bundle them with
            // the request in a MessageContainer so acks piggyback on every send.
            let acks: Vec<i64> = w.pending_ack.drain(..).collect();

            if acks.is_empty() {
                // Simple path: standalone request
                let (wire, msg_id) = w.enc.pack_body_with_msg_id(&body, true);
                w.sent_bodies.insert(msg_id, body); //
                self.inner.pending.lock().await.insert(msg_id, tx);
                (wire, fk)
            } else {
                // container path: [MsgsAck, request]
                let ack_body = build_msgs_ack_body(&acks);
                let (ack_msg_id, ack_seqno) = w.enc.alloc_msg_seqno(false); // non-content
                let (req_msg_id, req_seqno) = w.enc.alloc_msg_seqno(true); // content

                let container_payload = build_container_body(&[
                    (ack_msg_id, ack_seqno, ack_body.as_slice()),
                    (req_msg_id, req_seqno, body.as_slice()),
                ]);

                let (wire, container_msg_id) = w.enc.pack_container(&container_payload);

                w.sent_bodies.insert(req_msg_id, body); //
                w.container_map.insert(container_msg_id, req_msg_id); // 
                self.inner.pending.lock().await.insert(req_msg_id, tx);
                tracing::debug!(
                    "[ferogram] container: bundled {} acks + request (cid={container_msg_id})",
                    acks.len()
                );
                (wire, fk)
            }
        };
        // TCP send with writer lock free: reader can push pending_ack concurrently
        send_frame_write(&mut *self.inner.write_half.lock().await, &wire.0, &wire.1).await?;
        match rx.await {
            Ok(result) => result,
            Err(_) => Err(InvocationError::Deserialize(
                "RPC channel closed (reader died?)".into(),
            )),
        }
    }

    /// Like `rpc_call_raw` but for write RPCs (Serializable, return type is Updates).
    /// Uses the same oneshot mechanism: the reader task signals success/failure.
    async fn rpc_write<S: tl::Serializable>(&self, req: &S) -> Result<(), InvocationError> {
        let mut fail_count = NonZeroU32::new(1).unwrap();
        let mut slept_so_far = Duration::default();
        loop {
            let result = self.do_rpc_write(req).await;
            match result {
                Ok(()) => return Ok(()),
                Err(e) => {
                    let ctx = RetryContext {
                        fail_count,
                        slept_so_far,
                        error: e,
                    };
                    match self.inner.retry_policy.should_retry(&ctx) {
                        ControlFlow::Continue(delay) => {
                            sleep(delay).await;
                            slept_so_far += delay;
                            fail_count = fail_count.saturating_add(1);
                        }
                        ControlFlow::Break(()) => return Err(ctx.error),
                    }
                }
            }
        }
    }

    async fn do_rpc_write<S: tl::Serializable>(&self, req: &S) -> Result<(), InvocationError> {
        let (tx, rx) = oneshot::channel();
        let wire = {
            let raw_body = req.to_bytes();
            // compress large outgoing bodies
            let body = maybe_gz_pack(&raw_body);

            let mut w = self.inner.writer.lock().await;
            let fk = w.frame_kind.clone();

            // +: drain pending acks and bundle into container if any
            let acks: Vec<i64> = w.pending_ack.drain(..).collect();

            if acks.is_empty() {
                let (wire, msg_id) = w.enc.pack_body_with_msg_id(&body, true);
                w.sent_bodies.insert(msg_id, body); //
                self.inner.pending.lock().await.insert(msg_id, tx);
                (wire, fk)
            } else {
                let ack_body = build_msgs_ack_body(&acks);
                let (ack_msg_id, ack_seqno) = w.enc.alloc_msg_seqno(false);
                let (req_msg_id, req_seqno) = w.enc.alloc_msg_seqno(true);
                let container_payload = build_container_body(&[
                    (ack_msg_id, ack_seqno, ack_body.as_slice()),
                    (req_msg_id, req_seqno, body.as_slice()),
                ]);
                let (wire, container_msg_id) = w.enc.pack_container(&container_payload);
                w.sent_bodies.insert(req_msg_id, body); //
                w.container_map.insert(container_msg_id, req_msg_id); // 
                self.inner.pending.lock().await.insert(req_msg_id, tx);
                tracing::debug!(
                    "[ferogram] write container: bundled {} acks + write (cid={container_msg_id})",
                    acks.len()
                );
                (wire, fk)
            }
        };
        send_frame_write(&mut *self.inner.write_half.lock().await, &wire.0, &wire.1).await?;
        match rx.await {
            Ok(result) => result.map(|_| ()),
            Err(_) => Err(InvocationError::Deserialize(
                "rpc_write channel closed".into(),
            )),
        }
    }

    async fn init_connection(&self) -> Result<(), InvocationError> {
        use tl::functions::{InitConnection, InvokeWithLayer, help::GetConfig};
        let req = InvokeWithLayer {
            layer: tl::LAYER,
            query: InitConnection {
                api_id: self.inner.api_id,
                device_model: self.inner.device_model.clone(),
                system_version: self.inner.system_version.clone(),
                app_version: self.inner.app_version.clone(),
                system_lang_code: self.inner.system_lang_code.clone(),
                lang_pack: self.inner.lang_pack.clone(),
                lang_code: self.inner.lang_code.clone(),
                proxy: None,
                params: None,
                query: GetConfig {},
            },
        };

        // Use the split-writer oneshot path (reader task routes the response).
        let body = self.rpc_call_raw_serializable(&req).await?;

        let mut cur = Cursor::from_slice(&body);
        if let Ok(tl::enums::Config::Config(cfg)) = tl::enums::Config::deserialize(&mut cur) {
            let allow_ipv6 = self.inner.allow_ipv6;
            let mut opts = self.inner.dc_options.lock().await;
            let mut media_opts = self.inner.media_dc_options.lock().await;
            for opt in &cfg.dc_options {
                let tl::enums::DcOption::DcOption(o) = opt;
                if o.ipv6 && !allow_ipv6 {
                    continue;
                }
                let addr = format!("{}:{}", o.ip_address, o.port);
                let mut flags = DcFlags::NONE;
                if o.ipv6 {
                    flags.set(DcFlags::IPV6);
                }
                if o.media_only {
                    flags.set(DcFlags::MEDIA_ONLY);
                }
                if o.tcpo_only {
                    flags.set(DcFlags::TCPO_ONLY);
                }
                if o.cdn {
                    flags.set(DcFlags::CDN);
                }
                if o.r#static {
                    flags.set(DcFlags::STATIC);
                }

                if o.media_only || o.cdn {
                    let e = media_opts.entry(o.id).or_insert_with(|| DcEntry {
                        dc_id: o.id,
                        addr: addr.clone(),
                        auth_key: None,
                        first_salt: 0,
                        time_offset: 0,
                        flags,
                    });
                    e.addr = addr;
                    e.flags = flags;
                } else if !o.tcpo_only {
                    let e = opts.entry(o.id).or_insert_with(|| DcEntry {
                        dc_id: o.id,
                        addr: addr.clone(),
                        auth_key: None,
                        first_salt: 0,
                        time_offset: 0,
                        flags,
                    });
                    e.addr = addr;
                    e.flags = flags;
                }
            }
            tracing::info!(
                "[ferogram] initConnection ✓  ({} DCs, ipv6={})",
                cfg.dc_options.len(),
                allow_ipv6
            );
        }
        Ok(())
    }

    async fn migrate_to(&self, new_dc_id: i32) -> Result<(), InvocationError> {
        let addr = {
            let opts = self.inner.dc_options.lock().await;
            opts.get(&new_dc_id)
                .map(|e| e.addr.clone())
                .unwrap_or_else(|| crate::dc_migration::fallback_dc_addr(new_dc_id).to_string())
        };
        tracing::info!("[ferogram] Migrating to DC{new_dc_id} ({addr}) …");

        let saved_key = {
            let opts = self.inner.dc_options.lock().await;
            opts.get(&new_dc_id).and_then(|e| e.auth_key)
        };

        let socks5 = self.inner.socks5.clone();
        let mtproxy = self.inner.mtproxy.clone();
        let transport = self.inner.transport.clone();
        let conn = if let Some(key) = saved_key {
            Connection::connect_with_key(
                &addr,
                key,
                0,
                0,
                socks5.as_ref(),
                &transport,
                new_dc_id as i16,
            )
            .await?
        } else {
            Connection::connect_raw(
                &addr,
                socks5.as_ref(),
                mtproxy.as_ref(),
                &transport,
                new_dc_id as i16,
            )
            .await?
        };

        let new_key = conn.auth_key_bytes();
        {
            let mut opts = self.inner.dc_options.lock().await;
            let entry = opts.entry(new_dc_id).or_insert_with(|| DcEntry {
                dc_id: new_dc_id,
                addr: addr.clone(),
                auth_key: None,
                first_salt: 0,
                time_offset: 0,
                flags: DcFlags::NONE,
            });
            entry.auth_key = Some(new_key);
        }

        // Split the new connection and replace writer + read half.
        let (new_writer, new_wh, new_read, new_fk) = conn.into_writer();
        let new_ak = new_writer.enc.auth_key_bytes();
        let new_sid = new_writer.enc.session_id();
        *self.inner.writer.lock().await = new_writer;
        *self.inner.write_half.lock().await = new_wh;
        *self.inner.home_dc_id.lock().await = new_dc_id;

        // Hand the new read half to the reader task FIRST so it can route
        // the upcoming init_connection RPC response.
        let _ = self
            .inner
            .reconnect_tx
            .send((new_read, new_fk, new_ak, new_sid));

        // migrate_to() is called from user-facing methods (bot_sign_in,
        // request_login_code, sign_in): NOT from inside the reader loop.
        // The reader task is a separate tokio task running concurrently, so
        // awaiting init_connection() here is safe: the reader is free to route
        // the RPC response while we wait. We must await before returning so
        // the caller can safely retry the original request on the new DC.
        //
        // Respect FLOOD_WAIT: if Telegram rate-limits init, wait and retry
        // rather than returning an error that would abort the whole auth flow.
        loop {
            match self.init_connection().await {
                Ok(()) => break,
                Err(InvocationError::Rpc(ref r)) if r.flood_wait_seconds().is_some() => {
                    let secs = r.flood_wait_seconds().unwrap();
                    tracing::warn!(
                        "[ferogram] migrate_to DC{new_dc_id}: init FLOOD_WAIT_{secs}: waiting"
                    );
                    sleep(Duration::from_secs(secs + 1)).await;
                }
                Err(e) => return Err(e),
            }
        }

        self.save_session().await.ok();
        tracing::info!("[ferogram] Now on DC{new_dc_id} ✓");
        Ok(())
    }

    /// Gracefully shut down the client.
    ///
    /// Signals the reader task to exit cleanly. Same as cancelling the
    /// [`ShutdownToken`] returned from [`Client::connect`].
    ///
    /// In-flight RPCs will receive a `Dropped` error. Call `save_session()`
    /// before this if you want to persist the current auth state.
    pub fn disconnect(&self) {
        self.inner.shutdown_token.cancel();
    }

    /// Sync the internal pts/qts/seq/date state with the Telegram server.
    ///
    /// Called automatically on `connect()`. Call it manually if you
    /// need to reset the update gap-detection counters, e.g. after resuming
    /// from a long hibernation.
    pub async fn sync_update_state(&self) {
        let _ = self.sync_pts_state().await;
    }

    async fn cache_user(&self, user: &tl::enums::User) {
        self.inner.peer_cache.write().await.cache_user(user);
    }

    async fn cache_users_slice(&self, users: &[tl::enums::User]) {
        let mut cache = self.inner.peer_cache.write().await;
        cache.cache_users(users);
    }

    async fn cache_chats_slice(&self, chats: &[tl::enums::Chat]) {
        let mut cache = self.inner.peer_cache.write().await;
        cache.cache_chats(chats);
    }

    /// Cache users and chats in a single write-lock acquisition.
    async fn cache_users_and_chats(&self, users: &[tl::enums::User], chats: &[tl::enums::Chat]) {
        let mut cache = self.inner.peer_cache.write().await;
        cache.cache_users(users);
        cache.cache_chats(chats);
    }

    #[doc(hidden)]
    pub async fn cache_users_slice_pub(&self, users: &[tl::enums::User]) {
        self.cache_users_slice(users).await;
    }

    #[doc(hidden)]
    pub async fn cache_chats_slice_pub(&self, chats: &[tl::enums::Chat]) {
        self.cache_chats_slice(chats).await;
    }

    /// Public RPC call for use by sub-modules.
    #[doc(hidden)]
    pub async fn rpc_call_raw_pub<R: ferogram_tl_types::RemoteCall>(
        &self,
        req: &R,
    ) -> Result<Vec<u8>, InvocationError> {
        self.rpc_call_raw(req).await
    }

    /// Like rpc_call_raw but takes a Serializable (for InvokeWithLayer wrappers).
    async fn rpc_call_raw_serializable<S: tl::Serializable>(
        &self,
        req: &S,
    ) -> Result<Vec<u8>, InvocationError> {
        let mut fail_count = NonZeroU32::new(1).unwrap();
        let mut slept_so_far = Duration::default();
        loop {
            match self.do_rpc_write_returning_body(req).await {
                Ok(body) => return Ok(body),
                Err(e) => {
                    let ctx = RetryContext {
                        fail_count,
                        slept_so_far,
                        error: e,
                    };
                    match self.inner.retry_policy.should_retry(&ctx) {
                        ControlFlow::Continue(delay) => {
                            sleep(delay).await;
                            slept_so_far += delay;
                            fail_count = fail_count.saturating_add(1);
                        }
                        ControlFlow::Break(()) => return Err(ctx.error),
                    }
                }
            }
        }
    }

    async fn do_rpc_write_returning_body<S: tl::Serializable>(
        &self,
        req: &S,
    ) -> Result<Vec<u8>, InvocationError> {
        let (tx, rx) = oneshot::channel();
        let wire = {
            let raw_body = req.to_bytes();
            let body = maybe_gz_pack(&raw_body); //
            let mut w = self.inner.writer.lock().await;
            let fk = w.frame_kind.clone();
            let acks: Vec<i64> = w.pending_ack.drain(..).collect(); // 
            if acks.is_empty() {
                let (wire, msg_id) = w.enc.pack_body_with_msg_id(&body, true);
                w.sent_bodies.insert(msg_id, body); //
                self.inner.pending.lock().await.insert(msg_id, tx);
                (wire, fk)
            } else {
                let ack_body = build_msgs_ack_body(&acks);
                let (ack_msg_id, ack_seqno) = w.enc.alloc_msg_seqno(false);
                let (req_msg_id, req_seqno) = w.enc.alloc_msg_seqno(true);
                let container_payload = build_container_body(&[
                    (ack_msg_id, ack_seqno, ack_body.as_slice()),
                    (req_msg_id, req_seqno, body.as_slice()),
                ]);
                let (wire, container_msg_id) = w.enc.pack_container(&container_payload);
                w.sent_bodies.insert(req_msg_id, body); //
                w.container_map.insert(container_msg_id, req_msg_id); // 
                self.inner.pending.lock().await.insert(req_msg_id, tx);
                (wire, fk)
            }
        };
        send_frame_write(&mut *self.inner.write_half.lock().await, &wire.0, &wire.1).await?;
        match rx.await {
            Ok(result) => result,
            Err(_) => Err(InvocationError::Deserialize("rpc channel closed".into())),
        }
    }

    pub async fn count_channels(&self) -> Result<usize, InvocationError> {
        let mut iter = self.iter_dialogs();
        let mut count = 0usize;
        while let Some(dialog) = iter.next(self).await? {
            if matches!(dialog.peer(), Some(tl::enums::Peer::Channel(_))) {
                count += 1;
            }
        }
        Ok(count)
    }

    ///
    /// Returns a [`DialogIter`] that can be advanced with [`DialogIter::next`].
    /// Lets you page through all dialogs without loading them all at once.
    ///
    /// # Example
    /// ```rust,no_run
    /// # async fn f(client: ferogram::Client) -> Result<(), Box<dyn std::error::Error>> {
    /// let mut iter = client.iter_dialogs();
    /// while let Some(dialog) = iter.next(&client).await? {
    /// println!("{}", dialog.title());
    /// }
    /// # Ok(()) }
    /// ```
    pub fn iter_dialogs(&self) -> DialogIter {
        DialogIter {
            offset_date: 0,
            offset_id: 0,
            offset_peer: tl::enums::InputPeer::Empty,
            done: false,
            buffer: VecDeque::new(),
            total: None,
        }
    }

    /// Fetch messages from a peer, page by page.
    ///
    /// Returns a [`MessageIter`] that can be advanced with [`MessageIter::next`].
    ///
    /// # Example
    /// ```rust,no_run
    /// # async fn f(client: ferogram::Client, peer: ferogram_tl_types::enums::Peer) -> Result<(), Box<dyn std::error::Error>> {
    /// let mut iter = client.iter_messages(peer);
    /// while let Some(msg) = iter.next(&client).await? {
    /// println!("{:?}", msg.text());
    /// }
    /// # Ok(()) }
    /// ```
    pub fn iter_messages(&self, peer: impl Into<PeerRef>) -> MessageIter {
        MessageIter {
            unresolved: Some(peer.into()),
            peer: None,
            offset_id: 0,
            done: false,
            buffer: VecDeque::new(),
            total: None,
        }
    }

    /// Try to resolve a peer to InputPeer, returning an error if the access_hash
    /// is unknown (i.e. the peer has not been seen in any prior API call).
    pub async fn resolve_to_input_peer(
        &self,
        peer: &tl::enums::Peer,
    ) -> Result<tl::enums::InputPeer, InvocationError> {
        let cache = self.inner.peer_cache.read().await;
        match peer {
            tl::enums::Peer::User(u) => {
                if u.user_id == 0 {
                    return Ok(tl::enums::InputPeer::PeerSelf);
                }
                match cache.users.get(&u.user_id) {
                    Some(&hash) => Ok(tl::enums::InputPeer::User(tl::types::InputPeerUser {
                        user_id: u.user_id,
                        access_hash: hash,
                    })),
                    None => Err(InvocationError::Deserialize(format!(
                        "access_hash unknown for user {}; resolve via username first",
                        u.user_id
                    ))),
                }
            }
            tl::enums::Peer::Chat(c) => Ok(tl::enums::InputPeer::Chat(tl::types::InputPeerChat {
                chat_id: c.chat_id,
            })),
            tl::enums::Peer::Channel(c) => match cache.channels.get(&c.channel_id) {
                Some(&hash) => Ok(tl::enums::InputPeer::Channel(tl::types::InputPeerChannel {
                    channel_id: c.channel_id,
                    access_hash: hash,
                })),
                None => Err(InvocationError::Deserialize(format!(
                    "access_hash unknown for channel {}; resolve via username first",
                    c.channel_id
                ))),
            },
        }
    }

    /// Invoke a request on a specific DC, using the pool.
    ///
    /// If the target DC has no auth key yet, one is acquired via DH and then
    /// authorized via `auth.exportAuthorization` / `auth.importAuthorization`
    /// so the worker DC can serve user-account requests too.
    pub async fn invoke_on_dc<R: RemoteCall>(
        &self,
        dc_id: i32,
        req: &R,
    ) -> Result<R::Return, InvocationError> {
        let body = self.rpc_on_dc_raw(dc_id, req).await?;
        let mut cur = Cursor::from_slice(&body);
        R::Return::deserialize(&mut cur).map_err(Into::into)
    }

    /// Raw RPC call routed to `dc_id`, exporting auth if needed.
    async fn rpc_on_dc_raw<R: RemoteCall>(
        &self,
        dc_id: i32,
        req: &R,
    ) -> Result<Vec<u8>, InvocationError> {
        // Check if we need to open a new connection for this DC
        let needs_new = {
            let pool = self.inner.dc_pool.lock().await;
            !pool.has_connection(dc_id)
        };

        if needs_new {
            let addr = {
                let opts = self.inner.dc_options.lock().await;
                opts.get(&dc_id)
                    .map(|e| e.addr.clone())
                    .unwrap_or_else(|| crate::dc_migration::fallback_dc_addr(dc_id).to_string())
            };

            let socks5 = self.inner.socks5.clone();
            let transport = self.inner.transport.clone();
            let saved_key = {
                let opts = self.inner.dc_options.lock().await;
                opts.get(&dc_id).and_then(|e| e.auth_key)
            };

            let dc_conn = if let Some(key) = saved_key {
                dc_pool::DcConnection::connect_with_key(
                    &addr,
                    key,
                    0,
                    0,
                    socks5.as_ref(),
                    &transport,
                    dc_id as i16,
                )
                .await?
            } else {
                let conn = dc_pool::DcConnection::connect_raw(
                    &addr,
                    socks5.as_ref(),
                    &transport,
                    dc_id as i16,
                )
                .await?;
                // Export auth from home DC and import into worker DC
                let home_dc_id = *self.inner.home_dc_id.lock().await;
                if dc_id != home_dc_id
                    && let Err(e) = self.export_import_auth(dc_id, &conn).await
                {
                    tracing::warn!("[ferogram] Auth export/import for DC{dc_id} failed: {e}");
                }
                conn
            };

            let key = dc_conn.auth_key_bytes();
            {
                let mut opts = self.inner.dc_options.lock().await;
                if let Some(e) = opts.get_mut(&dc_id) {
                    e.auth_key = Some(key);
                }
            }
            self.inner.dc_pool.lock().await.insert(dc_id, dc_conn);
        }

        let dc_entries: Vec<DcEntry> = self
            .inner
            .dc_options
            .lock()
            .await
            .values()
            .cloned()
            .collect();
        self.inner
            .dc_pool
            .lock()
            .await
            .invoke_on_dc(dc_id, &dc_entries, req)
            .await
    }

    /// Export authorization from the home DC and import it into `dc_id`.
    async fn export_import_auth(
        &self,
        dc_id: i32,
        _dc_conn: &dc_pool::DcConnection, // reserved for future direct import
    ) -> Result<(), InvocationError> {
        // Export from home DC
        let export_req = tl::functions::auth::ExportAuthorization { dc_id };
        let body = self.rpc_call_raw(&export_req).await?;
        let mut cur = Cursor::from_slice(&body);
        let tl::enums::auth::ExportedAuthorization::ExportedAuthorization(exported) =
            tl::enums::auth::ExportedAuthorization::deserialize(&mut cur)?;

        // Import into the target DC via the pool
        let import_req = tl::functions::auth::ImportAuthorization {
            id: exported.id,
            bytes: exported.bytes,
        };
        let dc_entries: Vec<DcEntry> = self
            .inner
            .dc_options
            .lock()
            .await
            .values()
            .cloned()
            .collect();
        self.inner
            .dc_pool
            .lock()
            .await
            .invoke_on_dc(dc_id, &dc_entries, &import_req)
            .await?;
        tracing::debug!("[ferogram] Auth exported+imported to DC{dc_id} ✓");
        Ok(())
    }

    async fn get_password_info(&self) -> Result<PasswordToken, InvocationError> {
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

    #[allow(clippy::type_complexity)]
    fn extract_password_params(
        algo: &tl::enums::PasswordKdfAlgo,
    ) -> Result<(&[u8], &[u8], &[u8], i32), InvocationError> {
        match algo {
            tl::enums::PasswordKdfAlgo::Sha256Sha256Pbkdf2Hmacsha512iter100000Sha256ModPow(a) => {
                Ok((&a.salt1, &a.salt2, &a.p, a.g))
            }
            _ => Err(InvocationError::Deserialize(
                "unsupported password KDF algo".into(),
            )),
        }
    }
}

/// Attach an embedded `Client` to `NewMessage` and `MessageEdited` variants.
/// Other update variants are returned unchanged.
pub(crate) fn attach_client_to_update(u: update::Update, client: &Client) -> update::Update {
    match u {
        update::Update::NewMessage(msg) => {
            update::Update::NewMessage(msg.with_client(client.clone()))
        }
        update::Update::MessageEdited(msg) => {
            update::Update::MessageEdited(msg.with_client(client.clone()))
        }
        other => other,
    }
}

/// Cursor-based iterator over dialogs. Created by [`Client::iter_dialogs`].
pub struct DialogIter {
    offset_date: i32,
    offset_id: i32,
    offset_peer: tl::enums::InputPeer,
    done: bool,
    buffer: VecDeque<Dialog>,
    /// Total dialog count as reported by the first server response.
    /// `None` until the first page is fetched.
    pub total: Option<i32>,
}

impl DialogIter {
    const PAGE_SIZE: i32 = 100;

    /// Total number of dialogs as reported by the server on the first page fetch.
    ///
    /// Returns `None` before the first [`next`](Self::next) call, and `None` for
    /// accounts with fewer dialogs than `PAGE_SIZE` (where the server returns
    /// `messages.Dialogs` instead of `messages.DialogsSlice`).
    pub fn total(&self) -> Option<i32> {
        self.total
    }

    /// Fetch the next dialog. Returns `None` when all dialogs have been yielded.
    pub async fn next(&mut self, client: &Client) -> Result<Option<Dialog>, InvocationError> {
        if let Some(d) = self.buffer.pop_front() {
            return Ok(Some(d));
        }
        if self.done {
            return Ok(None);
        }

        let req = tl::functions::messages::GetDialogs {
            exclude_pinned: false,
            folder_id: None,
            offset_date: self.offset_date,
            offset_id: self.offset_id,
            offset_peer: self.offset_peer.clone(),
            limit: Self::PAGE_SIZE,
            hash: 0,
        };

        let (dialogs, count) = client.get_dialogs_raw_with_count(req).await?;
        // Populate total from the first response (messages.DialogsSlice carries a count).
        if self.total.is_none() {
            self.total = count;
        }
        if dialogs.is_empty() || dialogs.len() < Self::PAGE_SIZE as usize {
            self.done = true;
        }

        // Prepare cursor for next page
        if let Some(last) = dialogs.last() {
            self.offset_date = last
                .message
                .as_ref()
                .map(|m| match m {
                    tl::enums::Message::Message(x) => x.date,
                    tl::enums::Message::Service(x) => x.date,
                    _ => 0,
                })
                .unwrap_or(0);
            self.offset_id = last.top_message();
            if let Some(peer) = last.peer() {
                self.offset_peer = client.inner.peer_cache.read().await.peer_to_input(peer);
            }
        }

        self.buffer.extend(dialogs);
        Ok(self.buffer.pop_front())
    }
}

/// Cursor-based iterator over message history. Created by [`Client::iter_messages`].
pub struct MessageIter {
    unresolved: Option<PeerRef>,
    peer: Option<tl::enums::Peer>,
    offset_id: i32,
    done: bool,
    buffer: VecDeque<update::IncomingMessage>,
    /// Total message count from the first server response (messages.Slice).
    /// `None` until the first page is fetched, `None` for `messages.Messages`
    /// (which returns an exact slice with no separate count).
    pub total: Option<i32>,
}

impl MessageIter {
    const PAGE_SIZE: i32 = 100;

    /// Total message count from the first server response.
    ///
    /// Returns `None` before the first [`next`](Self::next) call, or for chats
    /// where the server returns an exact (non-slice) response.
    pub fn total(&self) -> Option<i32> {
        self.total
    }

    /// Fetch the next message (newest first). Returns `None` when all messages have been yielded.
    pub async fn next(
        &mut self,
        client: &Client,
    ) -> Result<Option<update::IncomingMessage>, InvocationError> {
        if let Some(m) = self.buffer.pop_front() {
            return Ok(Some(m));
        }
        if self.done {
            return Ok(None);
        }

        // Resolve PeerRef on first call, then reuse the cached Peer.
        let peer = if let Some(p) = &self.peer {
            p.clone()
        } else {
            let pr = self.unresolved.take().expect("MessageIter: peer not set");
            let p = pr.resolve(client).await?;
            self.peer = Some(p.clone());
            p
        };

        let input_peer = client.inner.peer_cache.read().await.peer_to_input(&peer);
        let (page, count) = client
            .get_messages_with_count(input_peer, Self::PAGE_SIZE, self.offset_id)
            .await?;

        if self.total.is_none() {
            self.total = count;
        }

        if page.is_empty() || page.len() < Self::PAGE_SIZE as usize {
            self.done = true;
        }
        if let Some(last) = page.last() {
            self.offset_id = last.id();
        }

        self.buffer.extend(page);
        Ok(self.buffer.pop_front())
    }
}

/// Public wrapper for `random_i64` used by sub-modules.
#[doc(hidden)]
pub fn random_i64_pub() -> i64 {
    random_i64()
}

pub fn is_bool_true(body: &[u8]) -> bool {
    body.len() == 4 && u32::from_le_bytes(body[0..4].try_into().unwrap_or([0u8; 4])) == 0x997275b5
}

pub fn is_bool_false(body: &[u8]) -> bool {
    body.len() == 4 && u32::from_le_bytes(body[0..4].try_into().unwrap_or([0u8; 4])) == 0xbc799737
}

/// How framing bytes are sent/received on a connection.
///
/// `Obfuscated` carries an `Arc<Mutex<ObfuscatedCipher>>` so the same cipher
/// state is shared (safely) between the writer task (TX / `encrypt`) and the
/// reader task (RX / `decrypt`).  The two directions are separate AES-CTR
/// instances inside `ObfuscatedCipher`, so locking is only needed to prevent
/// concurrent mutation of the struct, not to serialise TX vs RX.
#[derive(Clone)]
enum FrameKind {
    Abridged,
    Intermediate,
    #[allow(dead_code)]
    Full {
        send_seqno: Arc<std::sync::atomic::AtomicU32>,
        recv_seqno: Arc<std::sync::atomic::AtomicU32>,
    },
    /// Obfuscated2 over Abridged framing.
    Obfuscated {
        cipher: std::sync::Arc<tokio::sync::Mutex<ferogram_crypto::ObfuscatedCipher>>,
    },
    /// Obfuscated2 over Intermediate+padding framing (`0xDD` MTProxy).
    PaddedIntermediate {
        cipher: std::sync::Arc<tokio::sync::Mutex<ferogram_crypto::ObfuscatedCipher>>,
    },
    /// FakeTLS framing (`0xEE` MTProxy).
    FakeTls {
        cipher: std::sync::Arc<tokio::sync::Mutex<ferogram_crypto::ObfuscatedCipher>>,
    },
}

/// Write half of a split connection.  Held under `Mutex` in `ClientInner`.
/// A single server-provided salt with its validity window.
///
#[derive(Clone, Debug)]
struct FutureSalt {
    valid_since: i32,
    valid_until: i32,
    salt: i64,
}

/// Delay (seconds) before a salt is considered usable after its `valid_since`.
///
const SALT_USE_DELAY: i32 = 60;

/// Owns the EncryptedSession (for packing) and the pending-RPC map.
struct ConnectionWriter {
    enc: EncryptedSession,
    frame_kind: FrameKind,
    /// msg_ids of received content messages waiting to be acked.
    /// Drained into a MsgsAck on every outgoing frame (bundled into container
    /// when sending an RPC, or sent standalone after route_frame).
    pending_ack: Vec<i64>,
    /// raw TL body bytes of every sent request, keyed by msg_id.
    /// On bad_msg_notification the matching body is re-encrypted with a fresh
    /// msg_id and re-sent transparently.
    sent_bodies: std::collections::HashMap<i64, Vec<u8>>,
    /// maps container_msg_id -> inner request msg_id.
    /// When bad_msg_notification / bad_server_salt arrives for a container
    /// rather than the individual inner message, we look here to find the
    /// inner request to retry.
    ///
    container_map: std::collections::HashMap<i64, i64>,
    /// -style future salt pool.
    /// Sorted by valid_since ascending so the newest salt is LAST
    /// (.valid_since), which puts
    /// the highest valid_since at the end in ascending-key order).
    salts: Vec<FutureSalt>,
    /// Server-time anchor received with the last GetFutureSalts response.
    /// (server_now, local_instant) lets us approximate server time at any
    /// moment so we can check whether a salt's valid_since window has opened.
    ///
    start_salt_time: Option<(i32, std::time::Instant)>,
}

impl ConnectionWriter {
    fn auth_key_bytes(&self) -> [u8; 256] {
        self.enc.auth_key_bytes()
    }
    fn first_salt(&self) -> i64 {
        self.enc.salt
    }
    fn time_offset(&self) -> i32 {
        self.enc.time_offset
    }

    /// Proactively advance the active salt and prune expired ones.
    ///
    /// Called at the top of every RPC send.
    /// Salts are sorted ascending by `valid_since` (oldest=index 0, newest=last).
    ///
    /// Prunes expired salts, then advances `enc.salt` to the freshest usable one.
    ///
    /// Returns `true` when the pool has shrunk to a single entry: caller should
    /// fire a proactive `GetFutureSalts`.
    ///
    ///                  `try_request_salts()`.
    fn advance_salt_if_needed(&mut self) -> bool {
        let Some((server_now, start_instant)) = self.start_salt_time else {
            return self.salts.len() <= 1;
        };

        // Approximate current server time.
        let now = server_now + start_instant.elapsed().as_secs() as i32;

        // Prune expired salts.
        while self.salts.len() > 1 && now > self.salts[0].valid_until {
            let expired = self.salts.remove(0);
            tracing::debug!(
                "[ferogram] salt {:#x} expired (valid_until={}), pruned",
                expired.salt,
                expired.valid_until,
            );
        }

        // Advance to the freshest salt whose use-delay has opened.
        if self.salts.len() > 1 {
            let best = self
                .salts
                .iter()
                .rev()
                .find(|s| s.valid_since + SALT_USE_DELAY <= now)
                .map(|s| s.salt);
            if let Some(salt) = best
                && salt != self.enc.salt
            {
                tracing::debug!(
                    "[ferogram] proactive salt cycle: {:#x} -> {:#x}",
                    self.enc.salt,
                    salt
                );
                self.enc.salt = salt;
                // Drop all entries older than the newly active salt.
                self.salts.retain(|s| s.valid_since >= now - SALT_USE_DELAY);
                if self.salts.is_empty() {
                    // Safety net: keep a sentinel so we never go saltless.
                    self.salts.push(FutureSalt {
                        valid_since: 0,
                        valid_until: i32::MAX,
                        salt,
                    });
                }
            }
        }

        self.salts.len() <= 1
    }
}

struct Connection {
    stream: TcpStream,
    enc: EncryptedSession,
    frame_kind: FrameKind,
}

impl Connection {
    /// Open a TCP stream, optionally via SOCKS5, and apply transport init bytes.
    async fn open_stream(
        addr: &str,
        socks5: Option<&crate::socks5::Socks5Config>,
        transport: &TransportKind,
        dc_id: i16,
    ) -> Result<(TcpStream, FrameKind), InvocationError> {
        let stream = match socks5 {
            Some(proxy) => proxy.connect(addr).await?,
            None => {
                let stream = TcpStream::connect(addr)
                    .await
                    .map_err(InvocationError::Io)?;
                stream.set_nodelay(true).ok();
                {
                    let sock = socket2::SockRef::from(&stream);
                    let keepalive = TcpKeepalive::new()
                        .with_time(Duration::from_secs(TCP_KEEPALIVE_IDLE_SECS))
                        .with_interval(Duration::from_secs(TCP_KEEPALIVE_INTERVAL_SECS));
                    #[cfg(not(target_os = "windows"))]
                    let keepalive = keepalive.with_retries(TCP_KEEPALIVE_PROBES);
                    sock.set_tcp_keepalive(&keepalive).ok();
                }
                stream
            }
        };
        Self::apply_transport_init(stream, transport, dc_id).await
    }

    /// Open a stream routed through an MTProxy (connects to proxy host:port,
    /// not to the Telegram DC address).
    async fn open_stream_mtproxy(
        mtproxy: &crate::proxy::MtProxyConfig,
        dc_id: i16,
    ) -> Result<(TcpStream, FrameKind), InvocationError> {
        let stream = mtproxy.connect().await?;
        stream.set_nodelay(true).ok();
        Self::apply_transport_init(stream, &mtproxy.transport, dc_id).await
    }

    async fn apply_transport_init(
        mut stream: TcpStream,
        transport: &TransportKind,
        dc_id: i16,
    ) -> Result<(TcpStream, FrameKind), InvocationError> {
        match transport {
            TransportKind::Abridged => {
                stream.write_all(&[0xef]).await?;
                Ok((stream, FrameKind::Abridged))
            }
            TransportKind::Intermediate => {
                stream.write_all(&[0xee, 0xee, 0xee, 0xee]).await?;
                Ok((stream, FrameKind::Intermediate))
            }
            TransportKind::Full => {
                // Full transport has no init byte.
                Ok((
                    stream,
                    FrameKind::Full {
                        send_seqno: Arc::new(std::sync::atomic::AtomicU32::new(0)),
                        recv_seqno: Arc::new(std::sync::atomic::AtomicU32::new(0)),
                    },
                ))
            }
            TransportKind::Obfuscated { secret } => {
                use sha2::Digest;

                // Random 64-byte nonce: retry until it passes the reserved-pattern
                // check (mirrors tDesktop isGoodStartNonce). Without this, the nonce
                // could look like a plain HTTP request or another MTProto framing
                // tag to a proxy or DPI filter, causing the connection to be reset.
                let mut nonce = [0u8; 64];
                loop {
                    getrandom::getrandom(&mut nonce)
                        .map_err(|_| InvocationError::Deserialize("getrandom".into()))?;
                    let first = u32::from_le_bytes(nonce[0..4].try_into().unwrap());
                    let second = u32::from_le_bytes(nonce[4..8].try_into().unwrap());
                    let bad = nonce[0] == 0xEF
                        || first == 0x44414548 // HEAD
                        || first == 0x54534F50 // POST
                        || first == 0x20544547 // GET
                        || first == 0xEEEEEEEE
                        || first == 0xDDDDDDDD
                        || first == 0x02010316
                        || second == 0x00000000;
                    if !bad {
                        break;
                    }
                }

                // Key derivation from nonce[8..56]:
                //   TX: key=nonce[8..40]  iv=nonce[40..56]
                //   RX: key=rev[0..32]    iv=rev[32..48]   (rev = nonce[8..56] reversed)
                // When an MTProxy secret is present, each 32-byte key becomes
                // SHA-256(raw_key_slice || secret) tDesktop Version1::prepareKey.
                let tx_raw: [u8; 32] = nonce[8..40].try_into().unwrap();
                let tx_iv: [u8; 16] = nonce[40..56].try_into().unwrap();
                let mut rev48 = nonce[8..56].to_vec();
                rev48.reverse();
                let rx_raw: [u8; 32] = rev48[0..32].try_into().unwrap();
                let rx_iv: [u8; 16] = rev48[32..48].try_into().unwrap();

                let (tx_key, rx_key): ([u8; 32], [u8; 32]) = if let Some(s) = secret {
                    let mut h = sha2::Sha256::new();
                    h.update(tx_raw);
                    h.update(s.as_ref());
                    let tx: [u8; 32] = h.finalize().into();

                    let mut h = sha2::Sha256::new();
                    h.update(rx_raw);
                    h.update(s.as_ref());
                    let rx: [u8; 32] = h.finalize().into();
                    (tx, rx)
                } else {
                    (tx_raw, rx_raw)
                };

                // Stamp protocol id (Abridged = 0xEFEFEFEF) at nonce[56..60]
                // and DC id as little-endian i16 at nonce[60..62].
                nonce[56] = 0xef;
                nonce[57] = 0xef;
                nonce[58] = 0xef;
                nonce[59] = 0xef;
                let dc_bytes = dc_id.to_le_bytes();
                nonce[60] = dc_bytes[0];
                nonce[61] = dc_bytes[1];

                // Encrypt nonce[56..64] in-place using the TX cipher advanced
                // past the first 56 bytes (which are sent as plaintext).
                //
                // IMPORTANT: we must use the *same* cipher instance for both the
                // nonce tail encryption AND all subsequent TX data.  tDesktop uses a
                // single continuous AES-CTR stream after encrypting the full 64-byte
                // nonce the TX position is at byte 64.  Using a separate instance here
                // and storing a fresh-at-0 cipher was the dual-instance bug: the stored
                // cipher would re-encrypt from byte 0 instead of continuing from 64.
                let mut cipher =
                    ferogram_crypto::ObfuscatedCipher::from_keys(&tx_key, &tx_iv, &rx_key, &rx_iv);
                // Advance TX past nonce[0..56] (sent as plaintext, not encrypted).
                let mut skip = [0u8; 56];
                cipher.encrypt(&mut skip);
                // Encrypt nonce[56..64] in-place; cipher TX is now at position 64.
                cipher.encrypt(&mut nonce[56..64]);

                stream.write_all(&nonce).await?;

                let cipher_arc = std::sync::Arc::new(tokio::sync::Mutex::new(cipher));
                Ok((stream, FrameKind::Obfuscated { cipher: cipher_arc }))
            }
            TransportKind::PaddedIntermediate { secret } => {
                use sha2::Digest;
                let mut nonce = [0u8; 64];
                loop {
                    getrandom::getrandom(&mut nonce)
                        .map_err(|_| InvocationError::Deserialize("getrandom".into()))?;
                    let first = u32::from_le_bytes(nonce[0..4].try_into().unwrap());
                    let second = u32::from_le_bytes(nonce[4..8].try_into().unwrap());
                    let bad = nonce[0] == 0xEF
                        || first == 0x44414548
                        || first == 0x54534F50
                        || first == 0x20544547
                        || first == 0xEEEEEEEE
                        || first == 0xDDDDDDDD
                        || first == 0x02010316
                        || second == 0x00000000;
                    if !bad {
                        break;
                    }
                }
                let tx_raw: [u8; 32] = nonce[8..40].try_into().unwrap();
                let tx_iv: [u8; 16] = nonce[40..56].try_into().unwrap();
                let mut rev48 = nonce[8..56].to_vec();
                rev48.reverse();
                let rx_raw: [u8; 32] = rev48[0..32].try_into().unwrap();
                let rx_iv: [u8; 16] = rev48[32..48].try_into().unwrap();
                let (tx_key, rx_key): ([u8; 32], [u8; 32]) = if let Some(s) = secret {
                    let mut h = sha2::Sha256::new();
                    h.update(tx_raw);
                    h.update(s.as_ref());
                    let tx: [u8; 32] = h.finalize().into();
                    let mut h = sha2::Sha256::new();
                    h.update(rx_raw);
                    h.update(s.as_ref());
                    let rx: [u8; 32] = h.finalize().into();
                    (tx, rx)
                } else {
                    (tx_raw, rx_raw)
                };
                // PaddedIntermediate tag = 0xDDDDDDDD
                nonce[56] = 0xdd;
                nonce[57] = 0xdd;
                nonce[58] = 0xdd;
                nonce[59] = 0xdd;
                let dc_bytes = dc_id.to_le_bytes();
                nonce[60] = dc_bytes[0];
                nonce[61] = dc_bytes[1];
                let mut cipher =
                    ferogram_crypto::ObfuscatedCipher::from_keys(&tx_key, &tx_iv, &rx_key, &rx_iv);
                let mut skip = [0u8; 56];
                cipher.encrypt(&mut skip);
                cipher.encrypt(&mut nonce[56..64]);
                stream.write_all(&nonce).await?;
                let cipher_arc = std::sync::Arc::new(tokio::sync::Mutex::new(cipher));
                Ok((stream, FrameKind::PaddedIntermediate { cipher: cipher_arc }))
            }
            TransportKind::FakeTls { secret, domain } => {
                // Fake TLS 1.3 ClientHello with HMAC-SHA256 random field.
                // After the handshake, data flows as TLS Application Data records
                // over a shared Obfuscated2 cipher seeded from the secret+HMAC.
                let domain_bytes = domain.as_bytes();
                let mut session_id = [0u8; 32];
                getrandom::getrandom(&mut session_id)
                    .map_err(|_| InvocationError::Deserialize("getrandom".into()))?;

                // Build ClientHello body (random placeholder = zeros)
                let cipher_suites: &[u8] = &[0x00, 0x04, 0x13, 0x01, 0x13, 0x02];
                let compression: &[u8] = &[0x01, 0x00];
                let sni_name_len = domain_bytes.len() as u16;
                let sni_list_len = sni_name_len + 3;
                let sni_ext_len = sni_list_len + 2;
                let mut sni_ext = Vec::new();
                sni_ext.extend_from_slice(&[0x00, 0x00]);
                sni_ext.extend_from_slice(&sni_ext_len.to_be_bytes());
                sni_ext.extend_from_slice(&sni_list_len.to_be_bytes());
                sni_ext.push(0x00);
                sni_ext.extend_from_slice(&sni_name_len.to_be_bytes());
                sni_ext.extend_from_slice(domain_bytes);
                let sup_ver: &[u8] = &[0x00, 0x2b, 0x00, 0x03, 0x02, 0x03, 0x04];
                let sup_grp: &[u8] = &[0x00, 0x0a, 0x00, 0x04, 0x00, 0x02, 0x00, 0x1d];
                let sess_tick: &[u8] = &[0x00, 0x23, 0x00, 0x00];
                let ext_body_len = sni_ext.len() + sup_ver.len() + sup_grp.len() + sess_tick.len();
                let mut extensions = Vec::new();
                extensions.extend_from_slice(&(ext_body_len as u16).to_be_bytes());
                extensions.extend_from_slice(&sni_ext);
                extensions.extend_from_slice(sup_ver);
                extensions.extend_from_slice(sup_grp);
                extensions.extend_from_slice(sess_tick);

                let mut hello_body = Vec::new();
                hello_body.extend_from_slice(&[0x03, 0x03]);
                hello_body.extend_from_slice(&[0u8; 32]); // random placeholder
                hello_body.push(session_id.len() as u8);
                hello_body.extend_from_slice(&session_id);
                hello_body.extend_from_slice(cipher_suites);
                hello_body.extend_from_slice(compression);
                hello_body.extend_from_slice(&extensions);

                let hs_len = hello_body.len() as u32;
                let mut handshake = vec![
                    0x01,
                    ((hs_len >> 16) & 0xff) as u8,
                    ((hs_len >> 8) & 0xff) as u8,
                    (hs_len & 0xff) as u8,
                ];
                handshake.extend_from_slice(&hello_body);

                let rec_len = handshake.len() as u16;
                let mut record = Vec::new();
                record.push(0x16);
                record.extend_from_slice(&[0x03, 0x01]);
                record.extend_from_slice(&rec_len.to_be_bytes());
                record.extend_from_slice(&handshake);

                // HMAC-SHA256(secret, record) -> fill random field at offset 11
                use sha2::Digest;
                let random_offset = 5 + 4 + 2; // TLS-rec(5) + HS-hdr(4) + version(2)
                let hmac_result: [u8; 32] = {
                    use hmac::{Hmac, Mac};
                    type HmacSha256 = Hmac<sha2::Sha256>;
                    let mut mac = HmacSha256::new_from_slice(secret)
                        .map_err(|_| InvocationError::Deserialize("HMAC key error".into()))?;
                    mac.update(&record);
                    mac.finalize().into_bytes().into()
                };
                record[random_offset..random_offset + 32].copy_from_slice(&hmac_result);
                stream.write_all(&record).await?;

                // Derive Obfuscated2 key from secret + HMAC
                let mut h = sha2::Sha256::new();
                h.update(secret.as_ref());
                h.update(hmac_result);
                let derived: [u8; 32] = h.finalize().into();
                let iv = [0u8; 16];
                let cipher =
                    ferogram_crypto::ObfuscatedCipher::from_keys(&derived, &iv, &derived, &iv);
                let cipher_arc = std::sync::Arc::new(tokio::sync::Mutex::new(cipher));
                Ok((stream, FrameKind::FakeTls { cipher: cipher_arc }))
            }
        }
    }

    async fn connect_raw(
        addr: &str,
        socks5: Option<&crate::socks5::Socks5Config>,
        mtproxy: Option<&crate::proxy::MtProxyConfig>,
        transport: &TransportKind,
        dc_id: i16,
    ) -> Result<Self, InvocationError> {
        tracing::debug!("[ferogram] Connecting to {addr} (DH) …");

        let addr2 = addr.to_string();
        let socks5_c = socks5.cloned();
        let mtproxy_c = mtproxy.cloned();
        let transport_c = transport.clone();

        let fut = async move {
            let (mut stream, frame_kind) = if let Some(ref mp) = mtproxy_c {
                Self::open_stream_mtproxy(mp, dc_id).await?
            } else {
                Self::open_stream(&addr2, socks5_c.as_ref(), &transport_c, dc_id).await?
            };

            let mut plain = Session::new();

            let (req1, s1) =
                auth::step1().map_err(|e| InvocationError::Deserialize(e.to_string()))?;
            send_frame(
                &mut stream,
                &plain.pack(&req1).to_plaintext_bytes(),
                &frame_kind,
            )
            .await?;
            let res_pq: tl::enums::ResPq = recv_frame_plain(&mut stream, &frame_kind).await?;

            let (req2, s2) = auth::step2(s1, res_pq, dc_id as i32)
                .map_err(|e| InvocationError::Deserialize(e.to_string()))?;
            send_frame(
                &mut stream,
                &plain.pack(&req2).to_plaintext_bytes(),
                &frame_kind,
            )
            .await?;
            let dh: tl::enums::ServerDhParams = recv_frame_plain(&mut stream, &frame_kind).await?;

            let (req3, s3) =
                auth::step3(s2, dh).map_err(|e| InvocationError::Deserialize(e.to_string()))?;
            send_frame(
                &mut stream,
                &plain.pack(&req3).to_plaintext_bytes(),
                &frame_kind,
            )
            .await?;
            let ans: tl::enums::SetClientDhParamsAnswer =
                recv_frame_plain(&mut stream, &frame_kind).await?;

            // Retry loop for dh_gen_retry (up to 5 attempts, mirroring tDesktop).
            let done = {
                let mut result = auth::finish(s3, ans)
                    .map_err(|e| InvocationError::Deserialize(e.to_string()))?;
                let mut attempts = 0u8;
                loop {
                    match result {
                        auth::FinishResult::Done(d) => break d,
                        auth::FinishResult::Retry {
                            retry_id,
                            dh_params,
                            nonce,
                            server_nonce,
                            new_nonce,
                        } => {
                            attempts += 1;
                            if attempts >= 5 {
                                return Err(InvocationError::Deserialize(
                                    "dh_gen_retry exceeded 5 attempts".into(),
                                ));
                            }
                            let (req_retry, s3_retry) = auth::retry_step3(
                                &dh_params,
                                nonce,
                                server_nonce,
                                new_nonce,
                                retry_id,
                            )
                            .map_err(|e| InvocationError::Deserialize(e.to_string()))?;
                            send_frame(
                                &mut stream,
                                &plain.pack(&req_retry).to_plaintext_bytes(),
                                &frame_kind,
                            )
                            .await?;
                            let ans_retry: tl::enums::SetClientDhParamsAnswer =
                                recv_frame_plain(&mut stream, &frame_kind).await?;
                            result = auth::finish(s3_retry, ans_retry)
                                .map_err(|e| InvocationError::Deserialize(e.to_string()))?;
                        }
                    }
                }
            };
            tracing::debug!("[ferogram] DH complete ✓");

            Ok::<Self, InvocationError>(Self {
                stream,
                enc: EncryptedSession::new(done.auth_key, done.first_salt, done.time_offset),
                frame_kind,
            })
        };

        tokio::time::timeout(Duration::from_secs(15), fut)
            .await
            .map_err(|_| {
                InvocationError::Deserialize(format!(
                    "DH handshake with {addr} timed out after 15 s"
                ))
            })?
    }

    async fn connect_with_key(
        addr: &str,
        auth_key: [u8; 256],
        first_salt: i64,
        time_offset: i32,
        socks5: Option<&crate::socks5::Socks5Config>,
        transport: &TransportKind,
        dc_id: i16,
    ) -> Result<Self, InvocationError> {
        let addr2 = addr.to_string();
        let socks5_c = socks5.cloned();
        let transport_c = transport.clone();

        let fut = async move {
            let (stream, frame_kind) =
                Self::open_stream(&addr2, socks5_c.as_ref(), &transport_c, dc_id).await?;
            Ok::<Self, InvocationError>(Self {
                stream,
                enc: EncryptedSession::new(auth_key, first_salt, time_offset),
                frame_kind,
            })
        };

        tokio::time::timeout(Duration::from_secs(15), fut)
            .await
            .map_err(|_| {
                InvocationError::Deserialize(format!(
                    "connect_with_key to {addr} timed out after 15 s"
                ))
            })?
    }

    fn auth_key_bytes(&self) -> [u8; 256] {
        self.enc.auth_key_bytes()
    }

    /// Split into a write-only `ConnectionWriter` and the TCP read half.
    fn into_writer(self) -> (ConnectionWriter, OwnedWriteHalf, OwnedReadHalf, FrameKind) {
        let (read_half, write_half) = self.stream.into_split();
        let writer = ConnectionWriter {
            enc: self.enc,
            frame_kind: self.frame_kind.clone(),
            pending_ack: Vec::new(),
            sent_bodies: std::collections::HashMap::new(),
            container_map: std::collections::HashMap::new(),
            salts: Vec::new(),
            start_salt_time: None,
        };
        (writer, write_half, read_half, self.frame_kind)
    }
}

/// Send a framed message using the active transport kind.
async fn send_frame(
    stream: &mut TcpStream,
    data: &[u8],
    kind: &FrameKind,
) -> Result<(), InvocationError> {
    match kind {
        FrameKind::Abridged => send_abridged(stream, data).await,
        FrameKind::Intermediate => {
            let mut frame = Vec::with_capacity(4 + data.len());
            frame.extend_from_slice(&(data.len() as u32).to_le_bytes());
            frame.extend_from_slice(data);
            stream.write_all(&frame).await?;
            Ok(())
        }
        FrameKind::Full { send_seqno, .. } => {
            // Full: [total_len(4)][seq(4)][payload][crc32(4)]
            // total_len covers all 4 fields including itself.
            let seq = send_seqno.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let total_len = (data.len() as u32) + 12;
            let mut packet = Vec::with_capacity(total_len as usize);
            packet.extend_from_slice(&total_len.to_le_bytes());
            packet.extend_from_slice(&seq.to_le_bytes());
            packet.extend_from_slice(data);
            let crc = crate::transport_intermediate::crc32_ieee(&packet);
            packet.extend_from_slice(&crc.to_le_bytes());
            stream.write_all(&packet).await?;
            Ok(())
        }
        FrameKind::Obfuscated { cipher } => {
            // Abridged framing with AES-256-CTR encryption over the whole frame.
            let words = data.len() / 4;
            let mut frame = if words < 0x7f {
                let mut v = Vec::with_capacity(1 + data.len());
                v.push(words as u8);
                v
            } else {
                let mut v = Vec::with_capacity(4 + data.len());
                v.extend_from_slice(&[
                    0x7f,
                    (words & 0xff) as u8,
                    ((words >> 8) & 0xff) as u8,
                    ((words >> 16) & 0xff) as u8,
                ]);
                v
            };
            frame.extend_from_slice(data);
            cipher.lock().await.encrypt(&mut frame);
            stream.write_all(&frame).await?;
            Ok(())
        }
        FrameKind::PaddedIntermediate { cipher } => {
            // Intermediate framing + 0–15 random padding bytes, encrypted.
            let mut pad_len_buf = [0u8; 1];
            getrandom::getrandom(&mut pad_len_buf).ok();
            let pad_len = (pad_len_buf[0] & 0x0f) as usize;
            let total_payload = data.len() + pad_len;
            let mut frame = Vec::with_capacity(4 + total_payload);
            frame.extend_from_slice(&(total_payload as u32).to_le_bytes());
            frame.extend_from_slice(data);
            let mut pad = vec![0u8; pad_len];
            getrandom::getrandom(&mut pad).ok();
            frame.extend_from_slice(&pad);
            cipher.lock().await.encrypt(&mut frame);
            stream.write_all(&frame).await?;
            Ok(())
        }
        FrameKind::FakeTls { cipher } => {
            // Wrap each MTProto message as a TLS Application Data record (type 0x17).
            // Telegram's FakeTLS sends one MTProto frame per TLS record, encrypted
            // with the Obfuscated2 cipher (no real TLS encryption).
            const TLS_APP_DATA: u8 = 0x17;
            const TLS_VER: [u8; 2] = [0x03, 0x03];
            // Split into 2878-byte chunks if needed (tDesktop limit).
            const CHUNK: usize = 2878;
            let mut locked = cipher.lock().await;
            for chunk in data.chunks(CHUNK) {
                let chunk_len = chunk.len() as u16;
                let mut record = Vec::with_capacity(5 + chunk.len());
                record.push(TLS_APP_DATA);
                record.extend_from_slice(&TLS_VER);
                record.extend_from_slice(&chunk_len.to_be_bytes());
                record.extend_from_slice(chunk);
                // Encrypt only the payload portion (after the 5-byte header).
                locked.encrypt(&mut record[5..]);
                stream.write_all(&record).await?;
            }
            Ok(())
        }
    }
}

// Split-reader helpers

/// Outcome of a timed frame read attempt.
enum FrameOutcome {
    Frame(Vec<u8>),
    Error(InvocationError),
    Keepalive, // timeout elapsed but ping was sent; caller should loop
}

/// Read one frame with a 60-second keepalive timeout (PING_DELAY_SECS).
///
/// If the timeout fires we send a `PingDelayDisconnect`: this tells Telegram
/// to forcibly close the connection after `NO_PING_DISCONNECT` seconds of
/// silence, giving us a clean EOF to detect rather than a silently stale socket.
/// That mirrors what both  and the official Telegram clients do.
async fn recv_frame_with_keepalive(
    rh: &mut OwnedReadHalf,
    fk: &FrameKind,
    client: &Client,
    _ak: &[u8; 256],
) -> FrameOutcome {
    match tokio::time::timeout(
        Duration::from_secs(PING_DELAY_SECS),
        recv_frame_read(rh, fk),
    )
    .await
    {
        Ok(Ok(raw)) => FrameOutcome::Frame(raw),
        Ok(Err(e)) => FrameOutcome::Error(e),
        Err(_) => {
            // Keepalive timeout: send PingDelayDisconnect so Telegram closes the
            // connection cleanly (EOF) if it hears nothing for NO_PING_DISCONNECT
            // seconds, rather than leaving a silently stale socket.
            let ping_req = tl::functions::PingDelayDisconnect {
                ping_id: random_i64(),
                disconnect_delay: NO_PING_DISCONNECT,
            };
            let (wire, fk) = {
                let mut w = client.inner.writer.lock().await;
                let fk = w.frame_kind.clone();
                (w.enc.pack(&ping_req), fk)
            };
            match send_frame_write(&mut *client.inner.write_half.lock().await, &wire, &fk).await {
                Ok(()) => FrameOutcome::Keepalive,
                Err(e) => FrameOutcome::Error(e),
            }
        }
    }
}

/// Send a framed message via an OwnedWriteHalf (split connection).
///
/// Header and payload are combined into a single Vec before calling
/// write_all, reducing write syscalls from 2 -> 1 per frame.  With Abridged
/// framing this previously sent a 1-byte header then the payload in separate
/// syscalls (and two TCP segments even with TCP_NODELAY on fast paths).
async fn send_frame_write(
    stream: &mut OwnedWriteHalf,
    data: &[u8],
    kind: &FrameKind,
) -> Result<(), InvocationError> {
    match kind {
        FrameKind::Abridged => {
            let words = data.len() / 4;
            // Build header + payload in one allocation -> single syscall.
            let mut frame = if words < 0x7f {
                let mut v = Vec::with_capacity(1 + data.len());
                v.push(words as u8);
                v
            } else {
                let mut v = Vec::with_capacity(4 + data.len());
                v.extend_from_slice(&[
                    0x7f,
                    (words & 0xff) as u8,
                    ((words >> 8) & 0xff) as u8,
                    ((words >> 16) & 0xff) as u8,
                ]);
                v
            };
            frame.extend_from_slice(data);
            stream.write_all(&frame).await?;
            Ok(())
        }
        FrameKind::Intermediate => {
            let mut frame = Vec::with_capacity(4 + data.len());
            frame.extend_from_slice(&(data.len() as u32).to_le_bytes());
            frame.extend_from_slice(data);
            stream.write_all(&frame).await?;
            Ok(())
        }
        FrameKind::Full { send_seqno, .. } => {
            // Full: [total_len(4)][seq(4)][payload][crc32(4)]
            let seq = send_seqno.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let total_len = (data.len() as u32) + 12;
            let mut packet = Vec::with_capacity(total_len as usize);
            packet.extend_from_slice(&total_len.to_le_bytes());
            packet.extend_from_slice(&seq.to_le_bytes());
            packet.extend_from_slice(data);
            let crc = crate::transport_intermediate::crc32_ieee(&packet);
            packet.extend_from_slice(&crc.to_le_bytes());
            stream.write_all(&packet).await?;
            Ok(())
        }
        FrameKind::Obfuscated { cipher } => {
            // Abridged framing + AES-256-CTR encryption (cipher stored).
            let words = data.len() / 4;
            let mut frame = if words < 0x7f {
                let mut v = Vec::with_capacity(1 + data.len());
                v.push(words as u8);
                v
            } else {
                let mut v = Vec::with_capacity(4 + data.len());
                v.extend_from_slice(&[
                    0x7f,
                    (words & 0xff) as u8,
                    ((words >> 8) & 0xff) as u8,
                    ((words >> 16) & 0xff) as u8,
                ]);
                v
            };
            frame.extend_from_slice(data);
            cipher.lock().await.encrypt(&mut frame);
            stream.write_all(&frame).await?;
            Ok(())
        }
        FrameKind::PaddedIntermediate { cipher } => {
            let mut pad_len_buf = [0u8; 1];
            getrandom::getrandom(&mut pad_len_buf).ok();
            let pad_len = (pad_len_buf[0] & 0x0f) as usize;
            let total_payload = data.len() + pad_len;
            let mut frame = Vec::with_capacity(4 + total_payload);
            frame.extend_from_slice(&(total_payload as u32).to_le_bytes());
            frame.extend_from_slice(data);
            let mut pad = vec![0u8; pad_len];
            getrandom::getrandom(&mut pad).ok();
            frame.extend_from_slice(&pad);
            cipher.lock().await.encrypt(&mut frame);
            stream.write_all(&frame).await?;
            Ok(())
        }
        FrameKind::FakeTls { cipher } => {
            const TLS_APP_DATA: u8 = 0x17;
            const TLS_VER: [u8; 2] = [0x03, 0x03];
            const CHUNK: usize = 2878;
            let mut locked = cipher.lock().await;
            for chunk in data.chunks(CHUNK) {
                let chunk_len = chunk.len() as u16;
                let mut record = Vec::with_capacity(5 + chunk.len());
                record.push(TLS_APP_DATA);
                record.extend_from_slice(&TLS_VER);
                record.extend_from_slice(&chunk_len.to_be_bytes());
                record.extend_from_slice(chunk);
                locked.encrypt(&mut record[5..]);
                stream.write_all(&record).await?;
            }
            Ok(())
        }
    }
}

/// Receive a framed message via an OwnedReadHalf (split connection).
async fn recv_frame_read(
    stream: &mut OwnedReadHalf,
    kind: &FrameKind,
) -> Result<Vec<u8>, InvocationError> {
    match kind {
        FrameKind::Abridged => {
            let mut h = [0u8; 1];
            stream.read_exact(&mut h).await?;
            let words = if h[0] < 0x7f {
                h[0] as usize
            } else {
                let mut b = [0u8; 3];
                stream.read_exact(&mut b).await?;
                b[0] as usize | (b[1] as usize) << 8 | (b[2] as usize) << 16
            };
            let len = words * 4;
            let mut buf = vec![0u8; len];
            stream.read_exact(&mut buf).await?;
            if buf.len() == 4 {
                let code = i32::from_le_bytes(buf[..4].try_into().unwrap());
                if code < 0 {
                    return Err(InvocationError::Rpc(RpcError::from_telegram(
                        code,
                        "transport error",
                    )));
                }
            }
            Ok(buf)
        }
        FrameKind::Intermediate => {
            let mut len_buf = [0u8; 4];
            stream.read_exact(&mut len_buf).await?;
            let len_i32 = i32::from_le_bytes(len_buf);
            if len_i32 < 0 {
                return Err(InvocationError::Rpc(RpcError::from_telegram(
                    len_i32,
                    "transport error",
                )));
            }
            if len_i32 <= 4 {
                let mut code_buf = [0u8; 4];
                stream.read_exact(&mut code_buf).await?;
                let code = i32::from_le_bytes(code_buf);
                return Err(InvocationError::Rpc(RpcError::from_telegram(
                    code,
                    "transport error",
                )));
            }
            let len = len_i32 as usize;
            let mut buf = vec![0u8; len];
            stream.read_exact(&mut buf).await?;
            Ok(buf)
        }
        FrameKind::Full { recv_seqno, .. } => {
            let mut len_buf = [0u8; 4];
            stream.read_exact(&mut len_buf).await?;
            let total_len_i32 = i32::from_le_bytes(len_buf);
            if total_len_i32 < 0 {
                return Err(InvocationError::Rpc(RpcError::from_telegram(
                    total_len_i32,
                    "transport error",
                )));
            }
            let total_len = total_len_i32 as usize;
            if total_len < 12 {
                return Err(InvocationError::Deserialize(
                    "Full transport: packet too short".into(),
                ));
            }
            let mut rest = vec![0u8; total_len - 4];
            stream.read_exact(&mut rest).await?;
            let (body, crc_bytes) = rest.split_at(rest.len() - 4);
            let expected_crc = u32::from_le_bytes(crc_bytes.try_into().unwrap());
            let mut check_input = Vec::with_capacity(4 + body.len());
            check_input.extend_from_slice(&len_buf);
            check_input.extend_from_slice(body);
            let actual_crc = crate::transport_intermediate::crc32_ieee(&check_input);
            if actual_crc != expected_crc {
                return Err(InvocationError::Deserialize(format!(
                    "Full transport: CRC mismatch (got {actual_crc:#010x}, expected {expected_crc:#010x})"
                )));
            }
            let recv_seq = u32::from_le_bytes(body[..4].try_into().unwrap());
            let expected_seq = recv_seqno.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            if recv_seq != expected_seq {
                return Err(InvocationError::Deserialize(format!(
                    "Full transport: seqno mismatch (got {recv_seq}, expected {expected_seq})"
                )));
            }
            Ok(body[4..].to_vec())
        }
        FrameKind::Obfuscated { cipher } => {
            let mut h = [0u8; 1];
            stream.read_exact(&mut h).await?;
            cipher.lock().await.decrypt(&mut h);
            let words = if h[0] < 0x7f {
                h[0] as usize
            } else {
                let mut b = [0u8; 3];
                stream.read_exact(&mut b).await?;
                cipher.lock().await.decrypt(&mut b);
                b[0] as usize | (b[1] as usize) << 8 | (b[2] as usize) << 16
            };
            let mut buf = vec![0u8; words * 4];
            stream.read_exact(&mut buf).await?;
            cipher.lock().await.decrypt(&mut buf);
            if buf.len() == 4 {
                let code = i32::from_le_bytes(buf[..4].try_into().unwrap());
                if code < 0 {
                    return Err(InvocationError::Rpc(RpcError::from_telegram(
                        code,
                        "transport error",
                    )));
                }
            }
            Ok(buf)
        }
        FrameKind::PaddedIntermediate { cipher } => {
            // Read 4-byte encrypted length prefix, then payload+padding.
            let mut len_buf = [0u8; 4];
            stream.read_exact(&mut len_buf).await?;
            cipher.lock().await.decrypt(&mut len_buf);
            let total_len = i32::from_le_bytes(len_buf);
            if total_len < 0 {
                return Err(InvocationError::Rpc(RpcError::from_telegram(
                    total_len,
                    "transport error",
                )));
            }
            let mut buf = vec![0u8; total_len as usize];
            stream.read_exact(&mut buf).await?;
            cipher.lock().await.decrypt(&mut buf);
            // The actual MTProto payload starts at byte 0; padding is at the tail.
            // We don't know the inner length here the caller (EncryptedSession::unpack)
            // uses body_len from the plaintext header, so padding is harmlessly ignored.
            Ok(buf)
        }
        FrameKind::FakeTls { cipher } => {
            // Read TLS Application Data record: 5-byte header + payload.
            let mut hdr = [0u8; 5];
            stream.read_exact(&mut hdr).await?;
            if hdr[0] != 0x17 {
                return Err(InvocationError::Deserialize(format!(
                    "FakeTLS: unexpected record type 0x{:02x}",
                    hdr[0]
                )));
            }
            let payload_len = u16::from_be_bytes([hdr[3], hdr[4]]) as usize;
            let mut buf = vec![0u8; payload_len];
            stream.read_exact(&mut buf).await?;
            cipher.lock().await.decrypt(&mut buf);
            Ok(buf)
        }
    }
}

/// Send using Abridged framing (used for DH plaintext during connect).
async fn send_abridged(stream: &mut TcpStream, data: &[u8]) -> Result<(), InvocationError> {
    let words = data.len() / 4;
    // Single combined write (header + payload): same fix #1 as send_frame_write.
    let mut frame = if words < 0x7f {
        let mut v = Vec::with_capacity(1 + data.len());
        v.push(words as u8);
        v
    } else {
        let mut v = Vec::with_capacity(4 + data.len());
        v.extend_from_slice(&[
            0x7f,
            (words & 0xff) as u8,
            ((words >> 8) & 0xff) as u8,
            ((words >> 16) & 0xff) as u8,
        ]);
        v
    };
    frame.extend_from_slice(data);
    stream.write_all(&frame).await?;
    Ok(())
}

async fn recv_abridged(stream: &mut TcpStream) -> Result<Vec<u8>, InvocationError> {
    let mut h = [0u8; 1];
    stream.read_exact(&mut h).await?;
    let words = if h[0] < 0x7f {
        h[0] as usize
    } else {
        let mut b = [0u8; 3];
        stream.read_exact(&mut b).await?;
        let w = b[0] as usize | (b[1] as usize) << 8 | (b[2] as usize) << 16;
        // word count of 1 after 0xFF = Telegram 4-byte transport error code
        if w == 1 {
            let mut code_buf = [0u8; 4];
            stream.read_exact(&mut code_buf).await?;
            let code = i32::from_le_bytes(code_buf);
            return Err(InvocationError::Rpc(RpcError::from_telegram(
                code,
                "transport error",
            )));
        }
        w
    };
    // Guard against implausibly large reads: a raw 4-byte transport error
    // whose first byte was mis-read as a word count causes a hang otherwise.
    if words == 0 || words > 0x8000 {
        return Err(InvocationError::Deserialize(format!(
            "abridged: implausible word count {words} (possible transport error or framing mismatch)"
        )));
    }
    let mut buf = vec![0u8; words * 4];
    stream.read_exact(&mut buf).await?;
    Ok(buf)
}

/// Receive a plaintext (pre-auth) frame and deserialize it.
async fn recv_frame_plain<T: Deserializable>(
    stream: &mut TcpStream,
    kind: &FrameKind,
) -> Result<T, InvocationError> {
    // DH handshake frames use the same transport framing as all other frames.
    // The old code hardcoded recv_abridged here, which worked only when transport
    // was Abridged.  With Intermediate or Full, Telegram responds with a 4-byte
    // LE length prefix but we tried to parse it as a 1-byte Abridged header
    // mangling the length, reading garbage bytes, and corrupting the auth key.
    // Every single subsequent decrypt then failed with AuthKeyMismatch.
    let raw = match kind {
        FrameKind::Abridged => recv_abridged(stream).await?,
        FrameKind::Intermediate => {
            let mut len_buf = [0u8; 4];
            stream.read_exact(&mut len_buf).await?;
            let len = u32::from_le_bytes(len_buf) as usize;
            if len == 0 || len > 1 << 24 {
                return Err(InvocationError::Deserialize(format!(
                    "plaintext frame: implausible length {len}"
                )));
            }
            let mut buf = vec![0u8; len];
            stream.read_exact(&mut buf).await?;
            buf
        }
        FrameKind::Full { recv_seqno, .. } => {
            // Full: [total_len(4)][seq(4)][payload][crc32(4)]
            let mut len_buf = [0u8; 4];
            stream.read_exact(&mut len_buf).await?;
            let total_len = u32::from_le_bytes(len_buf) as usize;
            if !(12..=(1 << 24) + 12).contains(&total_len) {
                return Err(InvocationError::Deserialize(format!(
                    "Full plaintext frame: implausible total_len {total_len}"
                )));
            }
            let mut rest = vec![0u8; total_len - 4];
            stream.read_exact(&mut rest).await?;

            // Verify CRC-32.
            let (body, crc_bytes) = rest.split_at(rest.len() - 4);
            let expected_crc = u32::from_le_bytes(crc_bytes.try_into().unwrap());
            let mut check_input = Vec::with_capacity(4 + body.len());
            check_input.extend_from_slice(&len_buf);
            check_input.extend_from_slice(body);
            let actual_crc = crate::transport_intermediate::crc32_ieee(&check_input);
            if actual_crc != expected_crc {
                return Err(InvocationError::Deserialize(format!(
                    "Full plaintext: CRC mismatch (got {actual_crc:#010x}, expected {expected_crc:#010x})"
                )));
            }

            // Validate and advance seqno.
            let recv_seq = u32::from_le_bytes(body[..4].try_into().unwrap());
            let expected_seq = recv_seqno.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            if recv_seq != expected_seq {
                return Err(InvocationError::Deserialize(format!(
                    "Full plaintext: seqno mismatch (got {recv_seq}, expected {expected_seq})"
                )));
            }

            body[4..].to_vec()
        }
        FrameKind::Obfuscated { cipher } => {
            // Obfuscated2: Abridged framing with AES-256-CTR decryption.
            let mut h = [0u8; 1];
            stream.read_exact(&mut h).await?;
            cipher.lock().await.decrypt(&mut h);
            let words = if h[0] < 0x7f {
                h[0] as usize
            } else {
                let mut b = [0u8; 3];
                stream.read_exact(&mut b).await?;
                cipher.lock().await.decrypt(&mut b);
                b[0] as usize | (b[1] as usize) << 8 | (b[2] as usize) << 16
            };
            let mut buf = vec![0u8; words * 4];
            stream.read_exact(&mut buf).await?;
            cipher.lock().await.decrypt(&mut buf);
            buf
        }
        FrameKind::PaddedIntermediate { cipher } => {
            let mut len_buf = [0u8; 4];
            stream.read_exact(&mut len_buf).await?;
            cipher.lock().await.decrypt(&mut len_buf);
            let len = u32::from_le_bytes(len_buf) as usize;
            if len == 0 || len > 1 << 24 {
                return Err(InvocationError::Deserialize(format!(
                    "PaddedIntermediate plaintext: implausible length {len}"
                )));
            }
            let mut buf = vec![0u8; len];
            stream.read_exact(&mut buf).await?;
            cipher.lock().await.decrypt(&mut buf);
            buf
        }
        FrameKind::FakeTls { cipher } => {
            let mut hdr = [0u8; 5];
            stream.read_exact(&mut hdr).await?;
            if hdr[0] != 0x17 {
                return Err(InvocationError::Deserialize(format!(
                    "FakeTLS plaintext: unexpected record type 0x{:02x}",
                    hdr[0]
                )));
            }
            let payload_len = u16::from_be_bytes([hdr[3], hdr[4]]) as usize;
            let mut buf = vec![0u8; payload_len];
            stream.read_exact(&mut buf).await?;
            cipher.lock().await.decrypt(&mut buf);
            buf
        }
    };
    if raw.len() < 20 {
        return Err(InvocationError::Deserialize(
            "plaintext frame too short".into(),
        ));
    }
    if u64::from_le_bytes(raw[..8].try_into().unwrap()) != 0 {
        return Err(InvocationError::Deserialize(
            "expected auth_key_id=0 in plaintext".into(),
        ));
    }
    let body_len = u32::from_le_bytes(raw[16..20].try_into().unwrap()) as usize;
    if 20 + body_len > raw.len() {
        return Err(InvocationError::Deserialize(
            "plaintext frame: body_len exceeds frame size".into(),
        ));
    }
    let mut cur = Cursor::from_slice(&raw[20..20 + body_len]);
    T::deserialize(&mut cur).map_err(Into::into)
}

// MTProto envelope

enum EnvelopeResult {
    Payload(Vec<u8>),
    /// Raw update bytes to be routed through dispatch_updates for proper pts tracking.
    RawUpdates(Vec<Vec<u8>>),
    /// pts/pts_count from updateShortSentMessage: advance counter, emit nothing.
    Pts(i32, i32),
    None,
}

fn unwrap_envelope(body: Vec<u8>) -> Result<EnvelopeResult, InvocationError> {
    if body.len() < 4 {
        return Err(InvocationError::Deserialize("body < 4 bytes".into()));
    }
    let cid = u32::from_le_bytes(body[..4].try_into().unwrap());

    match cid {
        ID_RPC_RESULT => {
            if body.len() < 12 {
                return Err(InvocationError::Deserialize("rpc_result too short".into()));
            }
            unwrap_envelope(body[12..].to_vec())
        }
        ID_RPC_ERROR => {
            if body.len() < 8 {
                return Err(InvocationError::Deserialize("rpc_error too short".into()));
            }
            let code    = i32::from_le_bytes(body[4..8].try_into().unwrap());
            let message = tl_read_string(&body[8..]).unwrap_or_default();
            Err(InvocationError::Rpc(RpcError::from_telegram(code, &message)))
        }
        ID_MSG_CONTAINER => {
            if body.len() < 8 {
                return Err(InvocationError::Deserialize("container too short".into()));
            }
            let count = u32::from_le_bytes(body[4..8].try_into().unwrap()) as usize;
            let mut pos = 8usize;
            let mut payload: Option<Vec<u8>> = None;
            let mut raw_updates: Vec<Vec<u8>> = Vec::new();

            for _ in 0..count {
                if pos + 16 > body.len() { break; }
                let inner_len = u32::from_le_bytes(body[pos + 12..pos + 16].try_into().unwrap()) as usize;
                pos += 16;
                if pos + inner_len > body.len() { break; }
                let inner = body[pos..pos + inner_len].to_vec();
                pos += inner_len;
                match unwrap_envelope(inner)? {
                    EnvelopeResult::Payload(p)          => { payload = Some(p); }
                    EnvelopeResult::RawUpdates(mut raws) => { raw_updates.append(&mut raws); }
                    EnvelopeResult::Pts(_, _)            => {} // handled via spawned task in route_frame
                    EnvelopeResult::None                 => {}
                }
            }
            if let Some(p) = payload {
                Ok(EnvelopeResult::Payload(p))
            } else if !raw_updates.is_empty() {
                Ok(EnvelopeResult::RawUpdates(raw_updates))
            } else {
                Ok(EnvelopeResult::None)
            }
        }
        ID_GZIP_PACKED => {
            let bytes = tl_read_bytes(&body[4..]).unwrap_or_default();
            unwrap_envelope(gz_inflate(&bytes)?)
        }
        // MTProto service messages: silently acknowledged, no payload extracted.
        // NOTE: ID_PONG is intentionally NOT listed here. Pong arrives as a bare
        // top-level frame (never inside rpc_result), so it is handled in route_frame
        // directly. Silencing it here would drop it before invoke() can resolve it.
        ID_MSGS_ACK | ID_NEW_SESSION | ID_BAD_SERVER_SALT | ID_BAD_MSG_NOTIFY
        // These are correctly silenced ( silences these too)
        | 0xd33b5459  // MsgsStateReq
        | 0x04deb57d  // MsgsStateInfo
        | 0x8cc0d131  // MsgsAllInfo
        | 0x276d3ec6  // MsgDetailedInfo
        | 0x809db6df  // MsgNewDetailedInfo
        | 0x7d861a08  // MsgResendReq / MsgResendAnsReq
        | 0x0949d9dc  // FutureSalt
        | 0xae500895  // FutureSalts
        | 0x9299359f  // HttpWait
        | 0xe22045fc  // DestroySessionOk
        | 0x62d350c9  // DestroySessionNone
        => {
            Ok(EnvelopeResult::None)
        }
        // Route all update containers via RawUpdates so route_frame can call
        // dispatch_updates, which handles pts/seq tracking. Without this, updates
        // from RPC responses (e.g. updateNewMessage + updateReadHistoryOutbox from
        // messages.sendMessage) bypass pts entirely -> false gaps -> getDifference
        // -> duplicate message delivery.
        ID_UPDATES | ID_UPDATE_SHORT | ID_UPDATES_COMBINED
        | ID_UPDATE_SHORT_MSG | ID_UPDATE_SHORT_CHAT_MSG
        | ID_UPDATES_TOO_LONG => {
            Ok(EnvelopeResult::RawUpdates(vec![body]))
        }
        // updateShortSentMessage is the RPC response to messages.sendMessage.
        // It carries the ONLY pts record for the bot's own sent message.
        // Bots do NOT receive a push updateNewMessage for their own messages,
        // so if we absorb this silently, pts stays stale -> false gap -> getDifference
        // -> re-delivery of already-processed messages -> duplicate replies.
        // Fix: extract pts/pts_count and return Pts variant so route_frame advances the counter.
        ID_UPDATE_SHORT_SENT_MSG => {
            let mut cur = Cursor::from_slice(&body[4..]);
            match tl::types::UpdateShortSentMessage::deserialize(&mut cur) {
                Ok(m) => {
                    tracing::debug!(
                        "[ferogram] updateShortSentMessage (RPC): pts={} pts_count={}: advancing pts",
                        m.pts, m.pts_count
                    );
                    Ok(EnvelopeResult::Pts(m.pts, m.pts_count))
                }
                Err(e) => {
                    tracing::debug!("[ferogram] updateShortSentMessage deserialize error: {e}");
                    Ok(EnvelopeResult::None)
                }
            }
        }
        _ => Ok(EnvelopeResult::Payload(body)),
    }
}

// Utilities

fn random_i64() -> i64 {
    let mut b = [0u8; 8];
    getrandom::getrandom(&mut b).expect("getrandom");
    i64::from_le_bytes(b)
}

/// Apply ±20 % random jitter to a backoff delay.
/// Prevents thundering-herd when many clients reconnect simultaneously
/// (e.g. after a server restart or a shared network outage).
fn jitter_delay(base_ms: u64) -> Duration {
    // Use two random bytes for the jitter factor (0..=65535 -> 0.80 … 1.20).
    let mut b = [0u8; 2];
    getrandom::getrandom(&mut b).unwrap_or(());
    let rand_frac = u16::from_le_bytes(b) as f64 / 65535.0; // 0.0 … 1.0
    let factor = 0.80 + rand_frac * 0.40; // 0.80 … 1.20
    Duration::from_millis((base_ms as f64 * factor) as u64)
}

pub(crate) fn tl_read_bytes(data: &[u8]) -> Option<Vec<u8>> {
    if data.is_empty() {
        return Some(vec![]);
    }
    let (len, start) = if data[0] < 254 {
        (data[0] as usize, 1)
    } else if data.len() >= 4 {
        (
            data[1] as usize | (data[2] as usize) << 8 | (data[3] as usize) << 16,
            4,
        )
    } else {
        return None;
    };
    if data.len() < start + len {
        return None;
    }
    Some(data[start..start + len].to_vec())
}

fn tl_read_string(data: &[u8]) -> Option<String> {
    tl_read_bytes(data).map(|b| String::from_utf8_lossy(&b).into_owned())
}

pub(crate) fn gz_inflate(data: &[u8]) -> Result<Vec<u8>, InvocationError> {
    use std::io::Read;
    let mut out = Vec::new();
    if flate2::read::GzDecoder::new(data)
        .read_to_end(&mut out)
        .is_ok()
        && !out.is_empty()
    {
        return Ok(out);
    }
    out.clear();
    flate2::read::ZlibDecoder::new(data)
        .read_to_end(&mut out)
        .map_err(|_| InvocationError::Deserialize("decompression failed".into()))?;
    Ok(out)
}

pub(crate) fn maybe_gz_decompress(body: Vec<u8>) -> Result<Vec<u8>, InvocationError> {
    const ID_GZIP_PACKED_LOCAL: u32 = 0x3072cfa1;
    if body.len() >= 4 && u32::from_le_bytes(body[0..4].try_into().unwrap()) == ID_GZIP_PACKED_LOCAL
    {
        let bytes = tl_read_bytes(&body[4..]).unwrap_or_default();
        gz_inflate(&bytes)
    } else {
        Ok(body)
    }
}

// outgoing gzip compression

/// Minimum body size above which we attempt zlib compression.
/// Below this threshold the gzip_packed wrapper overhead exceeds the gain.
const COMPRESSION_THRESHOLD: usize = 512;

/// TL `bytes` wire encoding (used inside gzip_packed).
fn tl_write_bytes(data: &[u8]) -> Vec<u8> {
    let len = data.len();
    let mut out = Vec::with_capacity(4 + len);
    if len < 254 {
        out.push(len as u8);
        out.extend_from_slice(data);
        let pad = (4 - (1 + len) % 4) % 4;
        out.extend(std::iter::repeat_n(0u8, pad));
    } else {
        out.push(0xfe);
        out.extend_from_slice(&(len as u32).to_le_bytes()[..3]);
        out.extend_from_slice(data);
        let pad = (4 - (4 + len) % 4) % 4;
        out.extend(std::iter::repeat_n(0u8, pad));
    }
    out
}

/// Wrap `data` in a `gzip_packed#3072cfa1 packed_data:bytes` TL frame.
fn gz_pack_body(data: &[u8]) -> Vec<u8> {
    use std::io::Write;
    let mut enc = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
    let _ = enc.write_all(data);
    let compressed = enc.finish().unwrap_or_default();
    let mut out = Vec::with_capacity(4 + 4 + compressed.len());
    out.extend_from_slice(&ID_GZIP_PACKED.to_le_bytes());
    out.extend(tl_write_bytes(&compressed));
    out
}

/// Optionally compress `data`.  Returns the compressed `gzip_packed` wrapper
/// if it is shorter than the original; otherwise returns `data` unchanged.
fn maybe_gz_pack(data: &[u8]) -> Vec<u8> {
    if data.len() <= COMPRESSION_THRESHOLD {
        return data.to_vec();
    }
    let packed = gz_pack_body(data);
    if packed.len() < data.len() {
        packed
    } else {
        data.to_vec()
    }
}

// +: MsgsAck body builder

/// Build the TL body for `msgs_ack#62d6b459 msg_ids:Vector<long>`.
fn build_msgs_ack_body(msg_ids: &[i64]) -> Vec<u8> {
    // msgs_ack#62d6b459 msg_ids:Vector<long>
    // Vector<long>: 0x1cb5c415 + count:int + [i64...]
    let mut out = Vec::with_capacity(4 + 4 + 4 + msg_ids.len() * 8);
    out.extend_from_slice(&ID_MSGS_ACK.to_le_bytes());
    out.extend_from_slice(&0x1cb5c415_u32.to_le_bytes()); // Vector constructor
    out.extend_from_slice(&(msg_ids.len() as u32).to_le_bytes());
    for &id in msg_ids {
        out.extend_from_slice(&id.to_le_bytes());
    }
    out
}

// MessageContainer body builder

/// Build the body of a `msg_container#73f1f8dc` from a list of
/// `(msg_id, seqno, body)` inner messages.
///
/// The caller is responsible for allocating msg_id and seqno for each entry
/// via `EncryptedSession::alloc_msg_seqno`.
fn build_container_body(messages: &[(i64, i32, &[u8])]) -> Vec<u8> {
    let total_body: usize = messages.iter().map(|(_, _, b)| 16 + b.len()).sum();
    let mut out = Vec::with_capacity(8 + total_body);
    out.extend_from_slice(&ID_MSG_CONTAINER.to_le_bytes());
    out.extend_from_slice(&(messages.len() as u32).to_le_bytes());
    for &(msg_id, seqno, body) in messages {
        out.extend_from_slice(&msg_id.to_le_bytes());
        out.extend_from_slice(&seqno.to_le_bytes());
        out.extend_from_slice(&(body.len() as u32).to_le_bytes());
        out.extend_from_slice(body);
    }
    out
}

// Low-level re-exports (merged from the former `layer` shim crate)

/// Re-export of [`ferogram_mtproto`]: session, encrypted session, transport, and authentication.
pub use ferogram_mtproto as mtproto;

/// Re-export of [`ferogram_crypto`]: AES-IGE, SHA, RSA, factorize, AuthKey.
pub use ferogram_crypto as crypto;

/// Re-export of [`ferogram_tl_parser`] (requires `feature = "parser"`).
#[cfg(feature = "parser")]
pub use ferogram_tl_parser as parser;

/// Re-export of [`ferogram_tl_gen`] (requires `feature = "codegen"`).
#[cfg(feature = "codegen")]
pub use ferogram_tl_gen as codegen;

// Convenience flat re-exports
pub use ferogram_crypto::AuthKey;
pub use ferogram_mtproto::authentication::{self, Finished, finish, step1, step2, step3};
pub use ferogram_tl_types::{Identifiable, LAYER, Serializable};
