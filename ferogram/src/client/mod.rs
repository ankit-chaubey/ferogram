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

use std::collections::HashMap;
use std::num::NonZeroU32;
use std::ops::ControlFlow;
use std::sync::Arc;
use std::time::Duration;

use crate::dc_migration::fallback_dc_addr;
use crate::retry::RetryLoop;
use crate::session::{self, PersistedSession};
use crate::update;
use crate::{
    AutoSleep, ConnectionRestartPolicy, DcEntry, DcFlags, ExperimentalFeatures, InvocationError,
    NeverRestart, PeerCache, RetryContext, RetryPolicy, TransferLimits, dc_pool, message_box,
    persist,
};
use ferogram_tl_types as tl;
use ferogram_tl_types::{Cursor, Deserializable, RemoteCall};

use tokio::sync::{Mutex, RwLock, mpsc, oneshot};
use tokio::time::sleep;
use tokio_util::sync::CancellationToken;

/// Keepalive ping interval.
const _PING_DELAY_SECS: u64 = 60;

/// Disconnect delay for PingDelayDisconnect: 75 s (interval + 15 s slack).
const _NO_PING_DISCONNECT: i32 = 75;

/// Initial backoff before the first reconnect attempt.
const RECONNECT_BASE_MS: u64 = 500;

/// Maximum backoff between reconnect attempts.
const RECONNECT_MAX_SECS: u64 = 5;

/// TCP socket-level keepalive: start probes after this many seconds of idle.
const _TCP_KEEPALIVE_IDLE_SECS: u64 = 10;
/// Interval between TCP keepalive probes.
const _TCP_KEEPALIVE_INTERVAL_SECS: u64 = 5;
/// Number of failed probes before the OS declares the connection dead.
const _TCP_KEEPALIVE_PROBES: u32 = 3;

///
/// | Variant | Init bytes | Notes |
/// |---------|-----------|-------|
/// | `Abridged` | `0xef` | Smallest overhead |
/// | `Intermediate` | `0xeeeeeeee` | Better proxy compat |
/// | `Full` | none | Adds seqno + CRC32: **default** |
/// | `Obfuscated` | random 64B | Bypasses DPI / MTProxy |
/// | `PaddedIntermediate` | random 64B (`0xDDDDDDDD` tag) | Obfuscated padded intermediate required for `0xDD` MTProxy secrets |
/// | `FakeTls` | TLS 1.3 ClientHello | Most DPI-resistant; required for `0xEE` MTProxy secrets |
// TransportKind moved to ferogram-connect; re-export for backward compatibility.
pub use ferogram_connect::TransportKind;

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
    /// Override which `dc_id` a fresh connection (no saved session yet) is
    /// registered under. Combined with `dc_addr`, lets a custom address be
    /// dialed and tracked as an arbitrary `dc_id` instead of the default DC2.
    /// Ignored once a saved session already exists.
    pub dc_id_override: Option<i32>,
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
    /// Race Obfuscated / Abridged / HTTP transports in parallel on fresh connect
    /// and pick the fastest.  Incompatible with MTProxy.  Default: `false`.
    pub probe_transport: bool,
    /// If direct TCP fails, retry via DNS-over-HTTPS (Mozilla + Google),
    /// then fall back to Firebase / Google special-config.  Default: `false`.
    pub resilient_connect: bool,
    /// Enable PFS via `auth.bindTempAuthKey`. Adds one DH round-trip per
    /// fresh connection. Default: `false`.
    pub use_pfs: bool,
    /// Opt-in experimental behaviours (all off by default).
    ///
    /// See [`ExperimentalFeatures`] for per-flag documentation.
    pub experimental_features: ExperimentalFeatures,
    /// Controls the user-facing update dispatch buffer.
    ///
    /// Internal MTProto state (pts, qts, getDifference) is unaffected: it
    /// always runs inside the reader task regardless of this setting.
    /// Only the high-level [`crate::Update`] queue your application reads via
    /// [`Client::stream_updates`] is governed here.
    ///
    /// Default: 2048-slot ring buffer with `DropOldest` overflow.
    pub update_config: crate::update_config::UpdateConfig,
    /// Seed a `future_auth_token` for fast re-login, bypassing code entry on
    /// the next `request_login_code` call if Telegram still recognizes it.
    ///
    /// Normally this is captured automatically by `sign_out()` and persisted
    /// in the session file; set this directly for stateless setups (e.g. a
    /// server that stores the token itself rather than a session file), or
    /// to import a token obtained elsewhere. Overrides any value already in
    /// the loaded session.
    pub future_auth_token: Option<Vec<u8>>,
    /// Concurrency ceilings for file transfers. See [`TransferLimits`] for
    /// the highway/trucks model this controls.
    ///
    /// Default: `download_tcp_connections: 4, upload_tcp_connections: 4,
    /// max_tcp_connections: 12, download_pipeline_depth: 4, upload_pipeline_depth: 4,
    /// bypass_tcp_allotments: false`.
    pub transfer_limits: TransferLimits,
}

impl Config {
    /// Convenience builder: use a portable base64 string session.
    ///
    /// Pass the string exported from a previous `client.export_session_string()` call,
    /// or an empty string to start fresh (the string session will be populated after auth).
    ///
    /// # Example
    /// ```rust,no_run
    /// # use ferogram::Config;
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
        self.transport = cfg.transport.clone();
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
            dc_id_override: None,
            retry_policy: Arc::new(AutoSleep::default()),
            socks5: None,
            mtproxy: None,
            allow_ipv6: false,
            transport: TransportKind::Full,
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
            probe_transport: false,
            resilient_connect: false,
            use_pfs: false,
            experimental_features: ExperimentalFeatures::default(),
            update_config: crate::update_config::UpdateConfig::default(),
            future_auth_token: None,
            transfer_limits: TransferLimits::default(),
        }
    }
}
/// Asynchronous stream of [`crate::Update`]s.
pub struct UpdateStream {
    rx: mpsc::Receiver<update::Update>,
}

impl UpdateStream {
    /// Wait for the next update. Returns `None` when the client has disconnected.
    pub async fn next(&mut self) -> Option<update::Update> {
        self.rx.recv().await
    }

    /// Wait for the next **raw** (unrecognised) update frame, skipping all
    /// typed high-level variants. Useful for handling update types that
    /// `ferogram` does not yet wrap: match on `.inner` directly to extract
    /// fields, or use `.constructor_id` for a quick type check without moving.
    ///
    /// Returns `None` when the client has disconnected.
    pub async fn next_raw(&mut self) -> Option<update::RawUpdate> {
        loop {
            match self.rx.recv().await? {
                update::Update::Raw(r) => return Some(*r),
                _ => continue,
            }
        }
    }
}

pub(crate) struct ClientInner {
    /// Crypto/state for the connection: EncryptedSession, salts, acks, etc.
    /// Enqueue RPC requests to the sender task.
    rpc_tx: mpsc::Sender<ferogram_mtsender::RpcEnqueue>,
    /// Send a new stream to the sender task after reconnect.
    reconnect_tx: mpsc::Sender<ferogram_mtsender::ReconnectRequest>,
    /// Notify all waiters (reconnect loop + frame dispatch) that the network is back.
    network_hint: std::sync::Arc<tokio::sync::Notify>,
    /// Cancelled to signal graceful shutdown to the reader task.
    #[allow(dead_code)]
    shutdown_token: CancellationToken,
    /// Whether to replay missed updates via getDifference on connect.
    #[allow(dead_code)]
    catch_up: bool,
    #[allow(dead_code)]
    restart_policy: Arc<dyn ConnectionRestartPolicy>,
    pub(crate) home_dc_id: Mutex<i32>,
    pub(crate) dc_options: Mutex<HashMap<i32, DcEntry>>,
    /// Media-only DC options (ipv6/media_only/cdn filtered separately from API DCs).
    pub(crate) media_dc_options: Mutex<HashMap<i32, DcEntry>>,
    pub peer_cache: RwLock<PeerCache>,
    /// Single-authority update state machine (gap detection, difference fetching).
    pub message_box: Mutex<message_box::MessageBoxes>,
    /// Bounded ring-buffer dedup cache  safety net beneath the pts machinery.
    /// Uses `parking_lot::Mutex` for minimal overhead; the critical section is
    /// tiny (a HashSet lookup + optional VecDeque eviction).
    #[allow(dead_code)]
    pub(crate) dedupe_cache: parking_lot::Mutex<persist::BoundedDedupeCache>,
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
    /// The transport actually negotiated on the current connection.
    /// Updated by do_reconnect after deriving from the active FrameKind.
    /// Used by the supervisor reconnect path which doesn't have access to fk.
    active_transport: std::sync::Mutex<TransportKind>,
    session_backend: Arc<dyn crate::session_backend::SessionBackend>,
    dc_pool: Mutex<dc_pool::DcPool>,
    /// Dedicated pool for file transfer connections (upload/download).\n    /// Isolated from the main session to prevent crypto state contamination.
    transfer_pool: Mutex<dc_pool::DcPool>,
    update_tx: mpsc::Sender<update::Update>,
    /// Whether this client is signed in as a bot (set in `bot_sign_in`).
    /// Used by `get_channel_difference` to pick the correct diff limit:
    /// bots get 100_000 (BOT_CHANNEL_DIFF_LIMIT), users get 100 (USER_CHANNEL_DIFF_LIMIT).
    pub is_bot: std::sync::atomic::AtomicBool,
    /// Global MTProto sender semaphore  - limits total concurrent transfer workers
    /// across all uploads and downloads to `transfer_limits.max_tcp_connections`.
    /// Each concurrent worker acquires one permit; it is released on drop.
    pub(crate) worker_semaphore: Arc<tokio::sync::Semaphore>,
    /// User-tunable transfer concurrency ceilings. See [`TransferLimits`]
    /// for the highway/trucks model. Normalized (clamped to the safe range)
    /// on construction regardless of how `Config` was built.
    pub(crate) transfer_limits: TransferLimits,
    /// Guards against calling `stream_updates()` more than once.
    stream_active: std::sync::atomic::AtomicBool,

    /// Monotonic connection epoch. Incremented inside do_reconnect() each time
    /// writer and write_half are replaced. Every send site snapshots this before
    /// building the wire and re-checks after acquiring write_half; if the value
    /// changed the wire belongs to the old session and must not be written.
    /// Prevents two concurrent fresh-DH handshakes racing each other.
    /// A double-DH results in one key being unregistered on Telegram's servers,
    /// causing AUTH_KEY_UNREGISTERED immediately after reconnect.
    dh_in_progress: std::sync::atomic::AtomicBool,

    /// Whether PFS (bindTempAuthKey) is enabled for new connections.
    pfs_enabled: bool,
    /// Update dispatch buffer configuration: capacity and overflow strategy.
    /// Stored here so stream_updates() can read it without borrowing Config.
    pub(crate) update_config: crate::update_config::UpdateConfig,
    /// Guards sync_state_after_dh: the function is a no-op while false so that
    /// reconnect-triggered DH completions don't fire GetState before the client
    /// is actually authorised.
    pub signed_in: std::sync::atomic::AtomicBool,

    /// Tracks which foreign DC IDs have had `auth.importAuthorization` called
    /// successfully in the current process session (in-memory only, not persisted).
    ///
    /// Tracks which foreign DCs have had `auth.importAuthorization` called
    /// successfully in this session.  The account authorization binding is
    /// session-scoped and must be re-established each process run.
    pub(crate) auth_imported: parking_lot::Mutex<std::collections::HashSet<i32>>,

    /// Rolling count of peer-cache misses in the current window.
    /// Reset when the window expires.
    peer_cache_miss_count: std::sync::atomic::AtomicU32,
    /// Start of the current miss-counting window.
    peer_cache_miss_window_start: parking_lot::Mutex<std::time::Instant>,
    /// Last time bulk dialog hydration ran.
    /// Enforces the cooldown between getDialogs calls.
    last_bulk_hydration: parking_lot::Mutex<Option<std::time::Instant>>,

    /// Per-DC connect gate for transfer pool initialisation.
    ///
    /// When multiple tasks race to open the first connection to the same foreign
    /// DC, each would independently do DH + export/import, creating redundant
    /// sockets and triggering AUTH_KEY_UNREGISTERED (only one key survives per
    /// DC slot).  This map stores one `Arc<Mutex<()>>` per DC ID; a task holds
    /// the mutex for the entire setup phase.  Subsequent tasks wait on the same
    /// mutex, then find the connection already present via the double-check
    /// inside `rpc_transfer_on_dc`.
    dc_connect_gates:
        parking_lot::Mutex<std::collections::HashMap<i32, std::sync::Arc<tokio::sync::Mutex<()>>>>,
    /// Per-DC gate that serialises auth.exportAuthorization / importAuthorization.
    ///
    /// Per-DC gate that serialises auth.exportAuthorization / importAuthorization.
    /// exportAuthorization tokens are single-use; this ensures only one caller
    /// does the export/import per DC per session.
    auth_import_gates:
        parking_lot::Mutex<std::collections::HashMap<i32, std::sync::Arc<tokio::sync::Mutex<()>>>>,

    /// Cached numeric ID of the logged-in account.
    ///
    /// Populated the moment any `tl::types::User` with `self_ == true` passes
    /// through `cache_user`/`cache_users_slice` (sign-in, `get_me()`, contact
    /// lists, etc.) - no dedicated network call needed in the common case.
    /// `0` means "not yet known". Used to resolve the sender of private-chat
    /// (DM) messages, since Telegram omits `from_id` there: with only two
    /// participants, the sender is derivable from `out` + `peer_id` alone.
    pub(crate) self_user_id: std::sync::atomic::AtomicI64,

    /// Set to true after any peer-cache mutation so the full-snapshot periodic
    /// saver knows a flush to the session backend is needed.
    session_snapshot_dirty: std::sync::atomic::AtomicBool,

    /// Experimental feature flags, stored for runtime checks in files.rs etc.
    #[allow(dead_code)]
    pub(crate) experimental: ExperimentalFeatures,

    /// Wakes the persistent diff task when a deadline fires or a reconnect completes.
    /// A single long-lived task owns the sequential diff loop, and callers
    /// just notify it instead of spawning new tasks.
    diff_notify: std::sync::Arc<tokio::sync::Notify>,

    /// `future_auth_token` captured from a previous `sign_out()` call (or
    /// restored from the loaded session). Replayed automatically by
    /// `request_login_code` so a fresh login after logout can skip code
    /// entry. `None` once consumed by a successful re-auth, or if the
    /// account has 2FA enabled (Telegram never issues one in that case).
    pub(crate) future_auth_token: parking_lot::Mutex<Option<Vec<u8>>>,
}

/// Pipelined transfer connection handle. The struct itself now lives in
/// `ferogram-mtsender` (it never depended on `Client` state, only on
/// `ferogram_mtsender` types), so `ferogram-py` can use it directly through
/// its existing dependency on that crate without going through `ferogram`.
pub(crate) use ferogram_mtsender::PipelinedSender;

#[inline]
fn mark_session_snapshot_dirty_impl(inner: &Arc<ClientInner>) {
    inner
        .session_snapshot_dirty
        .store(true, std::sync::atomic::Ordering::Release);
}

/// If `user` is a `User::User` with `is_self == true`, cache its numeric ID
/// as the logged-in account's own ID (see `Inner::self_user_id`).
///
/// Telegram sets this flag on the account's own `User` object wherever it
/// appears (sign-in response, `users.getUsers(InputUser::UserSelf)`,
/// contact lists, etc.), so this captures it for free the first time any
/// such object is cached - no dedicated network round-trip required.
fn remember_self_id_impl(inner: &Arc<ClientInner>, user: &tl::enums::User) {
    if let tl::enums::User::User(u) = user
        && u.is_self
    {
        inner
            .self_user_id
            .store(u.id, std::sync::atomic::Ordering::Relaxed);
    }
}

async fn cache_user_impl(inner: &Arc<ClientInner>, user: &tl::enums::User) {
    remember_self_id_impl(inner, user);
    inner.peer_cache.write().await.cache_user(user);
    mark_session_snapshot_dirty_impl(inner);
}

async fn cache_users_slice_impl(inner: &Arc<ClientInner>, users: &[tl::enums::User]) {
    for u in users {
        remember_self_id_impl(inner, u);
    }
    let mut cache: tokio::sync::RwLockWriteGuard<'_, PeerCache> = inner.peer_cache.write().await;
    cache.cache_users(users);
    drop(cache);
    mark_session_snapshot_dirty_impl(inner);
}

async fn cache_chats_slice_impl(inner: &Arc<ClientInner>, chats: &[tl::enums::Chat]) {
    let mut cache: tokio::sync::RwLockWriteGuard<'_, PeerCache> = inner.peer_cache.write().await;
    cache.cache_chats(chats);
    drop(cache);
    mark_session_snapshot_dirty_impl(inner);
}

/// Cache users and chats in a single write-lock acquisition.
async fn cache_users_and_chats_impl(
    inner: &Arc<ClientInner>,
    users: &[tl::enums::User],
    chats: &[tl::enums::Chat],
) {
    let mut cache: tokio::sync::RwLockWriteGuard<'_, PeerCache> = inner.peer_cache.write().await;
    cache.cache_users(users);
    cache.cache_chats(chats);
    drop(cache);
    mark_session_snapshot_dirty_impl(inner);
}

/// Cache-management utilities, offered as an alternate entry point for code
/// that issues raw RPC calls by hand and wants to feed the resulting
/// `users`/`chats` into the peer cache without going through `Client`'s
/// named methods (`cache_user`/`cache_entities`, unchanged and still the
/// primary public API). Both entry points call the same underlying
/// functions, so there's exactly one implementation of the cache-write
/// logic - no drift risk between the two.
///
/// Cheap to clone: internally Arc-wrapped, same as `Client`.
#[derive(Clone)]
pub struct ClientInternal {
    inner: Arc<ClientInner>,
}

impl ClientInternal {
    /// Cache a `User` object's ID, access hash, and display info.
    /// Equivalent to [`Client::cache_user`].
    pub async fn cache_user(&self, user: &tl::enums::User) {
        cache_user_impl(&self.inner, user).await;
    }

    pub async fn cache_users_slice(&self, users: &[tl::enums::User]) {
        cache_users_slice_impl(&self.inner, users).await;
    }

    pub async fn cache_chats_slice(&self, chats: &[tl::enums::Chat]) {
        cache_chats_slice_impl(&self.inner, chats).await;
    }

    /// Cache users and chats in a single write-lock acquisition.
    /// Equivalent to [`Client::cache_entities`].
    pub async fn cache_users_and_chats(
        &self,
        users: &[tl::enums::User],
        chats: &[tl::enums::Chat],
    ) {
        cache_users_and_chats_impl(&self.inner, users, chats).await;
    }
}

/// The main Telegram client. Cheap to clone: internally Arc-wrapped.
#[derive(Clone)]
pub struct Client {
    pub(crate) inner: Arc<ClientInner>,
    pub __internal: ClientInternal,
    _update_rx: Arc<Mutex<mpsc::Receiver<update::Update>>>,
}

mod auth;
mod bots;
mod chats;
mod dialogs;
mod files;
mod forum;
mod invites;
mod messages;
mod payments;
mod polls;
mod privacy;
mod reactions;
mod resolve;
mod settings;
mod stickers;
mod users;

impl Client {
    /// Return a fluent [`crate::ClientBuilder`] for constructing and connecting a client.
    ///
    /// # Example
    /// ```rust,no_run
    /// # use ferogram::Client;
    /// # #[tokio::main] async fn main() -> Result<(), Box<dyn std::error::Error>> {
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

    /// Connect to Telegram and log in (or resume an existing session) using
    /// `config`. This is the first thing you call - it returns your `Client`
    /// plus a [`ShutdownToken`] you can use to disconnect cleanly later.
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

        // Internal dispatch channel. Capacity comes from UpdateConfig; default 2048.
        // If the consumer falls behind, the overflow strategy in stream_updates()
        // decides whether to drop oldest or newest. Never the internal reader.
        let (update_tx, update_rx) = mpsc::channel(config.update_config.queue_capacity.max(1));

        // Load or fresh-connect
        let socks5 = config.socks5.clone();
        let mtproxy = config.mtproxy.clone();
        // mtproxy always dictates its own transport - ignore any user-set transport.
        let transport = if let Some(ref proxy) = mtproxy {
            proxy.transport.clone()
        } else {
            config.transport.clone()
        };
        let probe_transport = config.probe_transport;
        let resilient_connect = config.resilient_connect;

        let (conn, home_dc_id, dc_opts, media_dc_opts, loaded_session) = match config
            .session_backend
            .load()
            .map_err(InvocationError::Io)?
        {
            Some(s) => {
                if let Some(dc) = s.dcs.iter().find(|d| d.dc_id == s.home_dc_id) {
                    if let Some(key) = dc.auth_key {
                        tracing::info!(
                            "[ferogram::client] loading saved session (DC{})",
                            s.home_dc_id
                        );
                        tracing::debug!(
                            "[ferogram::client] session DC{} address: {}",
                            s.home_dc_id,
                            dc.addr
                        );
                        match Connection::connect_with_key(
                            &dc.addr,
                            key,
                            dc.first_salt,
                            dc.time_offset,
                            socks5.as_ref(),
                            mtproxy.as_ref(),
                            &transport,
                            s.home_dc_id as i16,
                            config.use_pfs,
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
                                tracing::debug!(
                                    "[ferogram::client] session DC table loaded: {} entries, {} media/CDN",
                                    opts.len(),
                                    media_opts.len()
                                );
                                (c, s.home_dc_id, opts, media_opts, Some(s))
                            }
                            Err(e) => {
                                // never call fresh_connect on a TCP blip during
                                // startup: that would silently destroy the saved session
                                // by switching to DC2 with a fresh key.  Return the error
                                // so the caller gets a clear failure and can retry or
                                // prompt for re-auth without corrupting the session file.
                                tracing::warn!(
                                    "[ferogram::client] session connect failed ({e}); trying next DC address \
                                         returning error (delete session file to reset)"
                                );
                                return Err(e.into());
                            }
                        }
                    } else {
                        tracing::info!(
                            "[ferogram::client] saved session for DC{} has no auth key; fresh login required",
                            s.home_dc_id
                        );
                        let (c, dc, opts) = Self::fresh_connect_resilient(
                            socks5.as_ref(),
                            mtproxy.as_ref(),
                            &transport,
                            probe_transport,
                            resilient_connect,
                            config.dc_addr.as_deref(),
                            config.dc_id_override,
                        )
                        .await?;
                        (c, dc, opts, HashMap::new(), None)
                    }
                } else {
                    tracing::info!(
                        "[ferogram::client] saved session has no entry for home DC{}; fresh login required",
                        s.home_dc_id
                    );
                    let (c, dc, opts) = Self::fresh_connect_resilient(
                        socks5.as_ref(),
                        mtproxy.as_ref(),
                        &transport,
                        probe_transport,
                        resilient_connect,
                        config.dc_addr.as_deref(),
                        config.dc_id_override,
                    )
                    .await?;
                    (c, dc, opts, HashMap::new(), None)
                }
            }
            None => {
                tracing::info!("[ferogram::client] no saved session found; fresh login required");
                let (c, dc, opts) = Self::fresh_connect_resilient(
                    socks5.as_ref(),
                    mtproxy.as_ref(),
                    &transport,
                    probe_transport,
                    resilient_connect,
                    config.dc_addr.as_deref(),
                    config.dc_id_override,
                )
                .await?;
                (c, dc, opts, HashMap::new(), None)
            }
        };

        // Build DC pool (used for API/federation calls)
        let pool = dc_pool::DcPool::new(
            home_dc_id,
            &dc_opts.values().cloned().collect::<Vec<DcEntry>>(),
            config.socks5.clone(),
            config.transport.clone(),
        );
        // Dedicated transfer pool  - separate connections for file upload/download.
        let transfer_pool = dc_pool::DcPool::new(
            home_dc_id,
            &dc_opts.values().cloned().collect::<Vec<DcEntry>>(),
            config.socks5.clone(),
            config.transport.clone(),
        );
        tracing::debug!(
            "[ferogram::client] connection pools initialized (home=DC{home_dc_id}, {} known DCs)",
            dc_opts.len()
        );

        // Hand the TcpStream directly to MtpSender: a single task owns both halves.
        let perm_auth_key = conn.perm_auth_key;
        tracing::debug!("[ferogram::client] spawning sender task for DC{home_dc_id}");
        let (sender_handle, frame_rx) = ferogram_mtsender::spawn_sender_task(
            conn.stream,
            conn.enc,
            conn.frame_kind,
            perm_auth_key,
        );

        // Notify for external "network restored" hints.
        let network_hint = std::sync::Arc::new(tokio::sync::Notify::new());

        // Graceful shutdown token.
        let shutdown_token = CancellationToken::new();
        let catch_up = config.catch_up;
        let restart_policy = config.restart_policy;

        let inner = Arc::new(ClientInner {
            rpc_tx: sender_handle.rpc_tx,
            reconnect_tx: sender_handle.reconnect_tx,
            network_hint,
            shutdown_token: shutdown_token.clone(),
            catch_up,
            restart_policy,
            home_dc_id: Mutex::new(home_dc_id),
            dc_options: Mutex::new(dc_opts),
            media_dc_options: Mutex::new(media_dc_opts),
            peer_cache: RwLock::new(PeerCache::new(config.experimental_features.clone())),
            message_box: Mutex::new(message_box::MessageBoxes::new()),
            dedupe_cache: parking_lot::Mutex::new(persist::BoundedDedupeCache::default()),
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
            transport: config.transport.clone(),
            active_transport: std::sync::Mutex::new(config.transport),
            session_backend: config.session_backend,
            dc_pool: Mutex::new(pool),
            transfer_pool: Mutex::new(transfer_pool),
            update_tx,
            is_bot: std::sync::atomic::AtomicBool::new(false),
            worker_semaphore: Arc::new(tokio::sync::Semaphore::new(
                config.transfer_limits.normalized().max_tcp_connections,
            )),
            transfer_limits: config.transfer_limits.normalized(),
            stream_active: std::sync::atomic::AtomicBool::new(false),

            dh_in_progress: std::sync::atomic::AtomicBool::new(false),
            pfs_enabled: config.use_pfs,
            update_config: config.update_config,
            signed_in: std::sync::atomic::AtomicBool::new(false),
            dc_connect_gates: parking_lot::Mutex::new(std::collections::HashMap::new()),
            auth_import_gates: parking_lot::Mutex::new(std::collections::HashMap::new()),
            auth_imported: parking_lot::Mutex::new(std::collections::HashSet::new()),
            peer_cache_miss_count: std::sync::atomic::AtomicU32::new(0),
            peer_cache_miss_window_start: parking_lot::Mutex::new(std::time::Instant::now()),
            last_bulk_hydration: parking_lot::Mutex::new(None),
            self_user_id: std::sync::atomic::AtomicI64::new(0),
            session_snapshot_dirty: std::sync::atomic::AtomicBool::new(false),
            experimental: config.experimental_features.clone(),
            diff_notify: std::sync::Arc::new(tokio::sync::Notify::new()),
            future_auth_token: parking_lot::Mutex::new(config.future_auth_token.clone().or_else(
                || {
                    loaded_session
                        .as_ref()
                        .and_then(|s| s.future_auth_token.clone())
                },
            )),
        });

        let client = Self {
            inner: inner.clone(),
            __internal: ClientInternal { inner },
            _update_rx: Arc::new(Mutex::new(update_rx)),
        };

        // Spawn the frame dispatch loop.
        // Receives FrameEvent from the sender task and routes updates / errors.
        // This replaces run_reader_task + reader_loop.
        {
            let client_d = client.clone();
            let shutdown_d = shutdown_token.clone();
            tokio::spawn(async move {
                client_d.run_frame_dispatch(frame_rx, shutdown_d).await;
            });
        }

        // Spawn the single persistent diff task: one sequential loop, woken
        // by diff_notify, instead of a new task per deadline tick.
        {
            let client_diff = client.clone();
            let shutdown_diff = shutdown_token.clone();
            tokio::spawn(async move {
                loop {
                    tokio::select! {
                        biased;
                        _ = shutdown_diff.cancelled() => return,
                        _ = client_diff.inner.diff_notify.notified() => {
                            client_diff.run_pending_differences().await;
                        }
                        _ = {
                            let deadline = {
                                let mut mb = client_diff.inner.message_box.lock().await;
                                mb.check_deadlines()
                            };
                            tokio::time::sleep_until(deadline.into())
                        } => {
                            client_diff.run_pending_differences().await;
                        }
                    }
                }
            });
        }

        // Periodic state saver: writes pts/qts/seq/date to the session backend
        // every 5 seconds if anything has changed. Uses the targeted Primary and
        // Secondary variants so only the update counters are touched, not the
        // full session blob. Runs a final save on shutdown.
        //
        // The actual backend call is synchronous, blocking I/O (SQLite write,
        // file write, ...); it's dispatched via spawn_blocking so it can't
        // stall this task's tokio worker thread. On a server multiplexing
        // many `Client`s over one runtime, every client runs this task, so a
        // blocking call here would otherwise periodically stall unrelated
        // clients sharing the same worker.
        {
            let client_ps = client.clone();
            let shutdown_ps = shutdown_token.clone();
            tokio::spawn(async move {
                let mut interval = tokio::time::interval(Duration::from_secs(5));
                interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
                interval.tick().await; // skip the first immediate tick
                let mut last_pts = -1i32;
                loop {
                    tokio::select! {
                        biased;
                        _ = shutdown_ps.cancelled() => {
                            // Final shutdown: snapshot from MessageBoxes and persist.
                            let snap = client_ps.inner.message_box.lock().await.session_state();
                            let (pts, qts, date, seq) = (snap.pts, snap.qts, snap.date, snap.seq);
                            if pts > 0 {
                                let backend = Arc::clone(&client_ps.inner.session_backend);
                                let _ = tokio::task::spawn_blocking(move || {
                                    let _ = backend.apply_update_state(
                                        ferogram_session::UpdateStateChange::Primary { pts, date, seq },
                                    );
                                    let _ = backend.apply_update_state(
                                        ferogram_session::UpdateStateChange::Secondary { qts },
                                    );
                                }).await;
                            }
                            break;
                        }
                        _ = interval.tick() => {
                            let snap = client_ps.inner.message_box.lock().await.session_state();
                            let (pts, qts, date, seq) = (snap.pts, snap.qts, snap.date, snap.seq);
                            if pts > last_pts {
                                let backend = Arc::clone(&client_ps.inner.session_backend);
                                let _ = tokio::task::spawn_blocking(move || {
                                    let _ = backend.apply_update_state(
                                        ferogram_session::UpdateStateChange::Primary { pts, date, seq },
                                    );
                                    let _ = backend.apply_update_state(
                                        ferogram_session::UpdateStateChange::Secondary { qts },
                                    );
                                }).await;
                                last_pts = pts;
                                tracing::debug!(
                                    "[ferogram::persist] periodic state snapshot saved (pts={pts}, qts={qts})"
                                );
                            }
                        }
                    }
                }
            });
        }

        // Full-session snapshot saver: flushes peers, channel_pts, min_peers, and
        // DC auth/salt data every 60 seconds whenever the peer cache has been
        // mutated since the last save. The dirty flag is set by mark_session_snapshot_dirty()
        // which is called after every peer-cache write. On shutdown a final save runs
        // unconditionally so no data is lost if the process exits between intervals.
        {
            let client_full = client.clone();
            let shutdown_full = shutdown_token.clone();
            tokio::spawn(async move {
                let mut interval = tokio::time::interval(Duration::from_secs(60));
                interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
                interval.tick().await;
                loop {
                    tokio::select! {
                        biased;
                        _ = shutdown_full.cancelled() => {
                            let _ = client_full.save_session().await;
                            break;
                        }
                        _ = interval.tick() => {
                            if client_full
                                .inner
                                .session_snapshot_dirty
                                .swap(false, std::sync::atomic::Ordering::AcqRel)
                            {
                                if let Err(e) = client_full.save_session().await {
                                    tracing::warn!("[ferogram::persist] full snapshot save failed: {e}");
                                    client_full
                                        .inner
                                        .session_snapshot_dirty
                                        .store(true, std::sync::atomic::Ordering::Release);
                                } else {
                                    tracing::debug!("[ferogram::persist] full snapshot saved");
                                }
                            }
                        }
                    }
                }
            });
        }

        // Background ACK flush task removed: MtpSender::step() flushes pending_ack
        // on every outgoing frame automatically, so a separate timer-based
        // flush is no longer needed.

        // Only clear the auth key on definitive bad-key signals from Telegram.
        // Network errors (EOF mid-session, ConnectionReset, Rpc(-404)) mean the
        // server rejected our key. Any other error (I/O, etc.) is left intact
        // no RPC timeout exists anymore, so there is no "timed out = stale key" case.
        if let Err(e) = client.init_connection().await {
            let key_is_stale = matches!(&e, InvocationError::Rpc(r) if r.code == -404);

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
                tracing::warn!(
                    "[ferogram::client] init_connection: auth key rejected by server ({e}); re-running DH"
                );
                {
                    let home_dc_id = *client.inner.home_dc_id.lock().await;
                    let mut opts: tokio::sync::MutexGuard<
                        '_,
                        std::collections::HashMap<i32, DcEntry>,
                    > = client.inner.dc_options.lock().await;
                    if let Some(entry) = opts.get_mut(&home_dc_id)
                        && entry.auth_key.is_some()
                    {
                        tracing::warn!(
                            "[ferogram::client] clearing stale auth key for DC{home_dc_id}"
                        );
                        entry.auth_key = None;
                        entry.first_salt = 0;
                        entry.time_offset = 0;
                    }
                }
                client.save_session().await.ok();
                // Pending RPCs are owned by the sender task; fail_all() handles this on error.

                let socks5_r = client.inner.socks5.clone();
                let mtproxy_r = client.inner.mtproxy.clone();
                let transport_r = client.inner.transport.clone();

                // reconnect to the HOME DC with fresh DH, not DC2.
                // fresh_connect() was hardcoded to DC2 and wiped all learned DC state,
                // which is why sessions on DC3/DC4/DC5 were corrupted on every -404.
                let home_dc_id_r = *client.inner.home_dc_id.lock().await;
                let addr_r = {
                    let opts: tokio::sync::MutexGuard<'_, std::collections::HashMap<i32, DcEntry>> =
                        client.inner.dc_options.lock().await;
                    opts.get(&home_dc_id_r)
                        .map(|e| e.addr.clone())
                        .unwrap_or_else(|| fallback_dc_addr(home_dc_id_r).to_string())
                };
                let new_conn = Connection::connect_raw(
                    &addr_r,
                    socks5_r.as_ref(),
                    mtproxy_r.as_ref(),
                    &transport_r,
                    home_dc_id_r as i16,
                )
                .await?;

                // Read key/salt directly from the connection (single TcpStream owner now).
                let new_ak = new_conn.enc.auth_key_bytes();
                let new_first_salt = new_conn.enc.salt;
                let new_time_offset = new_conn.enc.time_offset;
                // Update ONLY the home DC entry: all other DC keys are preserved.
                {
                    let mut opts_guard: tokio::sync::MutexGuard<
                        '_,
                        std::collections::HashMap<i32, DcEntry>,
                    > = client.inner.dc_options.lock().await;
                    if let Some(entry) = opts_guard.get_mut(&home_dc_id_r) {
                        entry.auth_key = Some(new_ak);
                        entry.first_salt = new_first_salt;
                        entry.time_offset = new_time_offset;
                    }
                }
                // home_dc_id stays unchanged: we reconnected to the same DC.
                let perm_auth_key = new_conn.perm_auth_key;
                let _ = client
                    .inner
                    .reconnect_tx
                    .send(ferogram_mtsender::ReconnectRequest {
                        stream: new_conn.stream,
                        enc: new_conn.enc,
                        frame_kind: new_conn.frame_kind,
                        perm_auth_key,
                    })
                    .await;
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
                    "[ferogram::client] session invalidated by server; fresh login required"
                );
            } else {
                return Err(e);
            }
        }

        // After connect() returns, the caller must call bot_sign_in() or sign_in().
        // sync_pts_state() is called there, after authentication succeeds.
        // Calling GetState here (before auth) always returns AUTH_KEY_UNREGISTERED.

        // Restore peer access-hash cache from session
        if let Some(ref s) = loaded_session
            && !s.peers.is_empty()
        {
            let mut cache: tokio::sync::RwLockWriteGuard<'_, PeerCache> =
                client.inner.peer_cache.write().await;
            for p in &s.peers {
                if p.is_community {
                    cache.communities.entry(p.id).or_insert(p.access_hash);
                } else if p.is_chat {
                    cache.chats.insert(p.id);
                } else if p.is_channel {
                    if p.access_hash != 0 {
                        cache
                            .channels
                            .entry(p.id)
                            .or_insert((p.access_hash, p.channel_kind.map(Into::into)));
                    } else {
                        // Min channel: access_hash was 0 at save time.
                        // Only restore to channels_min if no full hash yet.
                        if !cache.channels.contains_key(&p.id) {
                            cache.channels_min.insert(p.id);
                        }
                    }
                } else {
                    cache.users.entry(p.id).or_insert(p.access_hash);
                }
            }
            for m in &s.min_peers {
                // Only restore if not already upgraded to a full user entry.
                if !cache.users.contains_key(&m.user_id) {
                    cache.min_contexts.insert(m.user_id, (m.peer_id, m.msg_id));
                }
            }
            tracing::debug!(
                "[ferogram::client] peer cache restored: {} users, {} channels, {} communities, {} chats, {} channels_min, {} min-peer contexts",
                cache.users.len(),
                cache.channels.len(),
                cache.communities.len(),
                cache.chats.len(),
                cache.channels_min.len(),
                cache.min_contexts.len(),
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
            // Session file has a valid auth key → client is already authorised.
            client
                .inner
                .signed_in
                .store(true, std::sync::atomic::Ordering::SeqCst);
            let snap = &loaded_session.as_ref().unwrap().updates_state;
            // Restore MessageBoxes from session snapshot.
            {
                let mb_snap = message_box::UpdatesStateSnap {
                    pts: snap.pts,
                    qts: snap.qts,
                    date: snap.date,
                    seq: snap.seq,
                    channels: snap
                        .channels
                        .iter()
                        .map(|&(id, pts)| message_box::ChannelState { id, pts })
                        .collect(),
                };
                *client.inner.message_box.lock().await = message_box::MessageBoxes::load(mb_snap);
                tracing::info!(
                    "[ferogram::client] update state restored: pts={}, qts={}, seq={}, {} channels tracked",
                    snap.pts,
                    snap.qts,
                    snap.seq,
                    snap.channels.len()
                );
            }

            // Spawn catch-up: MessageBoxes::load already marked all entries as needing diff.
            // run_pending_differences will drive getDifference + all getChannelDifference calls.
            //
            // Access hashes are resolved lazily:
            //   1. Channel access_hashes from the previous session are already restored
            //      into PeerCache above (from s.peers).
            //   2. Any channels not yet known will receive their access_hash from the
            //      entities embedded in future updates / getDifference responses.
            //   3. If getChannelDifference is needed for a channel whose hash is still
            //      missing, run_pending_differences skips it (end_channel_difference Banned)
            //      and continues; the hash will arrive via a subsequent update entity.
            //
            // We do NOT call prefetch_channel_access_hashes() (messages.getDialogs) here.
            // That call forces deep deserialization of Dialog/DraftMessage/PollResults/Story
            // which are high-churn Telegram objects and break on every new beta layer.
            tracing::debug!(
                "[ferogram::client] scheduling getDifference for catch-up after reconnect"
            );
            client.inner.diff_notify.notify_one();
        } else {
            // If there is a loaded session the client is already authorised.
            // Mark signed_in so sync_state_after_dh can run after reconnects,
            // then sync pts from the server.
            // Fresh sessions (no loaded_session) skip sync here entirely:
            // sync_pts_state is already called inside bot_sign_in / sign_in
            // after auth completes, so calling it now would produce a guaranteed
            // AUTH_KEY_UNREGISTERED 401 before the credential is exchanged.
            if loaded_session.is_some() {
                client
                    .inner
                    .signed_in
                    .store(true, std::sync::atomic::Ordering::SeqCst);
                let _ = client.sync_pts_state().await;
                // Channel access_hashes are resolved lazily:
                //   - Already-seen channels are restored from session (s.peers above).
                //   - New channels receive their hash from update entities.
                //   - We do NOT call messages.getDialogs here; that path forces
                //     deep deserialization of Dialog/PollResults/DraftMessage/Story
                //     which are high-churn objects that break on every Telegram beta.
            }
        }

        Ok((client, shutdown_token))
    }

    /// Race Obfuscated / Abridged / Http transports using `Connection::connect_raw`.
    /// The winner is returned directly - no second DH handshake.
    /// Logs per-transport start, result, and elapsed time in ms.
    async fn probe_transports_race(
        addr: &str,
        socks5: Option<&crate::socks5::Socks5Config>,

        dc_id: i16,
    ) -> Result<Connection, InvocationError> {
        use tokio::task::JoinSet;
        let mut set: JoinSet<Result<(Connection, &'static str, u64), InvocationError>> =
            JoinSet::new();

        // Obfuscated - starts immediately (best for DPI-heavy networks)
        {
            let a = addr.to_owned();
            let s = socks5.cloned();
            set.spawn(async move {
                tracing::debug!("[ferogram::connect] probing transport: Obfuscated (t=0ms)");
                let t0 = tokio::time::Instant::now();
                match Connection::connect_raw(
                    &a,
                    s.as_ref(),
                    None,
                    &TransportKind::Obfuscated { secret: None },
                    dc_id,
                )
                .await
                {
                    Ok(c) => {
                        let ms = t0.elapsed().as_millis() as u64;
                        tracing::debug!(
                            "[ferogram::connect] Obfuscated transport DH complete in {ms}ms"
                        );
                        Ok((c, "Obfuscated", ms))
                    }
                    Err(e) => {
                        let ms = t0.elapsed().as_millis() as u64;
                        tracing::debug!(
                            "[ferogram::connect] Obfuscated transport failed after {ms}ms: {e}"
                        );
                        Err(InvocationError::from(e))
                    }
                }
            });
        }

        // Abridged - 200 ms stagger
        {
            let a = addr.to_owned();
            let s = socks5.cloned();
            set.spawn(async move {
                tracing::debug!("[ferogram::connect] probing transport: Abridged (t=200ms)");
                tokio::time::sleep(std::time::Duration::from_millis(200)).await;
                let t0 = tokio::time::Instant::now();
                match Connection::connect_raw(&a, s.as_ref(), None, &TransportKind::Abridged, dc_id)
                    .await
                {
                    Ok(c) => {
                        let ms = t0.elapsed().as_millis() as u64;
                        tracing::debug!(
                            "[ferogram::connect] Abridged transport DH complete in {ms}ms"
                        );
                        Ok((c, "Abridged", ms))
                    }
                    Err(e) => {
                        let ms = t0.elapsed().as_millis() as u64;
                        tracing::debug!(
                            "[ferogram::connect] Abridged transport failed after {ms}ms: {e}"
                        );
                        Err(InvocationError::from(e))
                    }
                }
            });
        }

        // Http - 800 ms stagger (last resort, no socks5)
        {
            let a = addr.to_owned();
            set.spawn(async move {
                tracing::debug!("[ferogram::connect] probing transport: HTTP (t=800ms)");
                tokio::time::sleep(std::time::Duration::from_millis(800)).await;
                let t0 = tokio::time::Instant::now();
                match Connection::connect_raw(&a, None, None, &TransportKind::Http, dc_id).await {
                    Ok(c) => {
                        let ms = t0.elapsed().as_millis() as u64;
                        tracing::debug!("[ferogram::connect] HTTP transport DH complete in {ms}ms");
                        Ok((c, "Http", ms))
                    }
                    Err(e) => {
                        let ms = t0.elapsed().as_millis() as u64;
                        tracing::debug!(
                            "[ferogram::connect] HTTP transport failed after {ms}ms: {e}"
                        );
                        Err(InvocationError::from(e))
                    }
                }
            });
        }

        let mut last_err =
            InvocationError::Deserialize("probe_transports_race: all transports failed".into());
        while let Some(outcome) = set.join_next().await {
            match outcome {
                Ok(Ok((conn, label, ms))) => {
                    set.abort_all();
                    tracing::info!(
                        "[ferogram::connect] transport selected: {label} ({ms}ms); reusing connection"
                    );
                    // drain cancelled tasks
                    while let Some(r) = set.join_next().await {
                        if let Err(e) = r
                            && e.is_cancelled()
                        {
                            tracing::debug!(
                                "[ferogram::connect] slower transport probe cancelled (faster one won)"
                            );
                        }
                    }
                    return Ok(conn);
                }
                Ok(Err(e)) => {
                    last_err = e;
                }
                Err(e) if e.is_cancelled() => {}
                Err(_) => {}
            }
        }
        Err(last_err)
    }

    /// Fresh connect with optional transport probing and resilient fallback.
    async fn fresh_connect_resilient(
        socks5: Option<&crate::socks5::Socks5Config>,
        mtproxy: Option<&crate::proxy::MtProxyConfig>,

        transport: &TransportKind,
        probe_transport: bool,

        resilient_connect: bool,
        dc_addr_override: Option<&str>,
        dc_id_override: Option<i32>,
    ) -> Result<(Connection, i32, HashMap<i32, DcEntry>), InvocationError> {
        let dc_id: i16 = dc_id_override.unwrap_or(2) as i16;
        let default_addr = dc_addr_override
            .map(|addr| addr.to_owned())
            .unwrap_or_else(|| fallback_dc_addr(dc_id as i32).to_owned());

        let build_opts = || -> HashMap<i32, DcEntry> {
            session::default_dc_addresses()
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
                .collect()
        };

        // Transport probing: race transports; winner becomes the final connection.
        if probe_transport && mtproxy.is_none() {
            tracing::info!(
                "[ferogram::connect] probing DC{dc_id}: racing Obfuscated, Abridged, and HTTP transports in parallel"
            );
            match Self::probe_transports_race(&default_addr, socks5, dc_id).await {
                Ok(conn) => return Ok((conn, dc_id as i32, build_opts())),
                Err(e) => {
                    tracing::warn!(
                        "[ferogram::connect] all transport probes failed ({e}); falling back to default Full transport"
                    );
                }
            }
        }

        // Normal direct connect.
        if dc_addr_override.is_some() || dc_id_override.is_some() {
            tracing::debug!(
                "[ferogram::connect] using override for DC{dc_id}; connecting to address {default_addr}"
            );
        } else {
            tracing::debug!(
                "[ferogram::connect] no address override for DC{dc_id}; connecting to default address {default_addr}"
            );
        }
        let direct_result =
            Connection::connect_raw(&default_addr, socks5, mtproxy, transport, dc_id).await;

        if let Ok(conn) = direct_result {
            return Ok((conn, dc_id as i32, build_opts()));
        }
        let direct_err = direct_result.err().unwrap();

        if !resilient_connect {
            return Err(direct_err.into());
        }

        // DNS-over-HTTPS fallback.
        tracing::warn!(
            "[ferogram::connect] direct connect failed ({direct_err}); trying DoH fallback"
        );
        let resolver = crate::dns_resolver::DnsResolver::new();
        let doh_ips = resolver.resolve("venus.web.telegram.org").await;
        let port = default_addr.split(':').next_back().unwrap_or("443");
        for ip in &doh_ips {
            let addr = format!("{ip}:{port}");
            tracing::info!(
                "[ferogram::connect] DoH resolved DC{dc_id} to {addr}; attempting connection"
            );
            match Connection::connect_raw(&addr, socks5, mtproxy, transport, dc_id).await {
                Ok(conn) => {
                    tracing::info!(
                        "[ferogram::connect] DoH fallback: connected to DC{dc_id} via {addr}"
                    );
                    return Ok((conn, dc_id as i32, build_opts()));
                }
                Err(e) => tracing::debug!("[ferogram::connect] DoH address {addr} failed: {e}"),
            }
        }

        // Firebase / Google special-config fallback.
        tracing::warn!(
            "[ferogram::connect] DoH fallback exhausted ({} candidates); trying Firebase",
            doh_ips.len()
        );
        let special = crate::special_config::SpecialConfig::new();
        match special.fetch().await {
            Some(dc_options) => {
                for opt in dc_options.iter().filter(|o| o.dc_id == dc_id as i32) {
                    let addr = format!("{}:{}", opt.ip, opt.port);
                    tracing::info!(
                        "[ferogram::connect] Firebase fallback: trying DC{} address {addr}",
                        opt.dc_id
                    );
                    match Connection::connect_raw(&addr, socks5, mtproxy, transport, dc_id).await {
                        Ok(conn) => {
                            tracing::info!(
                                "[ferogram::connect] Firebase fallback: connected to DC{dc_id} via {addr}"
                            );
                            return Ok((conn, dc_id as i32, build_opts()));
                        }
                        Err(e) => tracing::debug!(
                            "[ferogram::connect] Firebase address {addr} failed: {e}"
                        ),
                    }
                }
                Err(InvocationError::Io(std::io::Error::new(
                    std::io::ErrorKind::ConnectionRefused,
                    "all resilient connect strategies exhausted",
                )))
            }
            None => Err(InvocationError::Io(std::io::Error::new(
                std::io::ErrorKind::ConnectionRefused,
                "all resilient connect strategies exhausted (Firebase unavailable)",
            ))),
        }
    }

    /// Build a [`PersistedSession`] snapshot from current client state.
    ///
    /// Single source of truth used by both [`save_session`] and
    /// [`export_session_string`]: any serialisation change only needs
    /// to be made here.
    async fn build_persisted_session(&self) -> PersistedSession {
        use crate::session::{CachedPeer, UpdatesStateSnap};

        let home_dc_id = *self.inner.home_dc_id.lock().await;
        let dc_options: tokio::sync::MutexGuard<'_, std::collections::HashMap<i32, DcEntry>> =
            self.inner.dc_options.lock().await;

        let mut dcs: Vec<DcEntry> = dc_options.values().cloned().collect();
        // Also persist media DCs so they survive restart.
        {
            let media_opts: tokio::sync::MutexGuard<
                '_,
                std::collections::HashMap<i32, crate::DcEntry>,
            > = self.inner.media_dc_options.lock().await;
            for e in media_opts.values() {
                dcs.push(e.clone());
            }
        }
        self.inner.dc_pool.lock().await.collect_keys(&mut dcs);
        // Also collect auth keys from the transfer pool so that after restart
        // foreign-DC transfer workers can be re-authenticated without a full
        // DH round-trip.  Without this, every restart seeds transfer connections
        // from stale dc_options and triggers AUTH_KEY_UNREGISTERED on the first use.
        self.inner.transfer_pool.lock().await.collect_keys(&mut dcs);

        let pts_snap = {
            let snap = self.inner.message_box.lock().await.session_state();
            UpdatesStateSnap {
                pts: snap.pts,
                qts: snap.qts,
                date: snap.date,
                seq: snap.seq,
                channels: snap.channels.iter().map(|c| (c.id, c.pts)).collect(),
            }
        };

        let peers: Vec<CachedPeer> = {
            let cache: tokio::sync::RwLockReadGuard<'_, PeerCache> =
                self.inner.peer_cache.read().await;
            let mut v = Vec::with_capacity(
                cache.users.len()
                    + cache.channels.len()
                    + cache.communities.len()
                    + cache.chats.len()
                    + cache.channels_min.len(),
            );
            for (&id, &hash) in &cache.users {
                v.push(CachedPeer {
                    id,
                    access_hash: hash,
                    is_channel: false,
                    is_chat: false,
                    channel_kind: None,
                    is_community: false,
                });
            }
            for (&id, &(hash, kind)) in &cache.channels {
                v.push(CachedPeer {
                    id,
                    access_hash: hash,
                    is_channel: true,
                    is_chat: false,
                    channel_kind: kind.map(Into::into),
                    is_community: false,
                });
            }
            for &id in &cache.chats {
                v.push(CachedPeer {
                    id,
                    access_hash: 0,
                    is_channel: false,
                    is_chat: true,
                    channel_kind: None,
                    is_community: false,
                });
            }
            // channels_min: type byte 3. No access_hash; just existence tracking.
            // Re-populated quickly on reconnect, but persisting avoids false "unknown peer" logs.
            for &id in &cache.channels_min {
                v.push(CachedPeer {
                    id,
                    access_hash: 0,
                    is_channel: true,
                    is_chat: false,
                    channel_kind: None,
                    is_community: false,
                });
            }
            // Communities: same wire shape as a channel, cached separately so
            // they round-trip as communities instead of collapsing into a
            // plain channel entry on reload.
            for (&id, &hash) in &cache.communities {
                v.push(CachedPeer {
                    id,
                    access_hash: hash,
                    is_channel: false,
                    is_chat: false,
                    channel_kind: None,
                    is_community: true,
                });
            }
            v
        };

        let min_peers: Vec<session::CachedMinPeer> = {
            let cache: tokio::sync::RwLockReadGuard<'_, PeerCache> =
                self.inner.peer_cache.read().await;
            cache
                .min_contexts
                .iter()
                .map(|(&user_id, &(peer_id, msg_id))| session::CachedMinPeer {
                    user_id,
                    peer_id,
                    msg_id,
                })
                .collect()
        };

        PersistedSession {
            home_dc_id,
            dcs,
            updates_state: pts_snap,
            peers,
            min_peers,
            future_auth_token: self.inner.future_auth_token.lock().clone(),
        }
    }

    /// Persist the current session to the configured [`crate::SessionBackend`].
    ///
    /// `SessionBackend` implementations (SQLite, binary file, ...) are
    /// synchronous, blocking calls under the hood. Running them on the
    /// caller's task would tie up a tokio worker thread for the duration of
    /// the disk I/O; on a server multiplexing many clients over a shared
    /// runtime, that stalls every other client's tasks on that worker for
    /// as long as the write takes. Every actual backend call here goes
    /// through [`tokio::task::spawn_blocking`] so it runs on the blocking
    /// thread pool instead.
    pub async fn save_session(&self) -> Result<(), InvocationError> {
        // build_persisted_session() is the source of truth for structural
        // session data: auth key, salts, DC table, peer cache.
        // MessageBoxes is the single authoritative source for pts/qts/date/seq.
        // build_persisted_session() already reads from message_box.session_state(),
        // so no secondary overwrite is needed.
        let session = self.build_persisted_session().await;

        let backend = Arc::clone(&self.inner.session_backend);
        let session_for_save = session.clone();
        tokio::task::spawn_blocking(move || backend.save(&session_for_save))
            .await
            .map_err(|e| InvocationError::Io(std::io::Error::other(e)))?
            .map_err(InvocationError::Io)?;

        // Secondary monotonic guard (defence-in-depth):
        //   SQL backends   MAX() in write_session absorbs any residual race; no-op.
        //   BinaryFile     re-applies the same fresh values written above.
        //   InMemory       same; low risk but keeps the invariant unbreakable.
        {
            let (pts, qts, date, seq) = (
                session.updates_state.pts,
                session.updates_state.qts,
                session.updates_state.date,
                session.updates_state.seq,
            );
            if pts > 0 {
                let backend = Arc::clone(&self.inner.session_backend);
                let _ =
                    tokio::task::spawn_blocking(move || {
                        let _ = backend.apply_update_state(
                            ferogram_session::UpdateStateChange::Primary { pts, date, seq },
                        );
                        let _ = backend.apply_update_state(
                            ferogram_session::UpdateStateChange::Secondary { qts },
                        );
                    })
                    .await;
            }
        }

        tracing::info!("[ferogram::client] session saved to disk");
        Ok(())
    }

    /// Export the session as a compact string (V2 format).
    ///
    /// Encodes dc_id, ip, port, user_id, and auth key. Store in an env var
    /// or secret manager and pass back to [`crate::ClientBuilder::session_string`]
    /// to resume without re-authenticating.
    ///
    /// Calls `get_me()` internally to obtain the user_id.
    pub async fn export_session_string(&self) -> Result<String, InvocationError> {
        use ferogram_session::string_session::{Session, StringSession};

        let me = self.get_me().await?;
        let persisted = self.build_persisted_session().await;
        let home_dc_id = persisted.home_dc_id;

        let dc = persisted.dc_for(home_dc_id, false).and_then(|dc| {
            let auth_key = dc.auth_key?;
            let socket_addr = dc.socket_addr().ok()?;
            Some((auth_key, socket_addr))
        });

        if let Some((auth_key, socket_addr)) = dc {
            let ss = StringSession::V2(Session {
                dc_id: home_dc_id as u8,
                ip: socket_addr.ip(),
                port: socket_addr.port(),
                auth_key,
                user_id: me.id,
            });
            Ok(ss.encode())
        } else {
            Ok(persisted.to_string())
        }
    }

    /// Export the full native session string (DC table, update state, peer cache).
    ///
    /// Use this when you need to resume update processing from exactly where
    /// you left off (PTS, QTS, seq, peer cache intact). Pass the result back
    /// to [`crate::ClientBuilder::session_string`] which auto-detects the format.
    pub async fn export_native_session_string(&self) -> Result<String, InvocationError> {
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
        let media: tokio::sync::MutexGuard<'_, std::collections::HashMap<i32, DcEntry>> =
            self.inner.media_dc_options.lock().await;
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
                tracing::warn!(
                    "[ferogram::client] is_authorized: auth.getState failed ({e}); assuming not authorized"
                );
                Ok(false)
            }
            Err(e) => Err(e),
        }
    }

    /// Return an [`UpdateStream`] that yields incoming [`crate::Update`]s.
    ///
    /// The reader task sends all updates to `inner.update_tx`.  This method
    /// proxies those updates into a caller-owned channel, applying the
    /// overflow strategy from [`UpdateConfig`]:
    ///
    /// * **`DropOldest`** (default) - a `VecDeque` ring buffer of
    ///   `queue_capacity` slots is maintained in the proxy task. When it is
    ///   full, the stalest *Ephemeral* update (typing, online status) is
    ///   evicted first; if there are none, the oldest *Normal* update is
    ///   evicted. The incoming update is always accepted.
    /// * **`DropNewest`** - the incoming update is dropped if the channel is
    ///   full (original `try_send` behaviour).
    ///
    /// In both cases, internal MTProto state (pts, qts, getDifference) is
    /// never touched here. It runs exclusively in the reader task.
    ///
    /// [`UpdateConfig`]: crate::update_config::UpdateConfig
    pub fn stream_updates(&self) -> UpdateStream {
        use crate::update_config::{OverflowStrategy, UpdatePriority, update_priority};
        use std::collections::VecDeque;

        // Guard: only one UpdateStream is supported per Client clone group.
        // A second call would compete for updates, causing non-deterministic
        // splitting.  Return a closed stream so the caller's loop exits cleanly.
        if self
            .inner
            .stream_active
            .swap(true, std::sync::atomic::Ordering::SeqCst)
        {
            tracing::error!(
                "[ferogram::client] stream_updates() called twice on the same Client; replacing the existing stream"
            );
            let (_dead_tx, rx) = mpsc::channel::<update::Update>(1);
            return UpdateStream { rx };
        }

        let capacity = self.inner.update_config.queue_capacity.max(1);
        let overflow = self.inner.update_config.overflow_strategy;
        let internal_rx = self._update_rx.clone();

        // The caller-facing channel needs at least `capacity` slots so the
        // proxy task can drain the ring into it without blocking.
        let (caller_tx, rx) = mpsc::channel::<update::Update>(capacity);

        tokio::spawn(async move {
            let mut guard = internal_rx.lock().await;

            match overflow {
                // DropOldest: priority-aware ring buffer
                OverflowStrategy::DropOldest => {
                    // Ring buffer: front = oldest, back = newest.
                    let mut ring: VecDeque<update::Update> = VecDeque::with_capacity(capacity);

                    while let Some(upd) = guard.recv().await {
                        // Try to drain the ring into the caller channel first.
                        while !ring.is_empty() {
                            match caller_tx.try_send(
                                // peek then pop; try_send takes ownership
                                ring.pop_front().expect("just peeked"),
                            ) {
                                Ok(()) => {}
                                Err(tokio::sync::mpsc::error::TrySendError::Full(returned)) => {
                                    // Caller not keeping up; put it back and stop draining.
                                    ring.push_front(returned);
                                    break;
                                }
                                Err(tokio::sync::mpsc::error::TrySendError::Closed(_)) => {
                                    // Caller dropped the UpdateStream; shut down.
                                    return;
                                }
                            }
                        }

                        // Deliver directly when ring is empty; buffering here causes one-update lag.
                        if ring.is_empty() {
                            match caller_tx.try_send(upd) {
                                Ok(()) => {}
                                Err(tokio::sync::mpsc::error::TrySendError::Full(returned)) => {
                                    ring.push_back(returned);
                                }
                                Err(tokio::sync::mpsc::error::TrySendError::Closed(_)) => {
                                    return;
                                }
                            }
                        } else if ring.len() < capacity {
                            // Ring still has backlog; preserve ordering.
                            ring.push_back(upd);
                        } else {
                            // Ring is full: evict the stalest Ephemeral slot first,
                            // then fall back to the oldest Normal slot.
                            let ephemeral_pos = ring
                                .iter()
                                .position(|u| update_priority(u) == UpdatePriority::Ephemeral);

                            if let Some(pos) = ephemeral_pos {
                                ring.remove(pos);
                            } else {
                                // No ephemerals; drop the oldest update regardless.
                                ring.pop_front();
                            }

                            metrics::counter!("ferogram.updates_dropped").increment(1);
                            tracing::debug!(
                                "[ferogram::client] update queue full (capacity {}): dropping update (consumer is falling behind)",
                                capacity
                            );
                            ring.push_back(upd);
                        }
                    }

                    // Reader task exited (disconnect): flush remaining ring to caller.
                    for queued in ring {
                        // Best-effort; ignore send errors (caller may have dropped stream).
                        let _ = caller_tx.send(queued).await;
                    }
                }

                // DropNewest: simple try_send (original behaviour)
                OverflowStrategy::DropNewest => {
                    while let Some(upd) = guard.recv().await {
                        if caller_tx.try_send(upd).is_err() {
                            tracing::warn!(
                                "[ferogram::client] update dropped: consumer too slow (queue depth >= {})",
                                capacity
                            );
                            metrics::counter!("ferogram.updates_dropped").increment(1);
                        }
                    }
                }
            }
        });

        UpdateStream { rx }
    }

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
        self.inner.network_hint.notify_one();
    }

    #[allow(clippy::too_many_arguments)]
    /// Frame dispatch loop, receives [`FrameEvent`] from the sender task and
    /// routes updates, connection errors, and reconnects. Replaces the old
    /// `run_reader_task` + `reader_loop` split-architecture pair.
    ///
    /// The sender task owns the TCP stream; this task just dispatches what
    /// comes out of it.
    async fn run_frame_dispatch(
        &self,
        mut frame_rx: tokio::sync::mpsc::Receiver<ferogram_mtsender::FrameEvent>,
        shutdown_token: tokio_util::sync::CancellationToken,
    ) {
        use ferogram_mtsender::FrameEvent;

        loop {
            tokio::select! {
                biased;
                _ = shutdown_token.cancelled() => {
                    tracing::info!("[ferogram::client] frame dispatch loop exiting cleanly");
                    return;
                }
                event = frame_rx.recv() => {
                    match event {
                        None => {
                            tracing::warn!("[ferogram::client] sender task exited unexpectedly; reconnect will be attempted");
                            return;
                        }
                        Some(FrameEvent::Connected { auth_key, first_salt, time_offset, session_id }) => {
                            tracing::debug!(
                                "[ferogram::client] frame dispatch connected (session_id={session_id:#x}, salt={first_salt:#018x})"
                            );
                            // Keep dc_options[home_dc] in sync so build_persisted_session
                            // and other readers don't need access to the sender task's
                            // internal EncryptedSession state.
                            {
                                let home_dc_id = *self.inner.home_dc_id.lock().await;
                                let mut opts = self.inner.dc_options.lock().await;
                                if let Some(entry) = opts.get_mut(&home_dc_id) {
                                    entry.auth_key = Some(*auth_key);
                                    entry.first_salt = first_salt;
                                    entry.time_offset = time_offset;
                                }
                            }
                            // Signal message_box so it queues getDifference.
                            {
                                let mut mb = self.inner.message_box.lock().await;
                                let _ = mb.process_updates(
                                    crate::message_box::UpdatesLike::ConnectionClosed,
                                );
                            }
                            // Drive catchup diffs on reconnect.
                            self.inner.diff_notify.notify_one();
                        }
                        Some(FrameEvent::Update(body)) => {
                            self.dispatch_updates(&body).await;
                        }
                        Some(FrameEvent::Error(e)) => {
                            tracing::warn!("[ferogram::client] connection error in frame dispatch: {e:?}");
                            // Sender task already called fail_all(); just reconnect.
                            self.handle_reconnect_after_error().await;
                        }
                    }
                }
            }
        }
    }

    /// Reconnect after a connection error from the sender task.
    /// Uses exponential backoff, then sends the new stream via reconnect_tx.
    /// Tries connect_with_key first (reusing the existing auth key); falls back
    /// to a fresh DH only if the key is missing or the server rejects it.
    async fn handle_reconnect_after_error(&self) {
        let mut delay_ms = RECONNECT_BASE_MS;
        loop {
            // Wait for backoff to expire OR a network-restored hint from the caller.
            // signal_network_restored() fires notify_one() which cancels the sleep
            // immediately so we try the next connect attempt right away.
            tokio::select! {
                _ = tokio::time::sleep(std::time::Duration::from_millis(delay_ms)) => {}
                _ = self.inner.network_hint.notified() => {
                    tracing::debug!("[ferogram::client] network hint received; skipping backoff");
                }
            }

            let (addr, dc_id, saved_key, first_salt, time_offset) = {
                let home = *self.inner.home_dc_id.lock().await;
                let dc_opts = self.inner.dc_options.lock().await;
                match dc_opts.get(&home) {
                    Some(e) => (
                        e.addr.clone(),
                        home as i16,
                        e.auth_key,
                        e.first_salt,
                        e.time_offset,
                    ),
                    None => {
                        tracing::error!(
                            "[ferogram::client] reconnect failed: no address entry for home DC {home}; cannot reconnect"
                        );
                        delay_ms = (delay_ms * 2).min(RECONNECT_MAX_SECS * 1000);
                        continue;
                    }
                }
            };

            let socks5 = self.inner.socks5.as_ref().cloned();
            let mtproxy = self.inner.mtproxy.as_ref().cloned();
            let transport = self.inner.active_transport.lock().unwrap().clone();
            let pfs = self.inner.pfs_enabled;

            // Try existing key first; only do full DH if key is absent or rejected.
            let conn_result = if let Some(auth_key) = saved_key {
                tracing::debug!(
                    "[ferogram::client] reconnect: reusing cached auth key for DC{dc_id}"
                );
                match Connection::connect_with_key(
                    &addr,
                    auth_key,
                    first_salt,
                    time_offset,
                    socks5.as_ref(),
                    mtproxy.as_ref(),
                    &transport,
                    dc_id,
                    pfs,
                )
                .await
                {
                    Ok(c) => Ok(c),
                    Err(e @ ferogram_connect::ConnectError::Io(_)) => {
                        tracing::warn!(
                            "[ferogram::client] reconnect: network error with cached key ({e}); will retry"
                        );
                        Err(e)
                    }
                    Err(e) => {
                        tracing::warn!(
                            "[ferogram::client] reconnect: auth key rejected ({e}); falling back to fresh DH"
                        );
                        {
                            let mut opts = self.inner.dc_options.lock().await;
                            if let Some(entry) = opts.get_mut(&(dc_id as i32)) {
                                entry.auth_key = None;
                                entry.first_salt = 0;
                                entry.time_offset = 0;
                            }
                        }
                        Connection::connect_raw(
                            &addr,
                            socks5.as_ref(),
                            mtproxy.as_ref(),
                            &transport,
                            dc_id,
                        )
                        .await
                    }
                }
            } else {
                tracing::debug!(
                    "[ferogram::client] reconnect: no cached auth key for DC{dc_id}; running DH"
                );
                Connection::connect_raw(&addr, socks5.as_ref(), mtproxy.as_ref(), &transport, dc_id)
                    .await
            };

            match conn_result {
                Ok(conn) => {
                    tracing::info!(
                        "[ferogram::client] reconnect: TCP connection established and auth confirmed"
                    );
                    {
                        let mut opts = self.inner.dc_options.lock().await;
                        if let Some(entry) = opts.get_mut(&(dc_id as i32)) {
                            entry.auth_key = Some(conn.auth_key_bytes());
                            entry.first_salt = conn.enc.salt;
                            entry.time_offset = conn.enc.time_offset;
                        }
                    }
                    let perm_auth_key = conn.perm_auth_key;
                    let _ = self
                        .inner
                        .reconnect_tx
                        .send(ferogram_mtsender::ReconnectRequest {
                            stream: conn.stream,
                            enc: conn.enc,
                            frame_kind: conn.frame_kind,
                            perm_auth_key,
                        })
                        .await;
                    tokio::task::yield_now().await;

                    if let Err(e) = self.init_connection().await {
                        tracing::warn!(
                            "[ferogram::client] reconnect: init_connection failed after TCP established: {e}"
                        );
                        // If Telegram rejected the auth key at the RPC level (AUTH_KEY_UNREGISTERED
                        // after TV sleep or extended inactivity), clear the cached key and loop
                        // back so the next iteration runs fresh DH instead of reusing the dead key.
                        if matches!(&e, InvocationError::Rpc(r) if r.code == 401) {
                            tracing::warn!(
                                "[ferogram::client] reconnect: auth key rejected by server (401); clearing cached key and retrying with fresh DH"
                            );
                            let home = *self.inner.home_dc_id.lock().await;
                            let mut opts = self.inner.dc_options.lock().await;
                            if let Some(entry) = opts.get_mut(&home) {
                                entry.auth_key = None;
                                entry.first_salt = 0;
                                entry.time_offset = 0;
                            }
                            delay_ms = RECONNECT_BASE_MS;
                            continue;
                        }
                    }
                    return;
                }
                Err(e) => {
                    tracing::warn!("[ferogram::client] reconnect attempt failed: {e}");
                    // ENETUNREACHABLE (101) and ECONNABORTED (103) are phone-side
                    // network states (no route, switching towers, airplane mode).
                    // Exponential backoff is wrong here: the network can come back at
                    // any moment and we want to reconnect immediately when it does.
                    // Use a flat short retry instead of doubling the delay.
                    let is_phone_network_error = matches!(
                        &e,
                        ferogram_connect::ConnectError::Io(io_err)
                            if matches!(io_err.raw_os_error(), Some(101) | Some(103))
                    );
                    if !is_phone_network_error {
                        delay_ms = (delay_ms * 2).min(RECONNECT_MAX_SECS * 1000);
                    } else {
                        delay_ms = RECONNECT_BASE_MS;
                    }
                }
            }
        }
    }

    /// Without the sort, a container arriving as [pts=5, pts=3, pts=4] produces
    /// a false gap on the first item (expected 3, got 5) and spuriously fires
    /// getDifference even though the filling updates are present in the same batch.
    #[allow(dead_code)]
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

    /// Parse an incoming update container and route each update through the
    /// pts/qts/seq gap-checkers before forwarding to `update_tx`.
    async fn dispatch_updates(&self, body: &[u8]) {
        if body.len() < 4 {
            return;
        }
        metrics::counter!("ferogram.updates_received").increment(1);
        let cid = u32::from_le_bytes(body[..4].try_into().unwrap());

        // updatesTooLong: signal message_box of gap and run getDifference.
        if cid == 0xe317af7e_u32 {
            tracing::warn!("[ferogram::client] updatesTooLong received; triggering getDifference");
            let c = self.clone();
            tokio::spawn(async move {
                {
                    let mut mb = c.inner.message_box.lock().await;
                    let _ = mb.process_updates(message_box::UpdatesLike::ConnectionClosed);
                }
                c.inner.diff_notify.notify_one();
            });
            return;
        }

        use ferogram_tl_types::{Cursor, Deserializable};

        #[allow(dead_code)]
        struct ParsedContainer {
            seq_info: Option<(i32, i32)>,
            users: Vec<tl::enums::User>,
            chats: Vec<tl::enums::Chat>,
            updates: Vec<tl::enums::Update>,
            /// The original parsed Updates enum, preserved so message_box
            /// never needs to re-deserialize raw bytes.
            original: Option<tl::enums::Updates>,
        }

        let mut cur = Cursor::from_slice(body);
        let parsed: ParsedContainer = match cid {
            0x74ae4240 => {
                // updates#74ae4240
                match tl::enums::Updates::deserialize(&mut cur) {
                    Ok(tl::enums::Updates::Updates(u)) => {
                        let original = tl::enums::Updates::Updates(u.clone());
                        ParsedContainer {
                            seq_info: Some((u.seq, u.seq)),
                            users: u.users,
                            chats: u.chats,
                            updates: u.updates,
                            original: Some(original),
                        }
                    }
                    _ => ParsedContainer {
                        seq_info: None,
                        users: vec![],
                        chats: vec![],
                        updates: vec![],
                        original: None,
                    },
                }
            }
            0x725b04c3 => {
                // updatesCombined#725b04c3
                match tl::enums::Updates::deserialize(&mut cur) {
                    Ok(tl::enums::Updates::Combined(u)) => {
                        let original = tl::enums::Updates::Combined(u.clone());
                        ParsedContainer {
                            seq_info: Some((u.seq, u.seq_start)),
                            users: u.users,
                            chats: u.chats,
                            updates: u.updates,
                            original: Some(original),
                        }
                    }
                    _ => ParsedContainer {
                        seq_info: None,
                        users: vec![],
                        chats: vec![],
                        updates: vec![],
                        original: None,
                    },
                }
            }
            0x78d4dec1 => {
                // updateShort: no users/chats/seq
                match tl::types::UpdateShort::deserialize(&mut Cursor::from_slice(&body[4..])) {
                    Ok(u) => {
                        let original = tl::enums::Updates::UpdateShort(u.clone());
                        ParsedContainer {
                            seq_info: None,
                            users: vec![],
                            chats: vec![],
                            updates: vec![u.update],
                            original: Some(original),
                        }
                    }
                    Err(_) => ParsedContainer {
                        seq_info: None,
                        users: vec![],
                        chats: vec![],
                        updates: vec![],
                        original: None,
                    },
                }
            }
            // updateShortSentMessage (0x9015e101) as a server-pushed frame:
            // carries pts/pts_count but no user-visible content.  Without this arm
            // it falls to `_ =>` → original:None → MalformedUpdates → Err(Gap) →
            // try_begin_get_diff(Key::Common) - a spurious getDifference triggered
            // on every poll vote or sent-message confirmation.
            0x9015e101 => {
                let mut cur = Cursor::from_slice(&body[4..]);
                if let Ok(sent) = tl::types::UpdateShortSentMessage::deserialize(&mut cur) {
                    tracing::debug!(
                        "[ferogram::client] updateShortSentMessage received: pts={}, pts_count={}",
                        sent.pts,
                        sent.pts_count
                    );
                    let _ = self.inner.message_box.lock().await.process_updates(
                        message_box::UpdatesLike::AffectedMessages(
                            tl::types::messages::AffectedMessages {
                                pts: sent.pts,
                                pts_count: sent.pts_count,
                            },
                        ),
                    );
                }
                return;
            }
            _ => ParsedContainer {
                seq_info: None,
                users: vec![],
                chats: vec![],
                updates: vec![],
                original: None,
            },
        };

        // Feed users/chats into the PeerCache so access_hash lookups work.
        //
        // Build a per-user context map so each min user gets the context of the
        // specific message they appeared in. Wrong context → CHANNEL_INVALID.
        //
        // Also extract fwd_from.from_id, via_bot_id, reply_to peer, and
        // MessageService action user IDs.
        if !parsed.users.is_empty() || !parsed.chats.is_empty() {
            // user_id → (peer_id, msg_id): built from every message in the batch.
            let mut user_ctx: HashMap<i64, (i64, i32)> = HashMap::new();
            // Channel IDs from saved_from_peer: register as min-channels (no hash yet).
            let mut channel_seen: std::collections::HashSet<i64> = std::collections::HashSet::new();

            // Helper: extract the inner tl::enums::Message from any message-bearing update.
            // NewMessage and EditMessage hold different struct types, so we can't use |
            // in a single arm  - extract via separate match arms returning &tl::enums::Message.
            // Named fn instead of closure so lifetime elision ties output to input correctly.
            fn get_message(upd: &tl::enums::Update) -> Option<&tl::enums::Message> {
                match upd {
                    tl::enums::Update::NewMessage(m) => Some(&m.message),
                    tl::enums::Update::EditMessage(m) => Some(&m.message),
                    tl::enums::Update::NewChannelMessage(m) => Some(&m.message),
                    tl::enums::Update::EditChannelMessage(m) => Some(&m.message),
                    _ => None,
                }
            }

            let extract_peer_id = |peer: &tl::enums::Peer| -> i64 {
                match peer {
                    tl::enums::Peer::Channel(c) => c.channel_id,
                    tl::enums::Peer::Chat(c) => c.chat_id,
                    tl::enums::Peer::User(u) => u.user_id,
                }
            };

            for upd in &parsed.updates {
                if let Some(envelope) = get_message(upd) {
                    if let tl::enums::Message::Message(msg) = envelope {
                        let ctx_peer = extract_peer_id(&msg.peer_id);
                        let ctx = (ctx_peer, msg.id);

                        // Primary sender.
                        if let Some(tl::enums::Peer::User(u)) = &msg.from_id {
                            user_ctx.insert(u.user_id, ctx);
                        }
                        // fwd_from  - destructure MessageFwdHeader before extracting user.
                        if let Some(tl::enums::MessageFwdHeader::MessageFwdHeader(fwd)) =
                            &msg.fwd_from
                        {
                            if let Some(tl::enums::Peer::User(u)) = &fwd.from_id {
                                user_ctx.entry(u.user_id).or_insert(ctx);
                            }
                            if let Some(tl::enums::Peer::User(u)) = &fwd.saved_from_peer {
                                user_ctx.entry(u.user_id).or_insert(ctx);
                            }
                            // saved_from_peer channel may not appear in chats[]; register as min.
                            if let Some(tl::enums::Peer::Channel(c)) = &fwd.saved_from_peer {
                                channel_seen.insert(c.channel_id);
                            }
                        }
                        // Inline bot.
                        if let Some(bot_id) = msg.via_bot_id {
                            user_ctx.entry(bot_id).or_insert(ctx);
                        }
                        // reply_to: extract user peer from reply header.
                        if let Some(tl::enums::MessageReplyHeader::MessageReplyHeader(h)) =
                            &msg.reply_to
                            && let Some(tl::enums::Peer::User(u)) = &h.reply_to_peer_id
                        {
                            user_ctx.entry(u.user_id).or_insert(ctx);
                        }
                    }

                    // Service message: extract sender and action user IDs.
                    if let tl::enums::Message::Service(svc) = envelope {
                        let ctx_peer = extract_peer_id(&svc.peer_id);
                        let ctx = (ctx_peer, svc.id);

                        // Service message sender (admin performing the action).
                        if let Some(tl::enums::Peer::User(u)) = &svc.from_id {
                            user_ctx.entry(u.user_id).or_insert(ctx);
                        }
                        // Action-specific user IDs. These users appear in batch users[]
                        // with full data; we register ctx so cache_user_with_context
                        // assigns the correct message context for any min users.
                        match &svc.action {
                            tl::enums::MessageAction::ChatAddUser(a) => {
                                for &uid in &a.users {
                                    user_ctx.entry(uid).or_insert(ctx);
                                }
                            }
                            tl::enums::MessageAction::ChatCreate(a) => {
                                for &uid in &a.users {
                                    user_ctx.entry(uid).or_insert(ctx);
                                }
                            }
                            tl::enums::MessageAction::ChatDeleteUser(a) => {
                                user_ctx.entry(a.user_id).or_insert(ctx);
                            }
                            tl::enums::MessageAction::ChatJoinedByLink(a) => {
                                user_ctx.entry(a.inviter_id).or_insert(ctx);
                            }
                            tl::enums::MessageAction::InviteToGroupCall(a) => {
                                for &uid in &a.users {
                                    user_ctx.entry(uid).or_insert(ctx);
                                }
                            }
                            tl::enums::MessageAction::GeoProximityReached(a) => {
                                if let tl::enums::Peer::User(u) = &a.from_id {
                                    user_ctx.entry(u.user_id).or_insert(ctx);
                                }
                                if let tl::enums::Peer::User(u) = &a.to_id {
                                    user_ctx.entry(u.user_id).or_insert(ctx);
                                }
                            }
                            tl::enums::MessageAction::RequestedPeer(a) => {
                                for peer in &a.peers {
                                    if let tl::enums::Peer::User(u) = peer {
                                        user_ctx.entry(u.user_id).or_insert(ctx);
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                }
            }

            let mut cache: tokio::sync::RwLockWriteGuard<'_, PeerCache> =
                self.inner.peer_cache.write().await;
            for u in &parsed.users {
                if let tl::enums::User::User(uu) = u {
                    if let Some(&(peer_id, msg_id)) = user_ctx.get(&uu.id) {
                        cache.cache_user_with_context(u, peer_id, msg_id);
                    } else {
                        cache.cache_user(u);
                    }
                }
            }
            for c in &parsed.chats {
                cache.cache_chat(c);
            }
            // Register channel IDs seen in saved_from_peer that were not in chats[].
            for ch_id in channel_seen {
                if !cache.channels.contains_key(&ch_id) && !cache.channels_min.contains(&ch_id) {
                    cache.channels_min.insert(ch_id);
                }
            }
            drop(cache);
            self.mark_session_snapshot_dirty();
        }

        // Use the already-parsed Updates object; never re-deserialize raw bytes.
        {
            let mb_input = match parsed.original {
                Some(u) => message_box::UpdatesLike::Updates(Box::new(u)),
                None => message_box::UpdatesLike::MalformedUpdates,
            };

            let mb_result = self
                .inner
                .message_box
                .lock()
                .await
                .process_updates(mb_input);

            // Wake the diff task unconditionally. process_updates() may have
            // shrunk MessageBoxes::next_deadline (a possible-gap was buffered
            // for some entry, or try_begin_get_diff() populated
            // getting_diff_for for a hard gap / ChannelTooLong), but the diff
            // task's select! loop is parked inside a `sleep_until` future that
            // was constructed from the *previous* (often much later) deadline
            // - it has no way to notice the new, sooner deadline on its own.
            // Without this notify, gaps only ever get resolved once the old
            // stale deadline naturally elapses (up to NO_UPDATES_TIMEOUT =
            // 15 minutes), which is why channel/common pts gaps could pile up
            // indefinitely and updates would stop flowing after the first
            // message. notify_one() is cheap even when nothing is pending:
            // run_pending_differences() just finds no diff request and
            // returns immediately.
            self.inner.diff_notify.notify_one();

            match mb_result {
                Ok((raw_updates, mb_users, mb_chats)) => {
                    // Cache any users/chats returned from getDifference-style paths.
                    if !mb_users.is_empty() || !mb_chats.is_empty() {
                        let mut cache: tokio::sync::RwLockWriteGuard<'_, PeerCache> =
                            self.inner.peer_cache.write().await;
                        for u in &mb_users {
                            cache.cache_user(u);
                        }
                        for c in &mb_chats {
                            cache.cache_chat(c);
                        }
                        drop(cache);
                        self.mark_session_snapshot_dirty();
                    }
                    // Convert and emit each approved update.
                    let peer_map = crate::peer_cache::build_peer_map(&parsed.chats);
                    for raw in raw_updates {
                        let highs = update::from_single_update_with_peers(raw, peer_map.clone());
                        for u in highs {
                            let u = attach_client_to_update(u, self);
                            if self.inner.update_tx.try_send(u).is_err() {
                                tracing::warn!(
                                    "[ferogram::client] update channel is full; dropping update (backpressure)"
                                );
                                metrics::counter!("ferogram.updates_dropped").increment(1);
                            }
                        }
                    }
                }
                Err(_gap) => {
                    // Gap detected; deadline loop (Step 5) will fire getDifference.
                    tracing::debug!(
                        "[ferogram::msgbox] gap detected in updates container; getDifference will be triggered on next deadline loop"
                    );
                }
            }
        }
    }

    /// Persist a single update-state change to the session backend.
    ///
    /// Dispatched via `spawn_blocking`; see [`Client::save_session`] for why.
    /// Called only from `sync_pts_state`/`force_sync_pts_state` (login,
    /// reconnect, gap recovery), not per-update, so this is a low-frequency
    /// path, but kept consistent with the rest of the persistence layer.
    fn persist_state(&self, change: ferogram_session::UpdateStateChange) {
        let backend = Arc::clone(&self.inner.session_backend);
        tokio::task::spawn_blocking(move || {
            if let Err(e) = backend.apply_update_state(change) {
                tracing::warn!("[ferogram::persist] state write failed: {e}");
            }
        });
    }

    /// Fetch the current server update state and initialise `MessageBoxes` if empty.
    ///
    /// Called after login / reconnect to anchor the pts counter so the first
    /// `getDifference` starts from the right position.
    pub(crate) async fn sync_pts_state(&self) -> Result<(), InvocationError> {
        use crate::util::decode_checked;
        let body = self
            .rpc_call_raw(&tl::functions::updates::GetState {})
            .await?;
        let tl::enums::updates::State::State(s) =
            decode_checked::<tl::enums::updates::State>("updates.getState", &body)?;

        {
            let mut mb = self.inner.message_box.lock().await;
            if mb.is_empty() {
                mb.set_state(s.clone());
            }
        }

        let (pts, qts, date, seq) = (s.pts, s.qts, s.date, s.seq);
        tracing::debug!(
            "[ferogram::client] pts state synced: pts={}, qts={}, seq={}",
            pts,
            qts,
            seq
        );
        self.persist_state(ferogram_session::UpdateStateChange::Primary { pts, date, seq });
        self.persist_state(ferogram_session::UpdateStateChange::Secondary { qts });
        Ok(())
    }

    /// Force-resync pts/qts from the server into the live message_box, regardless of
    /// whether the message_box is empty.
    ///
    /// Unlike [`sync_pts_state`] (which skips the in-memory update when the box is not
    /// empty), this method always calls [`MessageBoxes::force_reset_common_pts`] so that a
    /// stale pts caused by an unknown TL constructor does not re-trigger getDifference.
    async fn force_sync_pts_state(&self) -> Result<(), InvocationError> {
        use crate::util::decode_checked;
        let body = self
            .rpc_call_raw(&tl::functions::updates::GetState {})
            .await?;
        let tl::enums::updates::State::State(s) =
            decode_checked::<tl::enums::updates::State>("updates.getState", &body)?;
        {
            let mut mb = self.inner.message_box.lock().await;
            mb.force_reset_common_pts(s.pts, s.qts, s.date, s.seq);
        }
        let (pts, qts, date, seq) = (s.pts, s.qts, s.date, s.seq);
        tracing::debug!(
            "[ferogram::client] pts state force-synced: pts={}, qts={}, seq={}",
            pts,
            qts,
            seq
        );
        self.persist_state(ferogram_session::UpdateStateChange::Primary { pts, date, seq });
        self.persist_state(ferogram_session::UpdateStateChange::Secondary { qts });
        Ok(())
    }

    async fn run_pending_differences(&self) {
        use crate::message_box::PrematureEndReason;

        // No global diff-in-flight gate: message_box's own getting_diff_for and
        // channel_diff_in_flight fields serialise concurrent diff tasks.
        // Multiple spawned tasks are safe because message_box
        // will return None from get_difference/get_channel_difference when a diff
        // is already in flight for that channel/session.

        loop {
            let get_diff_req = self.inner.message_box.lock().await.get_difference();
            if let Some(req) = get_diff_req {
                tracing::debug!("[ferogram::client] running getDifference");
                let body = self.rpc_call_raw(&req).await;
                match body {
                    Ok(raw) => {
                        let diff = {
                            use ferogram_tl_types::{Cursor, Deserializable};
                            let mut cur = Cursor::from_slice(&raw);
                            tl::enums::updates::Difference::deserialize(&mut cur)
                        };
                        match diff {
                            Ok(diff) => {
                                let (updates, users, chats) =
                                    self.inner.message_box.lock().await.apply_difference(diff);
                                {
                                    let mut cache: tokio::sync::RwLockWriteGuard<'_, PeerCache> =
                                        self.inner.peer_cache.write().await;
                                    for u in &users {
                                        cache.cache_user(u);
                                    }
                                    for c in &chats {
                                        cache.cache_chat(c);
                                    }
                                    drop(cache);
                                    self.mark_session_snapshot_dirty();
                                }
                                self.emit_raw_updates(updates).await;
                            }
                            Err(e) => {
                                let preview = &raw[..raw.len().min(16)];
                                tracing::warn!(
                                    "[ferogram::client] getDifference response parse failed: {e} (layer too new?); pts advanced to avoid loop; preview: {preview:?}"
                                );
                                self.inner.message_box.lock().await.abort_difference();
                                // Force-advance pts to the server's current value so the
                                // stale gap does not immediately re-trigger getDifference
                                // (infinite loop when the server sends an unknown constructor
                                // from a newer TL layer).
                                match self.force_sync_pts_state().await {
                                    Ok(()) => {
                                        tracing::debug!(
                                            "[ferogram::client] pts re-synced after getDifference parse failure; resuming channel diffs"
                                        );
                                    }
                                    Err(_sync_err) => {
                                        tracing::warn!(
                                            "[ferogram::client] force_sync_pts_state failed after getDifference parse failure"
                                        );
                                        tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                                    }
                                }
                                // Use `continue` (not `break`) so pending channel diffs
                                // still get a chance to run in the same iteration.
                                continue;
                            }
                        }
                    }
                    Err(e) => {
                        tracing::warn!("[ferogram::client] getDifference RPC failed: {e}");
                        self.inner.message_box.lock().await.abort_difference();
                        // IO/transport error or 401 auth failure means the connection is
                        // dead or the session was invalidated. Break out so the reconnect
                        // path can notify diff_notify and start a fresh diff task once
                        // the connection (and auth key) is back.
                        let is_auth_error = matches!(&e, InvocationError::Rpc(r) if r.code == 401);
                        if matches!(&e, InvocationError::Io(_)) || is_auth_error {
                            break;
                        }
                        tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                        continue; // let channel diffs run even when common diff RPC fails (server-side error)
                    }
                }
                continue;
            }

            let get_chan_req = self.inner.message_box.lock().await.get_channel_difference();
            if let Some((channel_id, mut req)) = get_chan_req {
                let access_hash = {
                    let cache: tokio::sync::RwLockReadGuard<'_, PeerCache> =
                        self.inner.peer_cache.read().await;
                    cache
                        .channels
                        .get(&channel_id)
                        .map(|&(h, _)| h)
                        .unwrap_or(0)
                };
                if access_hash == 0 {
                    let auto_resolve = {
                        let cache = self.inner.peer_cache.read().await;
                        cache.experimental.auto_resolve_peers
                    };

                    self.record_peer_cache_miss();

                    if auto_resolve {
                        let peer = tl::enums::Peer::Channel(tl::types::PeerChannel { channel_id });
                        match self.fetch_by_id_rpc(peer).await {
                            Ok(_) => {
                                let h = self
                                    .inner
                                    .peer_cache
                                    .read()
                                    .await
                                    .channels
                                    .get(&channel_id)
                                    .map(|&(hash, _)| hash)
                                    .unwrap_or(0);
                                if h != 0 {
                                    req.channel = tl::enums::InputChannel::InputChannel(
                                        tl::types::InputChannel {
                                            channel_id,
                                            access_hash: h,
                                        },
                                    );
                                    // hash now set; fall through to run the diff
                                } else {
                                    tracing::debug!(
                                        "[ferogram::client] auto_resolve: channel {channel_id} still has no access_hash after fetch; deferring diff"
                                    );
                                    self.inner.message_box.lock().await.end_channel_difference(
                                        PrematureEndReason::TemporaryServerIssues,
                                    );
                                    continue;
                                }
                            }
                            Err(ref e) if e.is("CHANNEL_PRIVATE") => {
                                tracing::info!(
                                    "[ferogram::client] auto_resolve: channel {channel_id} is CHANNEL_PRIVATE; deferring diff"
                                );
                                self.inner.message_box.lock().await.end_channel_difference(
                                    PrematureEndReason::TemporaryServerIssues,
                                );
                                continue;
                            }
                            Err(e) => {
                                tracing::warn!(
                                    "[ferogram::client] auto_resolve: channel {channel_id} fetch failed ({e}); deferring diff"
                                );
                                self.inner.message_box.lock().await.end_channel_difference(
                                    PrematureEndReason::TemporaryServerIssues,
                                );
                                continue;
                            }
                        }
                    } else {
                        tracing::debug!(
                            "[ferogram::client] no access_hash cached for channel {channel_id}; deferring getDiff"
                        );
                        self.inner
                            .message_box
                            .lock()
                            .await
                            .end_channel_difference(PrematureEndReason::TemporaryServerIssues);
                        continue;
                    }
                }
                req.channel = tl::enums::InputChannel::InputChannel(tl::types::InputChannel {
                    channel_id,
                    access_hash,
                });
                req.limit = if self.inner.is_bot.load(std::sync::atomic::Ordering::Relaxed) {
                    100_000
                } else {
                    100
                };
                tracing::debug!(
                    "[ferogram::client] running getChannelDifference for channel {channel_id}"
                );
                match self.rpc_call_raw(&req).await {
                    Ok(raw) => {
                        use ferogram_tl_types::{Cursor, Deserializable};
                        let mut cur = Cursor::from_slice(&raw);
                        match tl::enums::updates::ChannelDifference::deserialize(&mut cur) {
                            Ok(diff) => {
                                let (updates, users, chats) = self
                                    .inner
                                    .message_box
                                    .lock()
                                    .await
                                    .apply_channel_difference(diff);
                                {
                                    let mut cache: tokio::sync::RwLockWriteGuard<'_, PeerCache> =
                                        self.inner.peer_cache.write().await;
                                    for u in &users {
                                        cache.cache_user(u);
                                    }
                                    for c in &chats {
                                        cache.cache_chat(c);
                                    }
                                    drop(cache);
                                    self.mark_session_snapshot_dirty();
                                }
                                self.emit_raw_updates(updates).await;
                            }
                            Err(e) => {
                                let preview = &raw[..raw.len().min(16)];
                                tracing::warn!(
                                    "[ferogram::client] getChannelDifference response parse failed: {e} (layer too new?); skipping; preview: {preview:?}"
                                );
                                // Drop the channel entry entirely so the stale pts does not
                                // re-trigger getChannelDifference on the next update (same
                                // infinite-loop fix as for Common getDifference).  The entry
                                // will be re-created when the next update for this channel
                                // arrives or when GetDialogs runs.
                                self.inner
                                    .message_box
                                    .lock()
                                    .await
                                    .end_channel_difference(PrematureEndReason::Banned);
                                tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                            }
                        }
                    }
                    Err(ref e) if e.is("PERSISTENT_TIMESTAMP_OUTDATED") => {
                        tracing::warn!(
                            "[ferogram::client] getChannelDifference: PERSISTENT_TIMESTAMP_OUTDATED; resetting channel pts"
                        );
                        self.inner
                            .message_box
                            .lock()
                            .await
                            .end_channel_difference(PrematureEndReason::TemporaryServerIssues);
                    }
                    Err(ref e) if e.is("PERSISTENT_TIMESTAMP_INVALID") => {
                        // pts is stale or was never set; Telegram rejects the diff request.
                        // Reset the channel state and let the next update trigger a fresh diff.
                        tracing::warn!(
                            "[ferogram::client] getChannelDifference: PERSISTENT_TIMESTAMP_INVALID for channel {channel_id}; resetting pts"
                        );
                        self.inner
                            .message_box
                            .lock()
                            .await
                            .end_channel_difference(PrematureEndReason::TemporaryServerIssues);
                    }
                    Err(ref e) if e.is("CHANNEL_PRIVATE") => {
                        tracing::info!(
                            "[ferogram::client] getChannelDifference: channel {channel_id} is now private; dropping from diff queue"
                        );
                        self.inner
                            .message_box
                            .lock()
                            .await
                            .end_channel_difference(PrematureEndReason::Banned);
                    }
                    Err(InvocationError::Rpc(ref rpc)) if rpc.code == 500 => {
                        tracing::warn!(
                            "[ferogram::client] getChannelDifference: server returned 500; will retry after backoff"
                        );
                        self.inner
                            .message_box
                            .lock()
                            .await
                            .end_channel_difference(PrematureEndReason::TemporaryServerIssues);
                    }
                    Err(e) => {
                        tracing::warn!(
                            "[ferogram::client] getChannelDifference for channel {channel_id} failed: {e}"
                        );
                        self.inner
                            .message_box
                            .lock()
                            .await
                            .end_channel_difference(PrematureEndReason::TemporaryServerIssues);
                        // Same as getDifference: IO error or 401 auth failure means
                        // dead connection / invalidated session; break so reconnect can
                        // notify diff_notify and retry with a fresh auth key.
                        let is_auth_error = matches!(&e, InvocationError::Rpc(r) if r.code == 401);
                        if matches!(&e, InvocationError::Io(_)) || is_auth_error {
                            break;
                        }
                    }
                }
                continue;
            }

            // Nothing pending.
            break;
        }
    }

    /// Convert raw `tl::enums::Update` list → high-level updates and send to `update_tx`.
    async fn emit_raw_updates(&self, updates: Vec<tl::enums::Update>) {
        for raw in updates {
            // No per-batch peer map is available from getDifference paths; the
            // peer cache (populated during dispatch) handles kind lookups.
            let highs = update::from_single_update(raw);
            for u in highs {
                let u = attach_client_to_update(u, self);
                if self.inner.update_tx.try_send(u).is_err() {
                    tracing::warn!(
                        "[ferogram::client] update channel is full; dropping update (backpressure)"
                    );
                    metrics::counter!("ferogram.updates_dropped").increment(1);
                }
            }
        }
    }

    /// Invoke any TL function directly, handling flood-wait retries.
    pub async fn invoke<R: RemoteCall>(&self, req: &R) -> Result<R::Return, InvocationError> {
        let body: Vec<u8> = self.rpc_call_raw(req).await?;
        <R::Return as tl::Deserializable>::from_bytes_exact(&body).map_err(Into::into)
    }

    pub(crate) async fn rpc_call_raw<R: RemoteCall>(
        &self,
        req: &R,
    ) -> Result<Vec<u8>, InvocationError> {
        let mut rl = RetryLoop::new(Arc::clone(&self.inner.retry_policy));
        loop {
            match self.do_rpc_call(req).await {
                Ok(body) => {
                    metrics::counter!("ferogram.rpc_calls_total", "result" => "ok").increment(1);
                    self.feed_own_updates(&body).await;
                    return Ok(body);
                }
                Err(e) if e.migrate_dc_id().is_some() => {
                    // Telegram is redirecting us to a different DC.
                    // Migrate transparently and retry: no error surfaces to caller.
                    self.migrate_to(e.migrate_dc_id().expect("matched Migrate variant"))
                        .await?;
                }
                // AUTH_KEY_UNREGISTERED (401): propagate immediately.
                // The reader loop does NOT trigger fresh DH on RPC-level 401 errors -
                // only on TCP disconnects (-404 / UnexpectedEof).  Retrying here was
                // pointless: it just delayed the error by 1-3 s and caused it to leak
                // as an I/O error, preventing callers like is_authorized() from ever
                // seeing the real 401 and returning Ok(false).
                Err(InvocationError::Rpc(ref r)) if r.code == 401 => {
                    metrics::counter!("ferogram.rpc_calls_total", "result" => "error").increment(1);
                    return Err(InvocationError::Rpc(r.clone()));
                }
                // Stale access hash: evict the bad entry from cache and surface
                // StaleHash so callers can retry with a fresh resolution.
                Err(InvocationError::Rpc(ref r))
                    if matches!(
                        r.name.as_str(),
                        "PEER_ID_INVALID"
                            | "CHANNEL_INVALID"
                            | "USER_ID_INVALID"
                            | "CHANNEL_PRIVATE"
                            | "INPUT_USER_DEACTIVATED"
                    ) =>
                {
                    metrics::counter!("ferogram.rpc_calls_total", "result" => "stale_hash")
                        .increment(1);
                    tracing::debug!(
                        "[ferogram::client] rpc_call_raw: {} triggered FILE_MIGRATE or PEER_INVALID; evicting stale peer from cache",
                        r.name
                    );
                    return Err(InvocationError::StaleHash);
                }
                Err(e) => {
                    metrics::counter!("ferogram.rpc_calls_total", "result" => "error").increment(1);
                    rl.advance(e).await?;
                }
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
        let raw_body = req.to_bytes();
        let body = maybe_gz_pack(&raw_body);
        let (tx, rx) = oneshot::channel();
        self.inner
            .rpc_tx
            .send(ferogram_mtsender::RpcEnqueue { body, tx })
            .await
            .map_err(|_| InvocationError::Deserialize("sender task shut down".into()))?;
        match rx.await {
            Ok(result) => result,
            Err(_) => Err(InvocationError::Deserialize(
                "RPC channel closed (sender task died?)".into(),
            )),
        }
    }

    /// Like `rpc_call_raw` but for write RPCs (Serializable, return type is Updates).
    /// Uses the same oneshot mechanism: the reader task signals success/failure.
    pub(crate) async fn rpc_write<S: tl::Serializable>(
        &self,
        req: &S,
    ) -> Result<(), InvocationError> {
        let mut fail_count = NonZeroU32::new(1).expect("1 is nonzero");
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
        let raw_body = req.to_bytes();
        let body = maybe_gz_pack(&raw_body);
        let (tx, rx) = oneshot::channel();
        self.inner
            .rpc_tx
            .send(ferogram_mtsender::RpcEnqueue { body, tx })
            .await
            .map_err(|_| InvocationError::Deserialize("sender task shut down".into()))?;
        match rx.await {
            Ok(Ok(body)) => {
                self.feed_own_updates(&body).await;
                Ok(())
            }
            Ok(Err(e)) => Err(e),
            Err(_) => Err(InvocationError::Deserialize(
                "rpc_write channel closed (sender task died?)".into(),
            )),
        }
    }

    /// Feed the raw body of one of our own RPC responses into `MessageBoxes`
    /// so self-sent actions (sending/editing/deleting a message, etc.)
    /// advance local pts the same way a pushed update would. Otherwise the
    /// next real update looks like a gap and triggers a spurious getDifference.
    ///
    /// Safe to call on any response: every TL type is prefixed with a
    /// 4-byte constructor ID, and `Updates::deserialize` checks it before
    /// matching a variant. A response that's neither `Updates` nor
    /// `messages.AffectedMessages` just fails to match and this is a no-op.
    async fn feed_own_updates(&self, body: &[u8]) {
        use ferogram_tl_types::Identifiable;

        if body.len() < 4 {
            return;
        }

        // Most write RPCs return `Updates` (sendMessage, editMessage, forwardMessages, leaveChannel, ...).
        let mut cur = Cursor::from_slice(body);
        if let Ok(updates) = tl::enums::Updates::deserialize(&mut cur) {
            let _ = self
                .inner
                .message_box
                .lock()
                .await
                .process_updates(message_box::UpdatesLike::Updates(Box::new(updates)));
            return;
        }

        // A few (deleteMessages, readHistory, ...) return bare messages.AffectedMessages
        // instead. It's not an enum, so deserialize() won't check the ctor id for us.
        let id = u32::from_le_bytes([body[0], body[1], body[2], body[3]]);
        if id == <tl::types::messages::AffectedMessages as Identifiable>::CONSTRUCTOR_ID {
            let mut cur = Cursor::from_slice(&body[4..]);
            if let Ok(affected) = tl::types::messages::AffectedMessages::deserialize(&mut cur) {
                let _ = self
                    .inner
                    .message_box
                    .lock()
                    .await
                    .process_updates(message_box::UpdatesLike::AffectedMessages(affected));
            }
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
        let body: Vec<u8> = self.rpc_call_raw_serializable(&req).await?;

        let mut cur = Cursor::from_slice(&body);
        if let Ok(tl::enums::Config::Config(cfg)) = tl::enums::Config::deserialize(&mut cur) {
            let allow_ipv6 = self.inner.allow_ipv6;
            let mut opts: tokio::sync::MutexGuard<'_, std::collections::HashMap<i32, DcEntry>> =
                self.inner.dc_options.lock().await;
            let mut media_opts: tokio::sync::MutexGuard<
                '_,
                std::collections::HashMap<i32, DcEntry>,
            > = self.inner.media_dc_options.lock().await;
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
                "[ferogram::client] initConnection complete ({} DCs registered, ipv6={})",
                cfg.dc_options.len(),
                allow_ipv6
            );
        }
        Ok(())
    }

    async fn migrate_to(&self, new_dc_id: i32) -> Result<(), InvocationError> {
        let addr = {
            let opts: tokio::sync::MutexGuard<'_, std::collections::HashMap<i32, DcEntry>> =
                self.inner.dc_options.lock().await;
            opts.get(&new_dc_id)
                .map(|e| e.addr.clone())
                .unwrap_or_else(|| fallback_dc_addr(new_dc_id).to_string())
        };
        tracing::info!("[ferogram::client] migrating account to DC{new_dc_id} at {addr}");

        let saved_key = {
            let opts: tokio::sync::MutexGuard<'_, std::collections::HashMap<i32, DcEntry>> =
                self.inner.dc_options.lock().await;
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
                mtproxy.as_ref(),
                &transport,
                new_dc_id as i16,
                self.inner.pfs_enabled,
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
            let mut opts: tokio::sync::MutexGuard<'_, std::collections::HashMap<i32, DcEntry>> =
                self.inner.dc_options.lock().await;
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

        *self.inner.home_dc_id.lock().await = new_dc_id;

        // Hand the new connection to the sender task. It will send a
        // FrameEvent::Connected once the swap completes, which updates
        // dc_options[home_dc] (now new_dc_id) with the fresh salt/time_offset.
        let perm_auth_key = conn.perm_auth_key;
        let _ = self
            .inner
            .reconnect_tx
            .send(ferogram_mtsender::ReconnectRequest {
                stream: conn.stream,
                enc: conn.enc,
                frame_kind: conn.frame_kind,
                perm_auth_key,
            })
            .await;

        // migrate_to() is called from user-facing methods (bot_sign_in,
        // request_login_code, sign_in): NOT from inside the sender task.
        // The sender task runs concurrently, so awaiting init_connection() here
        // is safe: it can route the RPC response while we wait. We must await
        // before returning so the caller can safely retry the original request
        // on the new DC.
        //
        // Respect FLOOD_WAIT: if Telegram rate-limits init, wait and retry
        // rather than returning an error that would abort the whole auth flow.
        loop {
            match self.init_connection().await {
                Ok(()) => break,
                Err(InvocationError::Rpc(ref r)) if r.flood_wait_seconds().is_some() => {
                    let secs = r.flood_wait_seconds().expect("is FLOOD_WAIT error");
                    tracing::warn!(
                        "[ferogram::client] migration to DC{new_dc_id}: FLOOD_WAIT_{secs} during initConnection; waiting"
                    );
                    sleep(Duration::from_secs(secs + 1)).await;
                }
                Err(e) => return Err(e),
            }
        }

        self.save_session().await.ok();
        tracing::info!("[ferogram::client] migration complete; now connected to DC{new_dc_id}");
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

    /// Mark the session snapshot as dirty so the periodic full-snapshot saver
    /// knows a flush is needed on the next interval tick.
    #[inline]
    fn mark_session_snapshot_dirty(&self) {
        mark_session_snapshot_dirty_impl(&self.inner);
    }

    /// Cache a `User` object's ID, access hash, and display info.
    ///
    /// Call this after obtaining a `User` from a raw RPC response the client
    /// doesn't already track, so later calls (e.g. sending a message) can
    /// resolve the peer without a redundant `users.getUsers`.
    pub async fn cache_user(&self, user: &tl::enums::User) {
        cache_user_impl(&self.inner, user).await;
    }

    pub(crate) async fn cache_users_slice(&self, users: &[tl::enums::User]) {
        cache_users_slice_impl(&self.inner, users).await;
    }

    pub(crate) async fn cache_chats_slice(&self, chats: &[tl::enums::Chat]) {
        cache_chats_slice_impl(&self.inner, chats).await;
    }

    /// Cache users and chats in a single write-lock acquisition.
    pub(crate) async fn cache_users_and_chats(
        &self,
        users: &[tl::enums::User],
        chats: &[tl::enums::Chat],
    ) {
        cache_users_and_chats_impl(&self.inner, users, chats).await;
    }

    /// Feed `users`/`chats` from a raw RPC response into the peer cache.
    ///
    /// Every built-in method that gets `users`/`chats` back from Telegram
    /// (`get_chat_full`, `get_message_history`, etc.) already calls this
    /// internally. It's exposed publicly so that when you issue a raw RPC
    /// call by hand for something not yet covered by the client's API
    /// surface, you can still populate the cache exactly like the library
    /// does - instead of losing access hashes and having to re-resolve
    /// peers yourself, or making an extra `users.getUsers`/`getChats` call.
    ///
    /// Equivalent to [`ClientInternal::cache_users_and_chats`] - same
    /// underlying cache, different entry point. Both call the same
    /// implementation, so there's no drift between them.
    ///
    /// ```rust,no_run
    /// # use ferogram::{Client, tl};
    /// # async fn example(client: Client, users: Vec<tl::enums::User>, chats: Vec<tl::enums::Chat>) {
    /// client.cache_entities(&users, &chats).await;
    /// # }
    /// ```
    pub async fn cache_entities(&self, users: &[tl::enums::User], chats: &[tl::enums::Chat]) {
        self.cache_users_and_chats(users, chats).await;
    }

    /// Warm the peer cache with access_hashes by fetching the first page of
    /// dialogs (`messages.getDialogs`).
    ///
    /// # When to use
    ///
    /// This is an **explicit, opt-in cache-warming call**.  It is **not** called
    /// automatically during startup.  Access hashes are resolved lazily:
    ///
    /// * Channels already seen in a previous session have their hash restored
    ///   from the persisted `peers` list in the session file.
    /// * New channels receive their hash from the entities embedded in incoming
    ///   updates, `getDifference`, or `getChannelDifference` responses.
    ///
    /// Call this only when you know that a channel the client needs to interact
    /// with has never appeared in an update (e.g. the very first `send_message`
    /// to a channel before any update has been received).
    ///
    /// # Why it is not called at startup
    ///
    /// `messages.getDialogs` forces full deserialization of
    /// `Dialog / DraftMessage / PollResults / PeerNotifySettings / Story`.
    /// These are high-churn Telegram objects that change silently across beta
    /// layers, causing spurious parse failures that break startup even when the
    /// bot would otherwise work perfectly.  Removing this from the startup path
    /// makes Ferogram resilient to Telegram schema drift - exactly the strategy
    const MISS_THRESHOLD: u32 = 10;
    const MISS_WINDOW: std::time::Duration = std::time::Duration::from_secs(30);
    const BULK_HYDRATION_COOLDOWN: std::time::Duration = std::time::Duration::from_secs(15 * 60);

    fn record_peer_cache_miss(&self) {
        use std::sync::atomic::Ordering;
        let now = std::time::Instant::now();
        let mut window_start = self.inner.peer_cache_miss_window_start.lock();
        if now.duration_since(*window_start) > Self::MISS_WINDOW {
            *window_start = now;
            self.inner.peer_cache_miss_count.store(1, Ordering::Relaxed);
        } else {
            let prev = self
                .inner
                .peer_cache_miss_count
                .fetch_add(1, Ordering::Relaxed);
            if prev + 1 >= Self::MISS_THRESHOLD {
                self.inner.peer_cache_miss_count.store(0, Ordering::Relaxed);
                drop(window_start);
                let client = self.clone();
                tokio::spawn(async move {
                    client.bulk_peer_hydration().await;
                });
            }
        }
    }

    async fn bulk_peer_hydration(&self) {
        {
            let last = self.inner.last_bulk_hydration.lock();
            if let Some(t) = *last
                && t.elapsed() < Self::BULK_HYDRATION_COOLDOWN
            {
                tracing::debug!("[ferogram::client] bulk peer hydration skipped (cooldown active)");
                return;
            }
        }
        *self.inner.last_bulk_hydration.lock() = Some(std::time::Instant::now());
        tracing::info!(
            "[ferogram::client] peer cache miss burst detected; loading dialogs to hydrate peer cache"
        );
        if let Err(e) = self.warm_peer_cache_from_dialogs().await {
            tracing::warn!("[ferogram::client] bulk peer hydration: getDialogs failed: {e}");
        }
    }

    /// Look up the cached [`crate::ChannelKind`] for a raw channel ID.
    ///
    /// Returns `None` if the channel is not in the peer cache yet. The cache is
    /// populated automatically as updates arrive, or you can warm it explicitly
    /// with [`warm_peer_cache_from_dialogs`].
    ///
    /// [`warm_peer_cache_from_dialogs`]: Client::warm_peer_cache_from_dialogs
    pub async fn channel_kind_of(&self, channel_id: i64) -> Option<crate::types::ChannelKind> {
        self.inner
            .peer_cache
            .read()
            .await
            .channel_kind_of(channel_id)
    }

    /// Fetch the first page of dialogs purely to populate the peer cache
    /// with their users/chats. Called automatically on a cache-miss burst
    /// (see `bulk_peer_hydration`); exposed publicly so callers can
    /// also warm it up-front instead of waiting for the first miss.
    pub async fn warm_peer_cache_from_dialogs(&self) -> Result<(), InvocationError> {
        let req = tl::functions::messages::GetDialogs {
            exclude_pinned: false,
            folder_id: None,
            offset_date: 0,
            offset_id: 0,
            offset_peer: tl::enums::InputPeer::Empty,
            limit: 100,
            hash: 0,
        };
        let body: Vec<u8> = self.rpc_call_raw(&req).await?;
        tracing::debug!(
            "[ferogram::client] warm_peer_cache: response body len={}, ctor=0x{:08x}",
            body.len(),
            if body.len() >= 4 {
                u32::from_le_bytes([body[0], body[1], body[2], body[3]])
            } else {
                0
            },
        );
        let mut cur = Cursor::from_slice(&body);
        let de =
            tl::enums::messages::Dialogs::deserialize(&mut cur).map_err(InvocationError::from)?;
        match de {
            tl::enums::messages::Dialogs::Dialogs(d) => {
                tracing::debug!(
                    "[ferogram::client] warm_peer_cache: loaded {} dialogs, {} users, {} chats",
                    d.dialogs.len(),
                    d.users.len(),
                    d.chats.len()
                );
                self.cache_chats_slice(&d.chats).await;
                self.cache_users_slice(&d.users).await;
            }
            tl::enums::messages::Dialogs::Slice(d) => {
                tracing::debug!(
                    "[ferogram::client] warm_peer_cache: slice {}, loaded {} dialogs, {} users, {} chats",
                    d.count,
                    d.dialogs.len(),
                    d.users.len(),
                    d.chats.len()
                );
                self.cache_chats_slice(&d.chats).await;
                self.cache_users_slice(&d.users).await;
            }
            tl::enums::messages::Dialogs::NotModified(_) => {
                tracing::debug!(
                    "[ferogram::client] warm_peer_cache: getDialogs returned NotModified; cache still current"
                );
            }
        }
        tracing::debug!("[ferogram::client] warm_peer_cache: peer cache refreshed from getDialogs");
        Ok(())
    }

    /// Route a file transfer RPC call through the dedicated transfer pool.
    ///
    /// Pass `dc_id = 0` for the home DC (uploads always go here).
    /// Pass the file's actual `dc_id` for downloads.
    ///
    /// The transfer pool is completely isolated from the main MTProto session:
    /// separate auth key, seq_no, msg_id stream, salt, and pending map.
    /// This prevents `Crypto(InvalidBuffer)` caused by mixing file traffic with
    /// the update/dialog stream on the main connection.
    pub(crate) async fn rpc_transfer_on_dc_pub<R: ferogram_tl_types::RemoteCall>(
        &self,
        dc_id: i32,

        req: &R,
    ) -> Result<Vec<u8>, InvocationError> {
        self.rpc_transfer_on_dc(dc_id, req).await
    }

    /// Internal: route req through the transfer pool for `dc_id`.
    async fn rpc_transfer_on_dc<R: RemoteCall>(
        &self,
        dc_id: i32,

        req: &R,
    ) -> Result<Vec<u8>, InvocationError> {
        let home = *self.inner.home_dc_id.lock().await;
        let target_dc = if dc_id == 0 { home } else { dc_id };

        // per-DC connect gate
        // Acquire (or create) a per-DC mutex that serialises the first-use
        // setup for each DC.  Tasks that arrive while another task is already
        // setting up the same DC will block here, then find the connection
        // ready in the pool (double-check below) and skip setup entirely.
        // This prevents redundant sockets and AUTH_KEY_UNREGISTERED caused by
        // two concurrent DH handshakes for the same DC slot.
        let gate: std::sync::Arc<tokio::sync::Mutex<()>> = {
            let mut gates = self.inner.dc_connect_gates.lock();
            gates
                .entry(target_dc)
                .or_insert_with(|| std::sync::Arc::new(tokio::sync::Mutex::new(())))
                .clone()
        };
        let _gate_guard = gate.lock().await;

        // Double-check: another task may have set up the connection while we
        // waited for the gate.
        let needs_new = {
            let pool = self.inner.transfer_pool.lock().await;
            !pool.has_connection(target_dc)
        };

        if needs_new {
            let addr = {
                let opts: tokio::sync::MutexGuard<'_, std::collections::HashMap<i32, DcEntry>> =
                    self.inner.dc_options.lock().await;
                opts.get(&target_dc)
                    .map(|e| e.addr.clone())
                    .unwrap_or_else(|| fallback_dc_addr(target_dc).to_string())
            };
            let socks5 = self.inner.socks5.clone();
            let mtproxy = self.inner.mtproxy.clone();

            // IMPORTANT: transfer connections always use Abridged transport (0xEF init byte)
            // regardless of the main connection transport.  Every read/write in DcConnection
            // uses send_abridged/recv_abridged, so the server MUST receive the 0xEF marker
            // first.  Using TransportKind::Full (no init byte) causes the server to fail
            // parsing the DH handshake and close the socket immediately → early EOF.

            if target_dc == home {
                // HOME DC: reuse the existing auth key  - no fresh DH, no export/import.
                tracing::debug!(
                    "[ferogram::transfer] using home auth key for DC{target_dc} (home=DC{home})"
                );
                // Read salt and time_offset from the live writer (FutureSalts may have
                // rotated since dc_options was last written).
                let key = {
                    let opts: tokio::sync::MutexGuard<'_, std::collections::HashMap<i32, DcEntry>> =
                        self.inner.dc_options.lock().await;
                    let e = opts.get(&target_dc);
                    e.and_then(|e| e.auth_key)
                };
                let (salt, time_offset) = {
                    let opts = self.inner.dc_options.lock().await;
                    let home = *self.inner.home_dc_id.lock().await;
                    opts.get(&home)
                        .map(|e| (e.first_salt, e.time_offset))
                        .unwrap_or((0, 0))
                };
                let conn = if let Some(key) = key {
                    dc_pool::DcConnection::connect_with_key(
                        &addr,
                        key,
                        salt,
                        time_offset,
                        socks5.as_ref(),
                        mtproxy.as_ref(),
                        &TransportKind::Abridged,
                        target_dc as i16,
                        self.inner.pfs_enabled,
                    )
                    .await?
                } else {
                    dc_pool::DcConnection::connect_raw(
                        &addr,
                        socks5.as_ref(),
                        &TransportKind::Abridged,
                        target_dc as i16,
                    )
                    .await?
                };
                // insert THEN init; remove on failure
                self.inner
                    .transfer_pool
                    .lock()
                    .await
                    .insert(target_dc, conn);
                if let Err(e) = self.init_transfer_session(target_dc).await {
                    tracing::warn!(
                        "[ferogram::transfer] initConnection for DC{target_dc} failed: {e}; evicting connection"
                    );
                    self.inner
                        .transfer_pool
                        .lock()
                        .await
                        .conns
                        .remove(&target_dc);
                    return Err(e);
                }
            } else {
                // FOREIGN DC: check for a cached auth key first.
                // If we already have the foreign DC's auth key (from a prior
                // export/import), skip DH + re-export and go straight to initConnection.
                let saved = {
                    let opts: tokio::sync::MutexGuard<'_, std::collections::HashMap<i32, DcEntry>> =
                        self.inner.dc_options.lock().await;
                    opts.get(&target_dc)
                        .and_then(|e| e.auth_key.map(|k| (k, e.first_salt, e.time_offset)))
                };

                if let Some((key, salt, time_offset)) = saved {
                    tracing::debug!(
                        "[ferogram::transfer] cached auth key for DC{target_dc}; running importAuth"
                    );
                    let conn = dc_pool::DcConnection::connect_with_key(
                        &addr,
                        key,
                        salt,
                        time_offset,
                        socks5.as_ref(),
                        mtproxy.as_ref(),
                        &TransportKind::Abridged,
                        target_dc as i16,
                        self.inner.pfs_enabled,
                    )
                    .await?;
                    // Cached key skips DH but importAuthorization is still required
                    // to activate the account on this session.
                    self.inner
                        .transfer_pool
                        .lock()
                        .await
                        .insert(target_dc, conn);
                    if let Err(e) = self.export_import_auth_transfer(target_dc).await {
                        tracing::warn!(
                            "[ferogram::transfer] importAuth for DC{target_dc} failed: {e}; evicting and retrying with fresh DH"
                        );
                        self.inner
                            .transfer_pool
                            .lock()
                            .await
                            .conns
                            .remove(&target_dc);
                        return Err(e);
                    }
                } else {
                    // No cached key: full DH + export/import.
                    tracing::debug!(
                        "[ferogram::transfer] no cached key for DC{target_dc}; running DH + importAuth"
                    );
                    let conn = dc_pool::DcConnection::connect_raw(
                        &addr,
                        socks5.as_ref(),
                        &TransportKind::Abridged,
                        target_dc as i16,
                    )
                    .await?;
                    // insert then import; evict on failure
                    self.inner
                        .transfer_pool
                        .lock()
                        .await
                        .insert(target_dc, conn);
                    if let Err(e) = self.export_import_auth_transfer(target_dc).await {
                        tracing::warn!(
                            "[ferogram::transfer] auth export/import for DC{target_dc} failed: {e}; evicting"
                        );
                        self.inner
                            .transfer_pool
                            .lock()
                            .await
                            .conns
                            .remove(&target_dc);
                        return Err(e);
                    }
                    // Save the newly obtained foreign-DC auth key so the NEXT
                    // transfer connection (and open_worker_conn) can skip DH.
                    //
                    // `ConnSlot` no longer owns a lockable `DcConnection` (the
                    // pipelined sender task does); `DcPool::collect_keys` is
                    // the pool's own sanctioned accessor for the auth key /
                    // salt / time offset snapshot taken when the slot was
                    // spawned, so we reuse it here for this single DC instead
                    // of reaching into private `ConnSlot` fields.
                    {
                        {
                            let pool = self.inner.transfer_pool.lock().await;
                            if pool.has_connection(target_dc) {
                                let mut opts: tokio::sync::MutexGuard<
                                    '_,
                                    std::collections::HashMap<i32, DcEntry>,
                                > = self.inner.dc_options.lock().await;
                                let entry = opts.entry(target_dc).or_insert_with(|| {
                                    crate::session::DcEntry {
                                        dc_id: target_dc,
                                        addr: addr.clone(),
                                        auth_key: None,
                                        first_salt: 0,
                                        time_offset: 0,
                                        flags: crate::session::DcFlags::NONE,
                                    }
                                });
                                let mut entries = [entry.clone()];
                                pool.collect_keys(&mut entries);
                                *entry = entries[0].clone();
                            }
                        }
                    }
                }
            }
        }

        let dc_entries: Vec<crate::DcEntry> = self
            .inner
            .dc_options
            .lock()
            .await
            .values()
            .cloned()
            .collect();
        let result = self
            .inner
            .transfer_pool
            .lock()
            .await
            .invoke_on_dc(target_dc, &dc_entries, req)
            .await;
        // Evict on fatal auth errors. Connection-death eviction (the old
        // Io(_) branch here) is now handled inside DcPool::invoke_on_dc
        // itself: connection failures surface as InvocationError::Deserialize
        // from the sender task, not Io, since fail_all has already fanned
        // the original error out to every caller waiting on that connection.
        if let Err(InvocationError::Rpc(rpc)) = &result
            && matches!(
                rpc.name.as_str(),
                "AUTH_KEY_UNREGISTERED"
                    | "SESSION_EXPIRED"
                    | "AUTH_KEY_INVALID"
                    | "AUTH_KEY_PERM_EMPTY"
            )
        {
            tracing::warn!(
                "[ferogram::transfer] auth error on DC{target_dc} ({}); evicting and clearing cached key",
                rpc.name
            );
            self.inner.transfer_pool.lock().await.evict(target_dc);
            let mut opts: tokio::sync::MutexGuard<'_, std::collections::HashMap<i32, DcEntry>> =
                self.inner.dc_options.lock().await;
            if let Some(e) = opts.get_mut(&target_dc) {
                e.auth_key = None;
            }
        }
        result
    }

    /// Initialize a home-DC transfer pool session by sending
    /// `invokeWithLayer(initConnection(..., help.getConfig))`.
    ///
    /// After `connect_with_key` the auth key is valid but Telegram doesn't know
    /// the client's layer yet; it will close the TCP connection on the first
    /// real RPC.  Sending `initConnection` here registers the session so that
    /// subsequent `upload.getFile` calls work correctly.
    async fn init_transfer_session(&self, dc_id: i32) -> Result<(), InvocationError> {
        use tl::functions::{InitConnection, InvokeWithLayer};
        let wrapped = InvokeWithLayer {
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
                query: tl::functions::help::GetConfig {},
            },
        };
        self.inner
            .transfer_pool
            .lock()
            .await
            .invoke_on_dc_serializable(dc_id, &wrapped)
            .await?;
        tracing::debug!("[ferogram::client] transfer connection to DC{dc_id} initialized");
        Ok(())
    }

    /// Export auth from the home DC (main connection) and import it into the
    /// transfer pool connection for `dc_id`.
    async fn export_import_auth_transfer(&self, dc_id: i32) -> Result<(), InvocationError> {
        // Export from the home (main) session  - works for home DC and foreign DCs.
        let export_req = tl::functions::auth::ExportAuthorization { dc_id };
        let body: Vec<u8> = self.rpc_call_raw(&export_req).await?;
        let mut cur = Cursor::from_slice(&body);
        let tl::enums::auth::ExportedAuthorization::ExportedAuthorization(exported) =
            tl::enums::auth::ExportedAuthorization::deserialize(&mut cur)?;

        // Wrap ImportAuthorization in invokeWithLayer(initConnection(...)) so Telegram
        // registers this as a fully-initialised session.  Without the wrapper Telegram
        // closes the connection on the next RPC with early-EOF or InvalidBuffer.
        use tl::functions::{InitConnection, InvokeWithLayer};
        let wrapped = InvokeWithLayer {
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
                query: tl::functions::auth::ImportAuthorization {
                    id: exported.id,
                    bytes: exported.bytes,
                },
            },
        };
        self.inner
            .transfer_pool
            .lock()
            .await
            .invoke_on_dc_serializable(dc_id, &wrapped)
            .await?;
        tracing::debug!(
            "[ferogram::client] transfer connection to DC{dc_id} initialized with imported auth"
        );
        Ok(())
    }

    /// Open a fresh, fully-initialised transfer connection for a single file worker.
    ///
    /// Each upload/download worker gets its **own** `DcConnection` so workers run
    /// truly in parallel without fighting over a shared mutex.
    ///
    /// * Home DC (`dc_id == 0`) -> reuses existing auth key, sends `initConnection`.
    /// * Foreign DC             -> fresh DH + `initConnection(importAuthorization)`.
    pub(crate) async fn open_worker_conn(
        &self,
        dc_id: i32,
    ) -> Result<dc_pool::DcConnection, InvocationError> {
        let home = *self.inner.home_dc_id.lock().await;
        let target_dc = if dc_id == 0 { home } else { dc_id };

        let addr = {
            let opts: tokio::sync::MutexGuard<'_, std::collections::HashMap<i32, DcEntry>> =
                self.inner.dc_options.lock().await;
            opts.get(&target_dc)
                .map(|e| e.addr.clone())
                .unwrap_or_else(|| fallback_dc_addr(target_dc).to_string())
        };
        let socks5 = self.inner.socks5.clone();
        let mtproxy = self.inner.mtproxy.clone();

        use tl::functions::{InitConnection, InvokeWithLayer};

        if target_dc == home {
            // auth_key comes from the session snapshot (persistent, never stale).
            // salt and time_offset come from the LIVE writer  - dc_options.first_salt
            // is only refreshed on reconnect, so it lags behind FutureSalts rotations
            // and new_session_created events.  Using a stale salt causes the worker's
            // first encrypted request to hit bad_server_salt, triggering an unnecessary
            // resend and often an early-eof on large-file transfers.
            let key = {
                let opts: tokio::sync::MutexGuard<'_, std::collections::HashMap<i32, DcEntry>> =
                    self.inner.dc_options.lock().await;
                opts.get(&target_dc).and_then(|e| e.auth_key)
            };
            let (salt, time_offset) = {
                let opts = self.inner.dc_options.lock().await;
                let home = *self.inner.home_dc_id.lock().await;
                opts.get(&home)
                    .map(|e| (e.first_salt, e.time_offset))
                    .unwrap_or((0, 0))
            };
            let mut conn = if let Some(key) = key {
                dc_pool::DcConnection::connect_with_key(
                    &addr,
                    key,
                    salt,
                    time_offset,
                    socks5.as_ref(),
                    mtproxy.as_ref(),
                    &TransportKind::Abridged,
                    target_dc as i16,
                    self.inner.pfs_enabled,
                )
                .await?
            } else {
                dc_pool::DcConnection::connect_raw(
                    &addr,
                    socks5.as_ref(),
                    &TransportKind::Abridged,
                    target_dc as i16,
                )
                .await?
            };
            conn.rpc_call_serializable(&InvokeWithLayer {
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
                    query: tl::functions::help::GetConfig {},
                },
            })
            .await?;
            tracing::debug!(
                "[ferogram::client] worker connection to DC{target_dc} ready (using home key)"
            );
            Ok(conn)
        } else {
            // Serialise export/import per DC: exportAuthorization tokens are single-use.
            let import_gate: std::sync::Arc<tokio::sync::Mutex<()>> = {
                let mut gates = self.inner.auth_import_gates.lock();
                gates
                    .entry(target_dc)
                    .or_insert_with(|| std::sync::Arc::new(tokio::sync::Mutex::new(())))
                    .clone()
            };
            let _import_guard = import_gate.lock().await;

            // Check for a cached auth key before opening a fresh connection.
            let saved = {
                let opts: tokio::sync::MutexGuard<'_, std::collections::HashMap<i32, DcEntry>> =
                    self.inner.dc_options.lock().await;
                opts.get(&target_dc)
                    .and_then(|e| e.auth_key.map(|k| (k, e.first_salt, e.time_offset)))
            };

            if let Some((key, salt, time_offset)) = saved {
                tracing::debug!(
                    "[ferogram::transfer] worker conn DC{target_dc}: cached key found, skipping DH"
                );
                let mut conn = dc_pool::DcConnection::connect_with_key(
                    &addr,
                    key,
                    salt,
                    time_offset,
                    socks5.as_ref(),
                    mtproxy.as_ref(),
                    &TransportKind::Abridged,
                    target_dc as i16,
                    self.inner.pfs_enabled,
                )
                .await?;

                // Re-check after acquiring gate: another worker may have already imported
                // during the same process lifetime. auth_imported is in-memory only and is
                // NOT cleared on reconnects. Authorization binding is per-session, not per-key.
                let already_imported = self.inner.auth_imported.lock().contains(&target_dc);

                if !already_imported {
                    // Must import: account authorization binding is not live on this session.
                    let export_req = tl::functions::auth::ExportAuthorization { dc_id: target_dc };
                    let body: Vec<u8> = self.rpc_call_raw(&export_req).await?;
                    let mut cur = Cursor::from_slice(&body);
                    let tl::enums::auth::ExportedAuthorization::ExportedAuthorization(exported) =
                        tl::enums::auth::ExportedAuthorization::deserialize(&mut cur)?;
                    conn.rpc_call_serializable(&InvokeWithLayer {
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
                            query: tl::functions::auth::ImportAuthorization {
                                id: exported.id,
                                bytes: exported.bytes,
                            },
                        },
                    })
                    .await?;
                    self.inner.auth_imported.lock().insert(target_dc);
                    tracing::debug!(
                        "[ferogram::transfer] worker conn DC{target_dc} ready (cached key, auth re-imported)"
                    );
                } else {
                    // Already imported this session; just register the layer.
                    conn.rpc_call_serializable(&InvokeWithLayer {
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
                            query: tl::functions::help::GetConfig {},
                        },
                    })
                    .await?;
                    tracing::debug!(
                        "[ferogram::transfer] worker conn DC{target_dc} ready (cached key, auth already active)"
                    );
                }
                Ok(conn)
            } else {
                // No cached key: full DH + export/import.
                let mut conn = dc_pool::DcConnection::connect_raw(
                    &addr,
                    socks5.as_ref(),
                    &TransportKind::Abridged,
                    target_dc as i16,
                )
                .await?;
                let export_req = tl::functions::auth::ExportAuthorization { dc_id: target_dc };
                let body: Vec<u8> = self.rpc_call_raw(&export_req).await?;
                let mut cur = Cursor::from_slice(&body);
                let tl::enums::auth::ExportedAuthorization::ExportedAuthorization(exported) =
                    tl::enums::auth::ExportedAuthorization::deserialize(&mut cur)?;
                conn.rpc_call_serializable(&InvokeWithLayer {
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
                        query: tl::functions::auth::ImportAuthorization {
                            id: exported.id,
                            bytes: exported.bytes,
                        },
                    },
                })
                .await?;
                // Save the newly established auth key so future worker connections
                // (and rpc_transfer_on_dc) can skip DH + export/import entirely.
                {
                    let mut opts: tokio::sync::MutexGuard<
                        '_,
                        std::collections::HashMap<i32, DcEntry>,
                    > = self.inner.dc_options.lock().await;
                    let entry = opts
                        .entry(target_dc)
                        .or_insert_with(|| crate::session::DcEntry {
                            dc_id: target_dc,
                            addr: addr.clone(),
                            auth_key: None,
                            first_salt: 0,
                            time_offset: 0,
                            flags: crate::session::DcFlags::NONE,
                        });
                    entry.auth_key = Some(conn.auth_key_bytes());
                    entry.first_salt = conn.first_salt();
                    entry.time_offset = conn.time_offset();
                }
                // Mark as auth-imported for this process session so subsequent
                // open_worker_conn calls on this DC can skip re-import.
                self.inner.auth_imported.lock().insert(target_dc);
                tracing::debug!("[ferogram::transfer] worker conn DC{target_dc} ready (fresh DH)");
                Ok(conn)
            }
        }
    }

    /// Open a pipelined transfer connection for `dc_id`: same DH / auth-import
    /// setup as [`open_worker_conn`](Self::open_worker_conn), but graduates
    /// the connection into a background [`ferogram_mtsender::sender_task`]
    /// instead of returning a blocking [`dc_pool::DcConnection`].
    ///
    /// The returned [`PipelinedSender`] supports **X > 1**: multiple chunk
    /// requests can be enqueued and in flight on the same connection at
    /// once, with responses matched back to callers out of order. This is
    /// the "X pieces in flight" half of Telegram's documented upload/download
    /// performance recommendation (the worker-count axis is "Y queues").
    /// `DcConnection::rpc_call` (used by [`open_worker_conn`](Self::open_worker_conn))
    /// only ever has X = 1: it blocks until its own response arrives before
    /// the caller can send anything else on that connection.
    pub(crate) async fn open_worker_sender(
        &self,
        dc_id: i32,
    ) -> Result<PipelinedSender, InvocationError> {
        let conn = self.open_worker_conn(dc_id).await?;
        let (stream, frame_kind, enc) = conn.into_parts();
        Ok(ferogram_mtsender::spawn_pipelined(
            stream, enc, frame_kind, None,
        ))
    }

    /// Like rpc_call_raw but takes a Serializable (for InvokeWithLayer wrappers).
    pub(crate) async fn rpc_call_raw_serializable<S: tl::Serializable>(
        &self,
        req: &S,
    ) -> Result<Vec<u8>, InvocationError> {
        let mut fail_count = NonZeroU32::new(1).expect("1 is nonzero");
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
        let raw_body = req.to_bytes();
        let body = maybe_gz_pack(&raw_body);
        let (tx, rx) = oneshot::channel();
        self.inner
            .rpc_tx
            .send(ferogram_mtsender::RpcEnqueue { body, tx })
            .await
            .map_err(|_| InvocationError::Deserialize("sender task shut down".into()))?;
        match rx.await {
            Ok(result) => result,
            Err(_) => Err(InvocationError::Deserialize(
                "rpc channel closed (sender task died?)".into(),
            )),
        }
    }

    /// Try to resolve a peer to InputPeer, returning an error if the access_hash
    /// is unknown (i.e. the peer has not been seen in any prior API call).
    pub async fn resolve_to_input_peer(
        &self,
        peer: &tl::enums::Peer,
    ) -> Result<tl::enums::InputPeer, InvocationError> {
        self.inner.peer_cache.read().await.peer_to_input(peer)
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
        <R::Return as tl::Deserializable>::from_bytes_exact(&body).map_err(Into::into)
    }

    /// Raw RPC call routed to `dc_id`, exporting auth if needed.
    ///
    /// Mirrors the setup sequence used by `open_worker_conn` / `rpc_transfer_on_dc`:
    /// - reads the *live* salt/time_offset (not 0,0) when reconnecting with a cached key,
    ///   since starting a connection with the wrong salt forces an extra bad_server_salt
    ///   round-trip on the very first request.
    /// - always wraps the connection's first request in `invokeWithLayer(initConnection(...))`,
    ///   including on the cached-key path, where the previous implementation skipped this
    ///   entirely. Without it the new session is never bound to the account on that DC and
    ///   never told which API layer to use, which is what caused inconsistent/failed
    ///   foreign-DC behavior while the home DC (already initialized) worked fine.
    /// - re-imports authorization on cached-key reconnects too (a cached auth *key* only
    ///   skips the DH handshake; the new session still needs `auth.importAuthorization`
    ///   to be usable for account requests on that DC). The old code never did this on
    ///   the cached-key branch, so a second call to a foreign DC could still misbehave.
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
            // Serialise setup per DC: concurrent callers racing to open the first
            // connection for the same DC must not both run DH/export/import.
            let gate: std::sync::Arc<tokio::sync::Mutex<()>> = {
                let mut gates = self.inner.auth_import_gates.lock();
                gates
                    .entry(dc_id)
                    .or_insert_with(|| std::sync::Arc::new(tokio::sync::Mutex::new(())))
                    .clone()
            };
            let _gate_guard = gate.lock().await;

            // Double-check: another task may have finished setup while we waited.
            let still_needs_new = {
                let pool = self.inner.dc_pool.lock().await;
                !pool.has_connection(dc_id)
            };

            if still_needs_new {
                let addr = {
                    let opts: tokio::sync::MutexGuard<'_, std::collections::HashMap<i32, DcEntry>> =
                        self.inner.dc_options.lock().await;
                    opts.get(&dc_id)
                        .map(|e| e.addr.clone())
                        .unwrap_or_else(|| fallback_dc_addr(dc_id).to_string())
                };

                let socks5 = self.inner.socks5.clone();
                let mtproxy = self.inner.mtproxy.clone();
                let transport = self.inner.transport.clone();
                let home_dc_id = *self.inner.home_dc_id.lock().await;

                use tl::functions::{InitConnection, InvokeWithLayer};

                let dc_conn = if dc_id == home_dc_id {
                    // Home DC: reuse the existing auth key (if any) with the live
                    // salt/time_offset, then just register the layer via GetConfig.
                    let key = {
                        let opts = self.inner.dc_options.lock().await;
                        opts.get(&dc_id).and_then(|e| e.auth_key)
                    };
                    let (salt, time_offset) = {
                        let opts = self.inner.dc_options.lock().await;
                        opts.get(&dc_id)
                            .map(|e| (e.first_salt, e.time_offset))
                            .unwrap_or((0, 0))
                    };
                    let mut conn = if let Some(key) = key {
                        dc_pool::DcConnection::connect_with_key(
                            &addr,
                            key,
                            salt,
                            time_offset,
                            socks5.as_ref(),
                            mtproxy.as_ref(),
                            &transport,
                            dc_id as i16,
                            self.inner.pfs_enabled,
                        )
                        .await?
                    } else {
                        dc_pool::DcConnection::connect_raw(
                            &addr,
                            socks5.as_ref(),
                            &transport,
                            dc_id as i16,
                        )
                        .await?
                    };
                    conn.rpc_call_serializable(&InvokeWithLayer {
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
                            query: tl::functions::help::GetConfig {},
                        },
                    })
                    .await?;
                    conn
                } else {
                    // Foreign DC: cached key skips DH but importAuthorization is
                    // still required to bind this NEW session to the account.
                    let saved = {
                        let opts = self.inner.dc_options.lock().await;
                        opts.get(&dc_id)
                            .and_then(|e| e.auth_key.map(|k| (k, e.first_salt, e.time_offset)))
                    };

                    if let Some((key, salt, time_offset)) = saved {
                        let mut conn = dc_pool::DcConnection::connect_with_key(
                            &addr,
                            key,
                            salt,
                            time_offset,
                            socks5.as_ref(),
                            mtproxy.as_ref(),
                            &transport,
                            dc_id as i16,
                            self.inner.pfs_enabled,
                        )
                        .await?;

                        let already_imported = self.inner.auth_imported.lock().contains(&dc_id);

                        if !already_imported {
                            let export_req = tl::functions::auth::ExportAuthorization { dc_id };
                            let body: Vec<u8> = self.rpc_call_raw(&export_req).await?;
                            let mut cur = Cursor::from_slice(&body);
                            let tl::enums::auth::ExportedAuthorization::ExportedAuthorization(
                                exported,
                            ) = tl::enums::auth::ExportedAuthorization::deserialize(&mut cur)?;
                            conn.rpc_call_serializable(&InvokeWithLayer {
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
                                    query: tl::functions::auth::ImportAuthorization {
                                        id: exported.id,
                                        bytes: exported.bytes,
                                    },
                                },
                            })
                            .await?;
                            self.inner.auth_imported.lock().insert(dc_id);
                            tracing::debug!(
                                "[ferogram::client] invoke_on_dc: DC{dc_id} ready (cached key, auth re-imported)"
                            );
                        } else {
                            conn.rpc_call_serializable(&InvokeWithLayer {
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
                                    query: tl::functions::help::GetConfig {},
                                },
                            })
                            .await?;
                            tracing::debug!(
                                "[ferogram::client] invoke_on_dc: DC{dc_id} ready (cached key, auth already active)"
                            );
                        }
                        conn
                    } else {
                        // No cached key: fresh DH + export/import, wrapped in initConnection.
                        let mut conn = dc_pool::DcConnection::connect_raw(
                            &addr,
                            socks5.as_ref(),
                            &transport,
                            dc_id as i16,
                        )
                        .await?;
                        let export_req = tl::functions::auth::ExportAuthorization { dc_id };
                        let body: Vec<u8> = self.rpc_call_raw(&export_req).await?;
                        let mut cur = Cursor::from_slice(&body);
                        let tl::enums::auth::ExportedAuthorization::ExportedAuthorization(exported) =
                            tl::enums::auth::ExportedAuthorization::deserialize(&mut cur)?;
                        conn.rpc_call_serializable(&InvokeWithLayer {
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
                                query: tl::functions::auth::ImportAuthorization {
                                    id: exported.id,
                                    bytes: exported.bytes,
                                },
                            },
                        })
                        .await?;
                        self.inner.auth_imported.lock().insert(dc_id);
                        tracing::debug!(
                            "[ferogram::client] invoke_on_dc: DC{dc_id} ready (fresh DH, auth imported)"
                        );
                        conn
                    }
                };

                // Persist the (possibly new) key + live salt/time_offset so the
                // next reconnect to this DC can skip DH with correct values.
                let key = dc_conn.auth_key_bytes();
                let live_salt = dc_conn.first_salt();
                let live_time_offset = dc_conn.time_offset();
                {
                    let mut opts: tokio::sync::MutexGuard<
                        '_,
                        std::collections::HashMap<i32, DcEntry>,
                    > = self.inner.dc_options.lock().await;
                    let entry = opts
                        .entry(dc_id)
                        .or_insert_with(|| crate::session::DcEntry {
                            dc_id,
                            addr: addr.clone(),
                            auth_key: None,
                            first_salt: 0,
                            time_offset: 0,
                            flags: crate::session::DcFlags::NONE,
                        });
                    entry.auth_key = Some(key);
                    entry.first_salt = live_salt;
                    entry.time_offset = live_time_offset;
                }
                self.inner.dc_pool.lock().await.insert(dc_id, dc_conn);
                self.inner.dc_pool.lock().await.mark_init_done(dc_id);
            }
        }

        let dc_entries: Vec<crate::DcEntry> = self
            .inner
            .dc_options
            .lock()
            .await
            .values()
            .cloned()
            .collect();
        // DcPool::invoke_on_dc evicts the broken slot and retries on a fresh
        // connection internally when the worker's sender task dies, so no
        // extra eviction is needed here. Connection failures surface as
        // InvocationError::Deserialize from the sender task (fail_all has no
        // way to preserve the original Io/etc. kind once it has fanned the
        // error out to every caller waiting on that connection), not as
        // InvocationError::Io, so don't match on the Io variant here.
        self.inner
            .dc_pool
            .lock()
            .await
            .invoke_on_dc(dc_id, &dc_entries, req)
            .await
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

/// Public wrapper for `random_i64` used by sub-modules.
#[doc(hidden)]
#[allow(dead_code)]
pub(crate) fn random_i64_pub() -> i64 {
    random_i64()
}

pub(crate) fn is_bool_true(body: &[u8]) -> bool {
    body.len() == 4 && u32::from_le_bytes(body[0..4].try_into().unwrap_or([0u8; 4])) == 0x997275b5
}

// Wire-layer types and helpers moved to ferogram-connect.
pub(crate) use ferogram_connect::util::maybe_gz_pack;
pub(crate) use ferogram_connect::{Connection, random_i64};

// Envelope types live in crate::envelope (tl-api functions, not in ferogram-connect).
pub(crate) use crate::envelope::{chat_to_peer, updates_entities};

// Low-level re-exports (merged from the former `layer` shim crate)

#[allow(unused_imports)]
/// Re-export of [`ferogram_mtproto`]: session, encrypted session, transport, and authentication.
pub use ferogram_mtproto as mtproto;

#[allow(unused_imports)]
/// Re-export of [`ferogram_crypto`]: AES-IGE, SHA, RSA, factorize, AuthKey.
pub use ferogram_crypto as crypto;

/// Re-export of [`ferogram_tl_parser`] (requires `feature = "parser"`).
#[cfg(feature = "parser")]
pub use ferogram_tl_parser as parser;

/// Re-export of [`ferogram_tl_gen`] (requires `feature = "codegen"`).
#[cfg(feature = "codegen")]
pub use ferogram_tl_gen as codegen;

// Convenience flat re-exports
#[allow(unused_imports)]
pub use ferogram_crypto::AuthKey;
#[allow(unused_imports)]
pub use ferogram_mtproto::authentication::{self, Finished, finish, step1, step2, step3};
#[allow(unused_imports)]
pub use ferogram_tl_types::{Identifiable, Serializable};
