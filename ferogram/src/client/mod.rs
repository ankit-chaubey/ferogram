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
    AutoSleep, ConnectionRestartPolicy, DcEntry, DcFlags, Dialog, ExperimentalFeatures,
    ForwardOptions, InputMessage, InvocationError, LinkKind, MiniApp, MiniAppSession, NeverRestart,
    PeerCache, PeerRef, RetryContext, RetryPolicy, dc_pool, message_box, persist,
};
use ferogram_tl_types as tl;
use ferogram_tl_types::{Cursor, Deserializable, RemoteCall};

use tokio::io::AsyncWriteExt;
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
/// | `Full` | none | Adds seqno + CRC32 |
/// | `Obfuscated` | random 64B | Bypasses DPI / MTProxy: **default** |
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
    /// Only the high-level [`Update`] queue your application reads via
    /// [`Client::stream_updates`] is governed here.
    ///
    /// Default: 2048-slot ring buffer with `DropOldest` overflow.
    pub update_config: crate::update_config::UpdateConfig,
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
            probe_transport: false,
            resilient_connect: false,
            use_pfs: false,
            experimental_features: ExperimentalFeatures::default(),
            update_config: crate::update_config::UpdateConfig::default(),
        }
    }
}
/// Asynchronous stream of [`Update`]s.
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
    /// Send () here to wake the network hint backoff.
    network_hint_tx: mpsc::Sender<()>,
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
    /// across all uploads and downloads to [`crate::media::MAX_GLOBAL_SENDERS`] (12).
    /// Each concurrent worker acquires one permit; it is released on drop.
    pub(crate) worker_semaphore: Arc<tokio::sync::Semaphore>,
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
}

/// The main Telegram client. Cheap to clone: internally Arc-wrapped.
#[derive(Clone)]
pub struct Client {
    pub(crate) inner: Arc<ClientInner>,
    _update_rx: Arc<Mutex<mpsc::Receiver<update::Update>>>,
}

mod auth;
mod bots;
mod chats;
mod dialogs;
mod files;
mod messages;
mod settings;
mod users;

impl Client {
    /// Return a fluent [`ClientBuilder`] for constructing and connecting a client.
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
        let transport = config.transport.clone();
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
                        tracing::info!("[ferogram] Loading session (DC{}) …", s.home_dc_id);
                        tracing::debug!(
                            "[ferogram] Session DC{} address: {}",
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
                                    "[ferogram] Session DC table loaded: {} entries, {} media/CDN",
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
                                    "[ferogram] Session connect failed ({e}): \
                                         returning error (delete session file to reset)"
                                );
                                return Err(e.into());
                            }
                        }
                    } else {
                        tracing::info!(
                            "[ferogram] Saved session for DC{} has no auth key, fresh login required …",
                            s.home_dc_id
                        );
                        let (c, dc, opts) = Self::fresh_connect_resilient(
                            socks5.as_ref(),
                            mtproxy.as_ref(),
                            &transport,
                            probe_transport,
                            resilient_connect,
                        )
                        .await?;
                        (c, dc, opts, HashMap::new(), None)
                    }
                } else {
                    tracing::info!(
                        "[ferogram] Saved session has no entry for home DC{}, fresh login required …",
                        s.home_dc_id
                    );
                    let (c, dc, opts) = Self::fresh_connect_resilient(
                        socks5.as_ref(),
                        mtproxy.as_ref(),
                        &transport,
                        probe_transport,
                        resilient_connect,
                    )
                    .await?;
                    (c, dc, opts, HashMap::new(), None)
                }
            }
            None => {
                tracing::info!("[ferogram] No saved session found, fresh login required …");
                let (c, dc, opts) = Self::fresh_connect_resilient(
                    socks5.as_ref(),
                    mtproxy.as_ref(),
                    &transport,
                    probe_transport,
                    resilient_connect,
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
            "[ferogram] DC pool + transfer pool initialized (home=DC{home_dc_id}, {} known DCs)",
            dc_opts.len()
        );

        // Hand the TcpStream directly to MtpSender: a single task owns both halves.
        let perm_auth_key = conn.perm_auth_key;
        tracing::debug!("[ferogram] Spawning sender task for DC{home_dc_id} …");
        let (sender_handle, frame_rx) = ferogram_mtsender::spawn_sender_task(
            conn.stream,
            conn.enc,
            conn.frame_kind,
            perm_auth_key,
        );

        // Channel for external "network restored" hints.
        let (network_hint_tx, network_hint_rx) = mpsc::channel::<()>(4);

        // Graceful shutdown token.
        let shutdown_token = CancellationToken::new();
        let catch_up = config.catch_up;
        let restart_policy = config.restart_policy;

        let inner = Arc::new(ClientInner {
            rpc_tx: sender_handle.rpc_tx,
            reconnect_tx: sender_handle.reconnect_tx,
            network_hint_tx,
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
                crate::media::MAX_GLOBAL_SENDERS,
            )),
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
        });

        let client = Self {
            inner,
            _update_rx: Arc::new(Mutex::new(update_rx)),
        };

        // Spawn the frame dispatch loop.
        // Receives FrameEvent from the sender task and routes updates / errors.
        // This replaces run_reader_task + reader_loop.
        {
            let client_d = client.clone();
            let shutdown_d = shutdown_token.clone();
            tokio::spawn(async move {
                client_d
                    .run_frame_dispatch(frame_rx, network_hint_rx, shutdown_d)
                    .await;
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
                                let b = &client_ps.inner.session_backend;
                                let _ = b.apply_update_state(
                                    ferogram_session::UpdateStateChange::Primary { pts, date, seq },
                                );
                                let _ = b.apply_update_state(
                                    ferogram_session::UpdateStateChange::Secondary { qts },
                                );
                            }
                            break;
                        }
                        _ = interval.tick() => {
                            let snap = client_ps.inner.message_box.lock().await.session_state();
                            let (pts, qts, date, seq) = (snap.pts, snap.qts, snap.date, snap.seq);
                            if pts > last_pts {
                                let backend = &client_ps.inner.session_backend;
                                let _ = backend.apply_update_state(
                                    ferogram_session::UpdateStateChange::Primary { pts, date, seq },
                                );
                                let _ = backend.apply_update_state(
                                    ferogram_session::UpdateStateChange::Secondary { qts },
                                );
                                last_pts = pts;
                                tracing::debug!(
                                    "[ferogram/persist] periodic save: pts={pts} qts={qts}"
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
                                    tracing::warn!("[ferogram/persist] full snapshot save failed: {e}");
                                    client_full
                                        .inner
                                        .session_snapshot_dirty
                                        .store(true, std::sync::atomic::Ordering::Release);
                                } else {
                                    tracing::debug!("[ferogram/persist] full snapshot saved");
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
                tracing::warn!("[ferogram] init_connection: definitive bad-key ({e}), fresh DH …");
                {
                    let home_dc_id = *client.inner.home_dc_id.lock().await;
                    let mut opts: tokio::sync::MutexGuard<
                        '_,
                        std::collections::HashMap<i32, DcEntry>,
                    > = client.inner.dc_options.lock().await;
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
                    "[ferogram] Session invalidated and reset. \
                     Call is_authorized() and re-authenticate if needed."
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
                if p.is_chat {
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
                "[ferogram] Peer cache restored: {} users, {} channels, {} chats, {} channels_min, {} min-contexts",
                cache.users.len(),
                cache.channels.len(),
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
                    "[ferogram] Update state restored: pts={}, qts={}, seq={}, {} channels",
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
            tracing::info!("[ferogram] catch_up: scheduling diff");
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
                tracing::debug!("[ferogram] probe_transport: Obfuscated starting (t=0 ms)");
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
                            "[ferogram] probe_transport: Obfuscated DH done in {ms} ms"
                        );
                        Ok((c, "Obfuscated", ms))
                    }
                    Err(e) => {
                        let ms = t0.elapsed().as_millis() as u64;
                        tracing::debug!(
                            "[ferogram] probe_transport: Obfuscated failed after {ms} ms: {e}"
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
                tracing::debug!("[ferogram] probe_transport: Abridged starting (t=200 ms)");
                tokio::time::sleep(std::time::Duration::from_millis(200)).await;
                let t0 = tokio::time::Instant::now();
                match Connection::connect_raw(&a, s.as_ref(), None, &TransportKind::Abridged, dc_id)
                    .await
                {
                    Ok(c) => {
                        let ms = t0.elapsed().as_millis() as u64;
                        tracing::debug!("[ferogram] probe_transport: Abridged DH done in {ms} ms");
                        Ok((c, "Abridged", ms))
                    }
                    Err(e) => {
                        let ms = t0.elapsed().as_millis() as u64;
                        tracing::debug!(
                            "[ferogram] probe_transport: Abridged failed after {ms} ms: {e}"
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
                tracing::debug!("[ferogram] probe_transport: Http starting (t=800 ms)");
                tokio::time::sleep(std::time::Duration::from_millis(800)).await;
                let t0 = tokio::time::Instant::now();
                match Connection::connect_raw(&a, None, None, &TransportKind::Http, dc_id).await {
                    Ok(c) => {
                        let ms = t0.elapsed().as_millis() as u64;
                        tracing::debug!("[ferogram] probe_transport: Http DH done in {ms} ms");
                        Ok((c, "Http", ms))
                    }
                    Err(e) => {
                        let ms = t0.elapsed().as_millis() as u64;
                        tracing::debug!(
                            "[ferogram] probe_transport: Http failed after {ms} ms: {e}"
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
                        "[ferogram] probe_transport winner: {label} ({ms} ms) - reusing connection, no second DH"
                    );
                    // drain cancelled tasks
                    while let Some(r) = set.join_next().await {
                        if let Err(e) = r
                            && e.is_cancelled()
                        {
                            tracing::debug!("[ferogram] probe_transport: slower transport aborted");
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
    ) -> Result<(Connection, i32, HashMap<i32, DcEntry>), InvocationError> {
        let dc_id: i16 = 2;
        let default_addr = fallback_dc_addr(dc_id as i32).to_owned();

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
            tracing::info!("[ferogram] probe_transport: racing transports for DC{dc_id} …");
            match Self::probe_transports_race(&default_addr, socks5, dc_id).await {
                Ok(conn) => return Ok((conn, dc_id as i32, build_opts())),
                Err(e) => {
                    tracing::warn!(
                        "[ferogram] probe_transport: all transports failed ({e}); \
                         falling through to resilient path"
                    );
                }
            }
        }

        // Normal direct connect.
        tracing::debug!("[ferogram] Fresh connect to DC{dc_id} ({default_addr}) …");
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
            "[ferogram] Direct connect failed ({direct_err}); \
             trying DNS-over-HTTPS fallback …"
        );
        let resolver = crate::dns_resolver::DnsResolver::new();
        let doh_ips = resolver.resolve("venus.web.telegram.org").await;
        let port = default_addr.split(':').next_back().unwrap_or("443");
        for ip in &doh_ips {
            let addr = format!("{ip}:{port}");
            tracing::info!("[ferogram] DoH resolved DC{dc_id} -> {addr}; connecting …");
            match Connection::connect_raw(&addr, socks5, mtproxy, transport, dc_id).await {
                Ok(conn) => {
                    tracing::info!("[ferogram] DoH fallback connect to DC{dc_id} ✓ ({addr})");
                    return Ok((conn, dc_id as i32, build_opts()));
                }
                Err(e) => tracing::debug!("[ferogram] DoH addr {addr} failed: {e}"),
            }
        }

        // Firebase / Google special-config fallback.
        tracing::warn!(
            "[ferogram] DoH fallback failed ({} candidates); \
             trying Firebase special-config …",
            doh_ips.len()
        );
        let special = crate::special_config::SpecialConfig::new();
        match special.fetch().await {
            Some(dc_options) => {
                for opt in dc_options.iter().filter(|o| o.dc_id == dc_id as i32) {
                    let addr = format!("{}:{}", opt.ip, opt.port);
                    tracing::info!(
                        "[ferogram] Firebase DC{} -> {addr}; connecting …",
                        opt.dc_id
                    );
                    match Connection::connect_raw(&addr, socks5, mtproxy, transport, dc_id).await {
                        Ok(conn) => {
                            tracing::info!("[ferogram] Firebase connect to DC{dc_id} ✓ ({addr})");
                            return Ok((conn, dc_id as i32, build_opts()));
                        }
                        Err(e) => tracing::debug!("[ferogram] Firebase addr {addr} failed: {e}"),
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
                });
            }
            for (&id, &(hash, kind)) in &cache.channels {
                v.push(CachedPeer {
                    id,
                    access_hash: hash,
                    is_channel: true,
                    is_chat: false,
                    channel_kind: kind.map(Into::into),
                });
            }
            for &id in &cache.chats {
                v.push(CachedPeer {
                    id,
                    access_hash: 0,
                    is_channel: false,
                    is_chat: true,
                    channel_kind: None,
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
        }
    }

    /// Persist the current session to the configured [`SessionBackend`].
    pub async fn save_session(&self) -> Result<(), InvocationError> {
        // build_persisted_session() is the source of truth for structural
        // session data: auth key, salts, DC table, peer cache.
        // MessageBoxes is the single authoritative source for pts/qts/date/seq.
        // build_persisted_session() already reads from message_box.session_state(),
        // so no secondary overwrite is needed.
        let session = self.build_persisted_session().await;

        self.inner
            .session_backend
            .save(&session)
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
                let b = &self.inner.session_backend;
                let _ = b.apply_update_state(ferogram_session::UpdateStateChange::Primary {
                    pts,
                    date,
                    seq,
                });
                let _ =
                    b.apply_update_state(ferogram_session::UpdateStateChange::Secondary { qts });
            }
        }

        tracing::info!("[ferogram] Session saved ✓");
        Ok(())
    }

    /// Export the session as a compact string (V2 format).
    ///
    /// Encodes dc_id, ip, port, user_id, and auth key. Store in an env var
    /// or secret manager and pass back to [`ClientBuilder::session_string`]
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
    /// to [`ClientBuilder::session_string`] which auto-detects the format.
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
                tracing::warn!("[ferogram] is_authorized: GetState rejected: {e}");
                Ok(false)
            }
            Err(e) => Err(e),
        }
    }

    /// Return an [`UpdateStream`] that yields incoming [`Update`]s.
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
                "[ferogram] stream_updates() called twice on the same Client: \
                 only one UpdateStream is supported per client. \
                 Returning a closed stream."
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
                                "[ferogram] update queue full (capacity {}): \
                                 evicted oldest to make room for incoming update.",
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
                                "[ferogram] update dropped: UpdateStream consumer is too slow \
                                 (channel full at capacity {}). Consider processing updates \
                                 faster or spawning handlers.",
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
        let _ = self.inner.network_hint_tx.try_send(());
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
        mut network_hint_rx: tokio::sync::mpsc::Receiver<()>,
        shutdown_token: tokio_util::sync::CancellationToken,
    ) {
        use ferogram_mtsender::FrameEvent;

        loop {
            tokio::select! {
                biased;
                _ = shutdown_token.cancelled() => {
                    tracing::info!("[ferogram] frame dispatch: shutdown");
                    return;
                }
                event = frame_rx.recv() => {
                    match event {
                        None => {
                            tracing::warn!("[ferogram] frame dispatch: sender task exited");
                            return;
                        }
                        Some(FrameEvent::Connected { auth_key, first_salt, time_offset, session_id }) => {
                            tracing::debug!(
                                "[ferogram] frame dispatch: connected sid={session_id:#x} salt={first_salt:#018x}"
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
                            tracing::warn!("[ferogram] frame dispatch: connection error: {e:?}");
                            // Sender task already called fail_all(); just reconnect.
                            self.handle_reconnect_after_error().await;
                        }
                    }
                }
                _ = network_hint_rx.recv() => {
                    // External hint (e.g. Android network-restored callback).
                    // The sender task will reconnect on its own; just wake the backoff.
                    tracing::debug!("[ferogram] frame dispatch: network hint received");
                }
            }
        }
    }

    /// Reconnect after a connection error from the sender task.
    /// Uses exponential backoff, then sends the new stream via reconnect_tx.
    async fn handle_reconnect_after_error(&self) {
        let mut delay_ms = RECONNECT_BASE_MS;
        loop {
            tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;

            let (addr, dc_id) = {
                let home = *self.inner.home_dc_id.lock().await;
                let dc_opts = self.inner.dc_options.lock().await;
                match dc_opts.get(&home) {
                    Some(e) => (e.addr.clone(), home as i16),
                    None => {
                        tracing::warn!("[ferogram] reconnect: no DC entry for home DC {home}");
                        delay_ms = (delay_ms * 2).min(RECONNECT_MAX_SECS * 1000);
                        continue;
                    }
                }
            };

            let socks5 = self.inner.socks5.as_ref().cloned();
            let mtproxy = self.inner.mtproxy.as_ref().cloned();
            let transport = self.inner.active_transport.lock().unwrap().clone();

            match Connection::connect_raw(
                &addr,
                socks5.as_ref(),
                mtproxy.as_ref(),
                &transport,
                dc_id,
            )
            .await
            {
                Ok(conn) => {
                    tracing::debug!("[ferogram] reconnect: TCP+DH OK");
                    {
                        let mut opts = self.inner.dc_options.lock().await;
                        if let Some(entry) = opts.get_mut(&(dc_id as i32)) {
                            entry.auth_key = Some(conn.enc.auth_key_bytes());
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
                    return;
                }
                Err(e) => {
                    tracing::warn!("[ferogram] reconnect attempt failed: {e}");
                    delay_ms = (delay_ms * 2).min(RECONNECT_MAX_SECS * 1000);
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
            tracing::warn!("[ferogram] updatesTooLong: triggering getDifference via message_box");
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
                        "[ferogram] updateShortSentMessage (server-push): pts={} pts_count={}",
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
                                tracing::warn!("[ferogram] update channel full: dropping update");
                                metrics::counter!("ferogram.updates_dropped").increment(1);
                            }
                        }
                    }
                }
                Err(_gap) => {
                    // Gap detected; deadline loop (Step 5) will fire getDifference.
                    tracing::debug!("[ferogram/msgbox] gap in container; deadline loop will diff");
                }
            }
        }
    }

    /// Persist a single update-state change to the session backend.
    fn persist_state(&self, change: ferogram_session::UpdateStateChange) {
        if let Err(e) = self.inner.session_backend.apply_update_state(change) {
            tracing::warn!("[ferogram/persist] state write failed: {e}");
        }
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
            "[ferogram] pts synced: pts={}, qts={}, seq={}",
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
            "[ferogram] force_sync_pts_state: pts={}, qts={}, seq={}",
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
                tracing::debug!("[ferogram] running getDifference");
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
                                    "[ferogram] getDifference parse failed: {e} \
                                     | first bytes: {:02x?}",
                                    preview
                                );
                                self.inner.message_box.lock().await.abort_difference();
                                // Force-advance pts to the server's current value so the
                                // stale gap does not immediately re-trigger getDifference
                                // (infinite loop when the server sends an unknown constructor
                                // from a newer TL layer).
                                match self.force_sync_pts_state().await {
                                    Ok(()) => {
                                        tracing::debug!(
                                            "[ferogram] pts resynced after getDifference \
                                             parse failure; resuming channel diffs"
                                        );
                                    }
                                    Err(sync_err) => {
                                        tracing::warn!(
                                            "[ferogram] force_sync_pts_state failed after \
                                             getDifference parse failure: {sync_err}"
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
                        tracing::warn!("[ferogram] getDifference RPC failed: {e}");
                        self.inner.message_box.lock().await.abort_difference();
                        // IO/transport error means the connection is dead. Break out
                        // so the reconnect path can notify diff_notify and start a
                        // fresh diff task once the connection is back.
                        if matches!(&e, InvocationError::Io(_)) {
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
                                        "[ferogram] auto_resolve: channel {channel_id} still no hash; deferring"
                                    );
                                    self.inner.message_box.lock().await.end_channel_difference(
                                        PrematureEndReason::TemporaryServerIssues,
                                    );
                                    continue;
                                }
                            }
                            Err(ref e) if e.is("CHANNEL_PRIVATE") => {
                                tracing::info!(
                                    "[ferogram] auto_resolve: channel {channel_id} CHANNEL_PRIVATE; deferring"
                                );
                                self.inner.message_box.lock().await.end_channel_difference(
                                    PrematureEndReason::TemporaryServerIssues,
                                );
                                continue;
                            }
                            Err(e) => {
                                tracing::warn!(
                                    "[ferogram] auto_resolve: channel {channel_id} fetch failed: {e}; deferring"
                                );
                                self.inner.message_box.lock().await.end_channel_difference(
                                    PrematureEndReason::TemporaryServerIssues,
                                );
                                continue;
                            }
                        }
                    } else {
                        tracing::debug!(
                            "[ferogram] no access_hash for channel {channel_id}; deferring diff"
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
                tracing::debug!("[ferogram] running getChannelDifference for {channel_id}");
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
                                    "[ferogram] getChannelDifference parse failed: {e} \
                                     | first bytes: {:02x?}",
                                    preview
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
                            "[ferogram] getChannelDifference: PERSISTENT_TIMESTAMP_OUTDATED"
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
                            "[ferogram] getChannelDifference: PERSISTENT_TIMESTAMP_INVALID for {channel_id}, resetting pts"
                        );
                        self.inner
                            .message_box
                            .lock()
                            .await
                            .end_channel_difference(PrematureEndReason::TemporaryServerIssues);
                    }
                    Err(ref e) if e.is("CHANNEL_PRIVATE") => {
                        tracing::info!(
                            "[ferogram] getChannelDifference: CHANNEL_PRIVATE for {channel_id}"
                        );
                        self.inner
                            .message_box
                            .lock()
                            .await
                            .end_channel_difference(PrematureEndReason::Banned);
                    }
                    Err(InvocationError::Rpc(ref rpc)) if rpc.code == 500 => {
                        tracing::warn!("[ferogram] getChannelDifference: server 500");
                        self.inner
                            .message_box
                            .lock()
                            .await
                            .end_channel_difference(PrematureEndReason::TemporaryServerIssues);
                    }
                    Err(e) => {
                        tracing::warn!(
                            "[ferogram] getChannelDifference for {channel_id} failed: {e}"
                        );
                        self.inner
                            .message_box
                            .lock()
                            .await
                            .end_channel_difference(PrematureEndReason::TemporaryServerIssues);
                        // Same as getDifference: IO error means dead connection; break so
                        // reconnect can notify diff_notify and retry.
                        if matches!(&e, InvocationError::Io(_)) {
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
                    tracing::warn!("[ferogram] update channel full: dropping update");
                    metrics::counter!("ferogram.updates_dropped").increment(1);
                }
            }
        }
    }

    /// Route one bare `tl::enums::Update` through the pts/qts gap-checker,
    /// then emit surviving updates to `update_tx`.
    pub async fn send_message(
        &self,
        peer: impl Into<PeerRef>,
        msg: impl Into<InputMessage>,
    ) -> Result<update::IncomingMessage, InvocationError> {
        let peer = peer.into().resolve(self).await?;
        let input_peer = self.inner.peer_cache.read().await.peer_to_input(&peer)?;
        let msg = msg.into();
        let entities = self.resolve_outgoing_entities(msg.entities.clone()).await;
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
            entities,
            schedule_date: msg.schedule_date,
            schedule_repeat_period: None,
            send_as: None,
            quick_reply_shortcut: None,
            effect: None,
            allow_paid_stars: None,
            suggested_post: None,
            rich_message: None,
        };
        let body: Vec<u8> = self.rpc_call_raw(&req).await?;
        Ok(self.parse_send_response(&body, &msg, &peer).await)
    }

    /// Convert `MessageEntity::MentionName` entities (what the markdown/html
    /// parsers emit for `tg://user?id=N`) into the `InputMessageEntityMentionName`
    /// constructor that Telegram actually requires on outgoing messages,
    /// resolving each user's `access_hash` from the peer cache.
    ///
    /// `messageEntityMentionName` (bare `user_id:long`, no access_hash) is the
    /// constructor Telegram sends back to you on *received* messages. Echoing
    /// it back on a *send* is a no-op server-side: Telegram can't resolve a
    /// peer from a bare integer, so the entity is silently dropped and the
    /// mention renders as plain text (0 entities, exactly as reported).
    ///
    /// If a mentioned user hasn't been seen yet (no cached access_hash), the
    /// entity is dropped with a warning rather than sending a request the
    /// server would reject anyway. The peer cache is populated automatically
    /// from incoming updates, so mentioning someone who has recently messaged
    /// the chat (e.g. the sender you're replying to) works as expected.
    pub(crate) async fn resolve_outgoing_entities(
        &self,
        entities: Option<Vec<tl::enums::MessageEntity>>,
    ) -> Option<Vec<tl::enums::MessageEntity>> {
        let entities = entities?;
        if !entities
            .iter()
            .any(|e| matches!(e, tl::enums::MessageEntity::MentionName(_)))
        {
            return Some(entities);
        }

        let cache = self.inner.peer_cache.read().await;
        let mut out = Vec::with_capacity(entities.len());
        for e in entities {
            match e {
                tl::enums::MessageEntity::MentionName(m) => {
                    match cache.users.get(&m.user_id).copied() {
                        Some(access_hash) => {
                            out.push(tl::enums::MessageEntity::InputMessageEntityMentionName(
                                tl::types::InputMessageEntityMentionName {
                                    offset: m.offset,
                                    length: m.length,
                                    user_id: tl::enums::InputUser::InputUser(
                                        tl::types::InputUser {
                                            user_id: m.user_id,
                                            access_hash,
                                        },
                                    ),
                                },
                            ));
                        }
                        None => {
                            tracing::warn!(
                                "[ferogram] dropping mention entity: user {} not in peer cache \
                                 (access_hash unknown) - Telegram would silently reject it",
                                m.user_id
                            );
                        }
                    }
                }
                other => out.push(other),
            }
        }
        if out.is_empty() { None } else { Some(out) }
    }

    pub(crate) async fn parse_send_response(
        &self,
        body: &[u8],

        input: &InputMessage,
        peer: &tl::enums::Peer,
    ) -> update::IncomingMessage {
        if body.len() < 4 {
            return self.synthetic_sent_from_short(input, peer, 0, 0);
        }
        let cid = u32::from_le_bytes(body[..4].try_into().unwrap());

        // updates#74ae4240 / updatesCombined#725b04c3: full Updates container
        if cid == 0x74ae4240 || cid == 0x725b04c3 {
            let mut cur = Cursor::from_slice(body);
            if let Ok(tl::enums::Updates::Updates(u)) = tl::enums::Updates::deserialize(&mut cur) {
                // Cache users/chats before the dispatch_updates spawn runs,
                // to prevent PeerNotCached races on the calling side.
                self.cache_users_and_chats(&u.users, &u.chats).await;
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
                self.cache_users_and_chats(&u.users, &u.chats).await;
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
        //
        // sent.media carries the real media for things sent to private chats
        // (dice, polls, photos, ...) - it must be threaded through, not dropped,
        // or callers get a synthetic message with media:None (e.g. send_dice()
        // would never be able to expose the rolled value).
        if cid == 0x9015e101 {
            let mut cur = Cursor::from_slice(&body[4..]);
            if let Ok(sent) = tl::types::UpdateShortSentMessage::deserialize(&mut cur) {
                let entities = sent.entities.clone().or_else(|| input.entities.clone());
                return self.synthetic_sent_from_short_ex(
                    input,
                    peer,
                    sent.id,
                    sent.date,
                    sent.media.clone(),
                    entities,
                );
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
                    guestchat_via_from: None,
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
                    rich_message: None,
                };
                return update::IncomingMessage::from_raw(tl::enums::Message::Message(msg))
                    .with_client(self.clone());
            }
        }

        // Fallback: synthetic stub with no message ID known
        self.synthetic_sent_from_short(input, peer, 0, 0)
    }

    #[allow(dead_code)]
    pub(crate) async fn extract_sent_message(
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
            guestchat_via_from: None,
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
                    reply_to_ephemeral: false,
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
            rich_message: None,
        };
        update::IncomingMessage::from_raw(tl::enums::Message::Message(msg))
            .with_client(self.clone())
    }

    fn synthetic_sent_from_short(
        &self,
        input: &InputMessage,

        peer: &tl::enums::Peer,
        id: i32,

        date: i32,
    ) -> update::IncomingMessage {
        self.synthetic_sent_from_short_ex(input, peer, id, date, None, input.entities.clone())
    }

    /// Like [`synthetic_sent_from_short`] but lets the caller supply the real
    /// `media` and `entities` returned by the server (e.g. from
    /// `updateShortSentMessage`) instead of always reconstructing from `input`.
    fn synthetic_sent_from_short_ex(
        &self,
        input: &InputMessage,
        peer: &tl::enums::Peer,
        id: i32,
        date: i32,
        media: Option<tl::enums::MessageMedia>,
        entities: Option<Vec<tl::enums::MessageEntity>>,
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
            guestchat_via_from: None,
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
                    reply_to_ephemeral: false,
                })
            }),
            date,
            message: input.text.clone(),
            media,
            reply_markup: input.reply_markup.clone(),
            entities,
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
            rich_message: None,
        };
        update::IncomingMessage::from_raw(tl::enums::Message::Message(msg))
            .with_client(self.clone())
    }

    pub async fn open_mini_app(
        &self,
        peer: impl Into<PeerRef>,

        app: MiniApp,
    ) -> Result<MiniAppSession, InvocationError> {
        let peer_ref = peer.into().resolve(self).await?;
        let input_peer = self
            .inner
            .peer_cache
            .read()
            .await
            .peer_to_input(&peer_ref)?;

        match app {
            MiniApp::Main => {
                let res = self
                    .invoke(&tl::functions::messages::RequestMainWebView {
                        compact: false,
                        fullscreen: false,
                        peer: input_peer.clone(),
                        bot: tl::enums::InputUser::UserSelf,
                        start_param: None,
                        theme_params: None,
                        platform: "android".into(),
                    })
                    .await
                    .map(|r: tl::enums::WebViewResult| r)?;
                MiniAppSession::from_result(self.clone(), input_peer, res)
            }
            MiniApp::Url(url) => {
                let res = self
                    .invoke(&tl::functions::messages::RequestWebView {
                        compact: false,
                        fullscreen: false,
                        from_bot_menu: false,
                        silent: false,
                        peer: input_peer.clone(),
                        bot: tl::enums::InputUser::UserSelf,
                        url: Some(url),
                        start_param: None,
                        theme_params: None,
                        platform: "android".into(),
                        reply_to: None,
                        send_as: None,
                    })
                    .await
                    .map(|r: tl::enums::WebViewResult| r)?;
                MiniAppSession::from_result(self.clone(), input_peer, res)
            }
            MiniApp::App {
                bot: _,
                app,
                start_param,
            } => {
                let res = self
                    .invoke(&tl::functions::messages::RequestAppWebView {
                        compact: false,
                        fullscreen: false,
                        write_allowed: false,
                        peer: input_peer.clone(),
                        app,
                        start_param,
                        theme_params: None,
                        platform: "android".into(),
                    })
                    .await
                    .map(|r: tl::enums::WebViewResult| r)?;
                MiniAppSession::from_result(self.clone(), input_peer, res)
            }
            MiniApp::Simple(url) => {
                let res = self
                    .invoke(&tl::functions::messages::RequestSimpleWebView {
                        compact: false,
                        fullscreen: false,
                        from_switch_webview: false,
                        from_side_menu: false,
                        bot: tl::enums::InputUser::UserSelf,
                        url: Some(url),
                        start_param: None,
                        theme_params: None,
                        platform: "android".into(),
                    })
                    .await
                    .map(|r: tl::enums::WebViewResult| r)?;
                Ok(MiniAppSession {
                    url: match res {
                        tl::enums::WebViewResult::Url(r) => r.url,
                    },
                    query_id: None,
                    client: self.clone(),
                    input_peer,
                })
            }
        }
    }

    pub async fn send_to_self(
        &self,
        msg: impl Into<InputMessage>,
    ) -> Result<update::IncomingMessage, InvocationError> {
        let msg = msg.into();
        let req = tl::functions::messages::SendMessage {
            no_webpage: msg.no_webpage,
            silent: msg.silent,
            background: msg.background,
            clear_draft: msg.clear_draft,
            noforwards: false,
            update_stickersets_order: false,
            invert_media: msg.invert_media,
            allow_paid_floodskip: false,
            peer: tl::enums::InputPeer::PeerSelf,
            reply_to: msg.reply_header(),
            message: msg.text.clone(),
            random_id: random_i64(),
            reply_markup: msg.reply_markup.clone(),
            entities: msg.entities.clone(),
            schedule_date: msg.schedule_date,
            schedule_repeat_period: None,
            send_as: None,
            quick_reply_shortcut: None,
            effect: None,
            allow_paid_stars: None,
            suggested_post: None,
            rich_message: None,
        };
        let body: Vec<u8> = self.rpc_call_raw(&req).await?;
        let self_peer = tl::enums::Peer::User(tl::types::PeerUser { user_id: 0 });
        Ok(self.parse_send_response(&body, &msg, &self_peer).await)
    }

    /// Edit the text of an existing message.
    pub async fn edit_message(
        &self,
        peer: impl Into<PeerRef>,
        message_id: i32,
        new_text: impl Into<InputMessage>,
    ) -> Result<(), InvocationError> {
        let peer = peer.into().resolve(self).await?;
        let input_peer = self.inner.peer_cache.read().await.peer_to_input(&peer)?;
        let msg = new_text.into();
        let req = tl::functions::messages::EditMessage {
            no_webpage: msg.no_webpage,
            invert_media: msg.invert_media,
            peer: input_peer,
            id: message_id,
            message: Some(msg.text.clone()),
            media: msg.media.clone(),
            reply_markup: msg.reply_markup.clone(),
            entities: msg.entities.clone(),
            schedule_date: msg.schedule_date,
            quick_reply_shortcut_id: None,
            schedule_repeat_period: None,
            rich_message: None,
        };
        self.rpc_write(&req).await
    }

    pub async fn forward_messages(
        &self,
        destination: impl Into<PeerRef>,

        message_ids: &[i32],
        source: impl Into<PeerRef>,

        opts: ForwardOptions,
    ) -> Result<Vec<update::IncomingMessage>, InvocationError> {
        let dest = destination.into().resolve(self).await?;
        let src = source.into().resolve(self).await?;
        let cache: tokio::sync::RwLockReadGuard<'_, PeerCache> = self.inner.peer_cache.read().await;
        let to_peer = cache.peer_to_input(&dest)?;
        let from_peer = cache.peer_to_input(&src)?;
        drop(cache);

        let reply_to = opts.reply_to.map(|id| {
            tl::enums::InputReplyTo::Message(tl::types::InputReplyToMessage {
                reply_to_msg_id: id,
                top_msg_id: None,
                reply_to_peer_id: None,
                quote_text: None,
                quote_entities: None,
                quote_offset: None,
                monoforum_peer_id: None,
                poll_option: None,
                todo_item_id: None,
            })
        });

        let req = tl::functions::messages::ForwardMessages {
            silent: opts.silent,
            background: false,
            with_my_score: false,
            drop_author: opts.drop_author,
            drop_media_captions: opts.drop_media_captions,
            noforwards: opts.noforwards,
            from_peer,
            id: message_ids.to_vec(),
            random_id: (0..message_ids.len()).map(|_| random_i64()).collect(),
            to_peer,
            top_msg_id: None,
            reply_to,
            schedule_date: opts.schedule_date,
            schedule_repeat_period: None,
            send_as: None,
            quick_reply_shortcut: None,
            effect: None,
            video_timestamp: None,
            allow_paid_stars: None,
            allow_paid_floodskip: false,
            suggested_post: None,
        };
        let body: Vec<u8> = self.rpc_call_raw(&req).await?;
        // Parse the Updates container, cache peer info, and collect messages.
        let mut out = Vec::new();
        if body.len() >= 4 {
            let cid = u32::from_le_bytes(body[..4].try_into().unwrap());
            if cid == 0x74ae4240 || cid == 0x725b04c3 {
                let updates_opt = match tl::enums::Updates::from_bytes_exact(&body) {
                    Ok(updates) => Some(updates),
                    Err(e) => {
                        tracing::warn!("[ferogram] updates parse error: {e}");
                        None
                    }
                };
                let (raw_updates, users, chats) = match updates_opt {
                    Some(tl::enums::Updates::Updates(u)) => (u.updates, u.users, u.chats),
                    Some(tl::enums::Updates::Combined(u)) => (u.updates, u.users, u.chats),
                    _ => (vec![], vec![], vec![]),
                };
                // Cache peers so returned IncomingMessage objects are immediately usable.
                self.cache_users_and_chats(&users, &chats).await;
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

    #[allow(dead_code)]
    pub(crate) async fn delete_messages_raw(
        &self,
        message_ids: &[i32],
        revoke: bool,
    ) -> Result<(), InvocationError> {
        let req = tl::functions::messages::DeleteMessages {
            revoke,
            id: message_ids.to_vec(),
        };
        self.rpc_write(&req).await
    }

    pub async fn delete_messages(
        &self,
        message_ids: &[i32],
        revoke: bool,
    ) -> Result<(), InvocationError> {
        let req = tl::functions::messages::DeleteMessages {
            revoke,
            id: message_ids.to_vec(),
        };
        self.rpc_write(&req).await
    }

    /// Fetch a single message by ID.
    pub async fn get_messages(
        &self,
        peer: impl Into<PeerRef>,
        ids: &[i32],
    ) -> Result<Vec<update::IncomingMessage>, InvocationError> {
        let peer = peer.into().resolve(self).await?;
        let input_peer = self.inner.peer_cache.read().await.peer_to_input(&peer)?;
        let id_list: Vec<tl::enums::InputMessage> = ids
            .iter()
            .map(|&id| tl::enums::InputMessage::Id(tl::types::InputMessageId { id }))
            .collect();
        let body: Vec<u8> = match &input_peer {
            tl::enums::InputPeer::Channel(c) => {
                let req = tl::functions::channels::GetMessages {
                    channel: tl::enums::InputChannel::InputChannel(tl::types::InputChannel {
                        channel_id: c.channel_id,
                        access_hash: c.access_hash,
                    }),
                    id: id_list,
                };
                self.rpc_call_raw(&req).await?
            }
            _ => {
                let req = tl::functions::messages::GetMessages { id: id_list };
                self.rpc_call_raw(&req).await?
            }
        };
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

    pub async fn get_pinned_message(
        &self,
        peer: impl Into<PeerRef>,
    ) -> Result<Option<update::IncomingMessage>, InvocationError> {
        let peer = peer.into().resolve(self).await?;
        let input_peer = self.inner.peer_cache.read().await.peer_to_input(&peer)?;
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
        let body: Vec<u8> = self.rpc_call_raw(&req).await?;
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

    /// Pin or unpin a message. `pin: true` pins, `pin: false` unpins.
    pub async fn pin_message(
        &self,
        peer: impl Into<PeerRef>,
        id: i32,
        pin: bool,
    ) -> Result<(), InvocationError> {
        self.update_pinned_message(peer, id, true, !pin, false)
            .await
    }

    pub(crate) async fn update_pinned_message(
        &self,
        peer: impl Into<PeerRef>,
        message_id: i32,
        silent: bool,
        unpin: bool,
        pm_oneside: bool,
    ) -> Result<(), InvocationError> {
        let peer = peer.into().resolve(self).await?;
        let input_peer = self.inner.peer_cache.read().await.peer_to_input(&peer)?;
        let req = tl::functions::messages::UpdatePinnedMessage {
            silent,
            unpin,
            pm_oneside,
            peer: input_peer,
            id: message_id,
        };
        self.rpc_write(&req).await
    }

    pub(crate) async fn pin_message_raw(
        &self,
        peer: impl Into<PeerRef>,
        message_id: i32,
    ) -> Result<(), InvocationError> {
        self.update_pinned_message(peer, message_id, true, false, false)
            .await
    }

    /// Fetch the message that `message` is replying to.
    ///
    /// Returns `None` if the message is not a reply, or if the original
    /// message could not be found (deleted / inaccessible).
    ///
    /// # Example
    /// ```rust,ignore
    /// # async fn f(client: ferogram::Client, msg: ferogram::update::IncomingMessage)
    /// #   -> Result<(), ferogram::InvocationError> {
    /// if let Some(replied) = client.get_reply_to_message(&msg).await? {
    /// println!("Replied to: {:?}", replied.text());
    /// }
    /// # Ok(()) }
    /// ```
    pub(crate) async fn get_reply_to_message(
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
        let input_peer = self.inner.peer_cache.read().await.peer_to_input(&peer)?;
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

    pub async fn unpin_all_messages(
        &self,
        peer: impl Into<PeerRef>,
    ) -> Result<(), InvocationError> {
        let peer = peer.into().resolve(self).await?;
        let input_peer = self.inner.peer_cache.read().await.peer_to_input(&peer)?;
        let req = tl::functions::messages::UnpinAllMessages {
            peer: input_peer,
            top_msg_id: None,
            saved_peer_id: None,
        };
        self.rpc_write(&req).await
    }

    pub async fn get_scheduled_messages(
        &self,
        peer: impl Into<PeerRef>,
    ) -> Result<Vec<update::IncomingMessage>, InvocationError> {
        let peer = peer.into().resolve(self).await?;
        let input_peer = self.inner.peer_cache.read().await.peer_to_input(&peer)?;
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

    pub async fn delete_scheduled_messages(
        &self,
        peer: impl Into<PeerRef>,
        ids: &[i32],
    ) -> Result<(), InvocationError> {
        let peer = peer.into().resolve(self).await?;
        let input_peer = self.inner.peer_cache.read().await.peer_to_input(&peer)?;
        let req = tl::functions::messages::DeleteScheduledMessages {
            peer: input_peer,
            id: ids.to_vec(),
        };
        self.rpc_write(&req).await
    }

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
        let body: Vec<u8> = self.rpc_call_raw(&req).await?;
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
        let body: Vec<u8> = self.rpc_call_raw(&req).await?;
        Ok(body.len() >= 4 && u32::from_le_bytes(body[..4].try_into().unwrap()) == 0x997275b5)
    }

    pub async fn get_dialogs_slice(
        &self,
        req: tl::functions::messages::GetDialogs,
    ) -> Result<Vec<Dialog>, InvocationError> {
        let body: Vec<u8> = self.rpc_call_raw(&req).await?;
        let raw = match tl::enums::messages::Dialogs::from_bytes_exact(&body)? {
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

    pub async fn get_dialogs_paginated(
        &self,
        req: tl::functions::messages::GetDialogs,
    ) -> Result<(Vec<Dialog>, Option<i32>), InvocationError> {
        let body: Vec<u8> = self.rpc_call_raw(&req).await?;
        let (raw, count) = match tl::enums::messages::Dialogs::from_bytes_exact(&body)? {
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

    pub async fn get_message_history(
        &self,
        peer: impl Into<PeerRef>,
        limit: i32,
        offset_id: i32,
    ) -> Result<Vec<update::IncomingMessage>, InvocationError> {
        let peer = peer.into().resolve(self).await?;
        let input_peer = self.inner.peer_cache.read().await.peer_to_input(&peer)?;
        let req = tl::functions::messages::GetHistory {
            peer: input_peer,
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

    /// Download message media to any [`AsyncWrite`] sink (file, socket, in-memory buffer…).
    ///
    /// Returns total bytes written. Uses the sequential streaming path so the
    /// entire file is never buffered in memory.
    ///
    /// [`AsyncWrite`]: tokio::io::AsyncWrite
    ///
    /// # Example
    /// ```rust,no_run
    /// # use ferogram::Client; use ferogram_tl_types as tl;
    /// # async fn ex(client: Client, msg: ferogram::update::IncomingMessage) {
    /// // to memory
    /// let mut buf = Vec::new();
    /// client.download(msg.media().unwrap(), &mut buf, None).await.unwrap();
    ///
    /// // to file
    /// let mut file = tokio::fs::File::create("photo.jpg").await.unwrap();
    /// client.download(msg.media().unwrap(), &mut file, None).await.unwrap();
    /// # }
    /// ```
    pub async fn download(
        &self,
        media: &tl::enums::MessageMedia,
        mut dest: impl tokio::io::AsyncWrite + Unpin,
        handle: Option<&crate::transfer::TransferHandle>,
    ) -> Result<u64, InvocationError> {
        let (loc, dc) = crate::media::location_from_media(media).ok_or_else(|| {
            InvocationError::Deserialize("media has no downloadable location".into())
        })?;
        if let Some(h) = handle {
            let total = crate::media::size_from_media(media).unwrap_or(0);
            h.set_total(total as u64);
            h.reset_start();
        }
        self.download_streaming_on_dc(loc, dc, &mut dest, handle)
            .await
    }

    /// Download message media directly to a file at `path`. Returns bytes written.
    ///
    /// Creates (or truncates) the file. Streams directly to disk.
    /// Use [`download_file_with_handle`] to track progress or support pause/cancel.
    ///
    /// [`download_file_with_handle`]: Client::download_file_with_handle
    pub async fn download_file(
        &self,
        media: &tl::enums::MessageMedia,
        path: impl AsRef<std::path::Path>,
    ) -> Result<u64, InvocationError> {
        self.download_file_with_handle(media, path, None).await
    }

    /// Like [`download_file`] but accepts a [`TransferHandle`] for progress
    /// tracking or pause/cancel support.
    ///
    /// [`download_file`]: Client::download_file
    /// [`TransferHandle`]: crate::transfer::TransferHandle
    pub async fn download_file_with_handle(
        &self,
        media: &tl::enums::MessageMedia,
        path: impl AsRef<std::path::Path>,
        handle: Option<&crate::transfer::TransferHandle>,
    ) -> Result<u64, InvocationError> {
        let mut file = tokio::fs::File::create(path)
            .await
            .map_err(InvocationError::Io)?;
        self.download(media, &mut file, handle).await
    }

    /// Return a lazy chunk iterator for `media`.
    ///
    /// Call [`DownloadIter::next`] until it returns `Ok(None)`. Each chunk is
    /// a [`bytes::Bytes`] slice - zero-copy where possible.
    ///
    /// Returns `None` if `media` has no downloadable location.
    pub fn iter_download(
        &self,
        media: &tl::enums::MessageMedia,
    ) -> Option<crate::media::DownloadIter> {
        let (loc, dc) = crate::media::location_from_media(media)?;
        Some(crate::media::DownloadIter::new(self.clone(), loc, dc))
    }

    /// Upload from any [`AsyncRead`] source.
    ///
    /// Buffers the stream to determine size, then uploads using the optimal
    /// part size and worker count. Use [`upload_file`] when you have a path -
    /// it avoids the double-buffer.
    ///
    /// Use [`upload_with_handle`] to track progress or support pause/cancel.
    ///
    /// [`AsyncRead`]: tokio::io::AsyncRead
    /// [`upload_file`]: Client::upload_file
    /// [`upload_with_handle`]: Client::upload_with_handle
    pub async fn upload(
        &self,
        source: impl tokio::io::AsyncRead + Unpin + Send,
        name: &str,
    ) -> Result<crate::media::UploadedFile, InvocationError> {
        self.upload_with_handle(source, name, None).await
    }

    /// Like [`upload`] but accepts a [`TransferHandle`] for progress tracking
    /// or pause/cancel support.
    ///
    /// [`upload`]: Client::upload
    /// [`TransferHandle`]: crate::transfer::TransferHandle
    pub async fn upload_with_handle(
        &self,
        mut source: impl tokio::io::AsyncRead + Unpin + Send,
        name: &str,
        handle: Option<&crate::transfer::TransferHandle>,
    ) -> Result<crate::media::UploadedFile, InvocationError> {
        use tokio::io::AsyncReadExt;
        let mut data = Vec::new();
        source
            .read_to_end(&mut data)
            .await
            .map_err(InvocationError::Io)?;
        if data.len() > crate::media::BIG_FILE_THRESHOLD {
            self.upload_file_concurrent(std::sync::Arc::new(data), name, "", handle)
                .await
        } else {
            self.upload_bytes(&data, name, "", handle).await
        }
    }

    /// Upload a file from disk by path.
    ///
    /// Stats the file first (for optimal part sizing), then streams in chunks.
    /// Use [`upload_file_with_handle`] to track progress or support pause/cancel.
    ///
    /// [`upload_file_with_handle`]: Client::upload_file_with_handle
    pub async fn upload_file(
        &self,
        path: impl AsRef<std::path::Path>,
    ) -> Result<crate::media::UploadedFile, InvocationError> {
        self.upload_file_with_handle(path, None).await
    }

    /// Like [`upload_file`] but accepts a [`TransferHandle`] for progress
    /// tracking or pause/cancel support.
    ///
    /// [`upload_file`]: Client::upload_file
    /// [`TransferHandle`]: crate::transfer::TransferHandle
    pub async fn upload_file_with_handle(
        &self,
        path: impl AsRef<std::path::Path>,
        handle: Option<&crate::transfer::TransferHandle>,
    ) -> Result<crate::media::UploadedFile, InvocationError> {
        use tokio::io::AsyncReadExt;
        let path = path.as_ref();
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("file");
        let mut file = tokio::fs::File::open(path)
            .await
            .map_err(InvocationError::Io)?;
        let meta = file.metadata().await.map_err(InvocationError::Io)?;
        let size = meta.len() as usize;
        let mut data = Vec::with_capacity(size);
        file.read_to_end(&mut data)
            .await
            .map_err(InvocationError::Io)?;
        if data.len() > crate::media::BIG_FILE_THRESHOLD {
            self.upload_file_concurrent(std::sync::Arc::new(data), name, "", handle)
                .await
        } else {
            self.upload_bytes(&data, name, "", handle).await
        }
    }

    pub async fn send_chat_action(
        &self,
        peer: impl Into<PeerRef>,

        action: tl::enums::SendMessageAction,
    ) -> Result<(), InvocationError> {
        let peer = peer.into().resolve(self).await?;
        self.send_chat_action_ex(peer, action, None).await
    }

    pub async fn get_history_range(
        &self,
        peer: impl Into<PeerRef>,

        limit: i32,
        offset_id: i32,
    ) -> Result<Vec<update::IncomingMessage>, InvocationError> {
        let peer = peer.into().resolve(self).await?;
        let input_peer = self.inner.peer_cache.read().await.peer_to_input(&peer)?;
        let req = tl::functions::messages::GetHistory {
            peer: input_peer,
            offset_id,
            offset_date: 0,
            add_offset: 0,
            limit,
            max_id: 0,
            min_id: 0,
            hash: 0,
        };
        let body: Vec<u8> = self.rpc_call_raw(&req).await?;
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

    /// Resolve any peer reference to a [`tl::enums::Peer`].
    ///
    /// Accepts everything [`PeerRef`] accepts:
    ///
    /// - `&str` / `String`: `"@username"`, `"me"`, `"self"`, numeric string,
    ///   `t.me/` URL, invite link, E.164 phone
    /// - `i64` / `i32`: Bot-API encoded numeric ID
    /// - [`tl::enums::Peer`]: returned as-is (zero cost)
    /// - [`tl::enums::InputPeer`]: hash cached, then stripped to `Peer`
    ///
    /// Resolution is cache-first; an RPC is only made on a genuine cache miss.
    pub async fn resolve<P: Into<PeerRef>>(
        &self,
        peer: P,
    ) -> Result<tl::enums::Peer, InvocationError> {
        peer.into().resolve(self).await
    }

    /// `contacts.resolveUsername` RPC; called only on cache miss.
    pub(crate) async fn resolve_username_rpc(
        &self,
        username: &str,
    ) -> Result<tl::enums::Peer, InvocationError> {
        let req = tl::functions::contacts::ResolveUsername {
            username: username.to_string(),
            referer: None,
        };
        let body: Vec<u8> = self.rpc_call_raw(&req).await?;
        let mut cur = Cursor::from_slice(&body);
        let tl::enums::contacts::ResolvedPeer::ResolvedPeer(resolved) =
            tl::enums::contacts::ResolvedPeer::deserialize(&mut cur)?;
        self.cache_users_slice(&resolved.users).await;
        self.cache_chats_slice(&resolved.chats).await;
        Ok(resolved.peer)
    }

    /// RPC fallback for `PeerRef::Id` when the peer is not in the cache.
    ///
    /// - `Peer::User`    → `users.getUsers` with `access_hash = 0` (works for
    ///   contacts and recently-interacted users; may return `UserEmpty` for
    ///   strangers; falls back to `messages.getPeerDialogs` in that case).
    /// - `Peer::Channel` → `channels.getChannels` with `access_hash = 0`.
    ///   This works only for public channels; returns `ChannelEmpty` otherwise.
    /// - `Peer::Chat`    → basic groups never need a hash; return immediately.
    ///
    /// On success the resolved entity is inserted into the peer cache so
    /// subsequent lookups are free.
    pub(crate) async fn fetch_by_id_rpc(
        &self,
        peer: tl::enums::Peer,
    ) -> Result<tl::enums::Peer, InvocationError> {
        match &peer {
            tl::enums::Peer::Chat(_) => {
                // Basic groups need no access_hash; always resolvable from ID.
                Ok(peer)
            }

            tl::enums::Peer::User(u) => {
                let req = tl::functions::users::GetUsers {
                    id: vec![tl::enums::InputUser::InputUser(tl::types::InputUser {
                        user_id: u.user_id,
                        access_hash: 0,
                    })],
                };
                let body: Vec<u8> = self.rpc_call_raw(&req).await?;
                let mut cur = Cursor::from_slice(&body);
                let users = Vec::<tl::enums::User>::deserialize(&mut cur)?;
                // Filter out UserEmpty responses
                let valid: Vec<_> = users
                    .into_iter()
                    .filter(|u| matches!(u, tl::enums::User::User(_)))
                    .collect();
                if !valid.is_empty() {
                    self.cache_users_slice(&valid).await;
                    return Ok(peer);
                }

                // Fallback: messages.getPeerDialogs (finds peers you've interacted with)
                let cache_read: tokio::sync::RwLockReadGuard<'_, crate::PeerCache> =
                    self.inner.peer_cache.read().await;
                if cache_read.users.contains_key(&match &peer {
                    tl::enums::Peer::User(u) => u.user_id,
                    _ => unreachable!(),
                }) {
                    drop(cache_read);
                    return Ok(peer);
                }
                drop(cache_read);

                let uid = match &peer {
                    tl::enums::Peer::User(u) => u.user_id,
                    _ => unreachable!(),
                };
                let req2 = tl::functions::messages::GetPeerDialogs {
                    peers: vec![tl::enums::InputDialogPeer::InputDialogPeer(
                        tl::types::InputDialogPeer {
                            peer: tl::enums::InputPeer::User(tl::types::InputPeerUser {
                                user_id: uid,
                                access_hash: 0,
                            }),
                        },
                    )],
                };
                let body2 = self.rpc_call_raw(&req2).await;
                match body2 {
                    Ok(b) => {
                        let mut cur2 = Cursor::from_slice(&b);
                        if let Ok(tl::enums::messages::PeerDialogs::PeerDialogs(pd)) =
                            tl::enums::messages::PeerDialogs::deserialize(&mut cur2)
                        {
                            self.cache_users_and_chats(&pd.users, &pd.chats).await;
                        }
                        Ok(peer)
                    }
                    Err(e) => Err(e),
                }
            }

            tl::enums::Peer::Channel(c) => {
                let req = tl::functions::channels::GetChannels {
                    id: vec![tl::enums::InputChannel::InputChannel(
                        tl::types::InputChannel {
                            channel_id: c.channel_id,
                            access_hash: 0,
                        },
                    )],
                };
                let body: Vec<u8> = self.rpc_call_raw(&req).await?;
                let mut cur = Cursor::from_slice(&body);
                let chats = tl::enums::messages::Chats::deserialize(&mut cur)?;
                let chats_vec = match chats {
                    tl::enums::messages::Chats::Chats(c) => c.chats,
                    tl::enums::messages::Chats::Slice(c) => c.chats,
                };
                let non_empty: Vec<_> = chats_vec
                    .into_iter()
                    .filter(|ch| !matches!(ch, tl::enums::Chat::Empty(_)))
                    .collect();
                if !non_empty.is_empty() {
                    self.cache_chats_slice(&non_empty).await;
                    return Ok(peer);
                }

                // Fallback: getPeerDialogs
                let cid = c.channel_id;
                let req2 = tl::functions::messages::GetPeerDialogs {
                    peers: vec![tl::enums::InputDialogPeer::InputDialogPeer(
                        tl::types::InputDialogPeer {
                            peer: tl::enums::InputPeer::Channel(tl::types::InputPeerChannel {
                                channel_id: cid,
                                access_hash: 0,
                            }),
                        },
                    )],
                };
                let body2 = self.rpc_call_raw(&req2).await;
                match body2 {
                    Ok(b) => {
                        let mut cur2 = Cursor::from_slice(&b);
                        if let Ok(tl::enums::messages::PeerDialogs::PeerDialogs(pd)) =
                            tl::enums::messages::PeerDialogs::deserialize(&mut cur2)
                        {
                            self.cache_users_and_chats(&pd.users, &pd.chats).await;
                        }
                        Ok(peer)
                    }
                    Err(e) => Err(e),
                }
            }
        }
    }

    /// `contacts.importContacts` RPC for phone-based resolution.
    ///
    /// Imports the phone as a temporary contact, caches the returned user, and
    /// returns the resolved Peer.
    pub(crate) async fn resolve_phone_rpc(
        &self,
        phone: &str,
    ) -> Result<tl::enums::Peer, InvocationError> {
        let req = tl::functions::contacts::ImportContacts {
            contacts: vec![tl::enums::InputContact::InputPhoneContact(
                tl::types::InputPhoneContact {
                    client_id: 0,
                    phone: phone.to_string(),
                    first_name: String::new(),
                    last_name: String::new(),
                    note: None,
                },
            )],
        };
        let body: Vec<u8> = self.rpc_call_raw(&req).await?;
        let mut cur = Cursor::from_slice(&body);
        let tl::enums::contacts::ImportedContacts::ImportedContacts(result) =
            tl::enums::contacts::ImportedContacts::deserialize(&mut cur)?;
        self.cache_users_slice(&result.users).await;

        // Check if the phone is now in the cache reverse index
        {
            let cache: tokio::sync::RwLockReadGuard<'_, PeerCache> =
                self.inner.peer_cache.read().await;
            if let Some(&uid) = cache.phone_to_user.get(phone) {
                return Ok(tl::enums::Peer::User(tl::types::PeerUser { user_id: uid }));
            }
        }

        // Fall back: first imported contact's user_id
        result
            .imported
            .first()
            .map(|imp| match imp {
                tl::enums::ImportedContact::ImportedContact(c) => {
                    Ok(tl::enums::Peer::User(tl::types::PeerUser {
                        user_id: c.user_id,
                    }))
                }
            })
            .unwrap_or_else(|| {
                Err(InvocationError::Deserialize(format!(
                    "phone {phone} not found on Telegram"
                )))
            })
    }

    /// `messages.checkChatInvite`; resolves an invite hash to a Peer.
    ///
    /// Succeeds only when you are already a member (`chatInviteAlready` or
    /// `chatInvitePeek`).  Use [`Client::join_by_invite`] to join first.
    pub(crate) async fn resolve_invite_hash_rpc(
        &self,
        hash: &str,
    ) -> Result<tl::enums::Peer, InvocationError> {
        let req = tl::functions::messages::CheckChatInvite {
            hash: hash.to_string(),
        };
        let body: Vec<u8> = self.rpc_call_raw(&req).await?;
        let mut cur = Cursor::from_slice(&body);
        let invite = tl::enums::ChatInvite::deserialize(&mut cur)?;

        match invite {
            tl::enums::ChatInvite::Already(a) => {
                let peer = chat_to_peer(&a.chat);
                self.cache_chats_slice(&[a.chat]).await;
                peer.ok_or_else(|| {
                    InvocationError::Deserialize(
                        "chatInviteAlready: unrecognised chat variant".into(),
                    )
                })
            }
            tl::enums::ChatInvite::Peek(p) => {
                let peer = chat_to_peer(&p.chat);
                self.cache_chats_slice(&[p.chat]).await;
                peer.ok_or_else(|| {
                    InvocationError::Deserialize("chatInvitePeek: unrecognised chat variant".into())
                })
            }
            tl::enums::ChatInvite::ChatInvite(_) => Err(InvocationError::Deserialize(
                "not a member of this chat yet; call client.join_by_invite() first".into(),
            )),
        }
    }

    /// Join a chat by invite link and return its `InputPeer`.
    ///
    /// Calls `messages.importChatInvite`, caches all returned entities, and
    /// returns the `InputPeer` of the joined chat.
    /// Join a chat or channel via an invite link.
    pub async fn join_link(&self, link: &str) -> Result<tl::enums::InputPeer, InvocationError> {
        let hash = PeerRef::parse_invite_hash(link)
            .ok_or_else(|| InvocationError::Deserialize(format!("invalid invite link: {link}")))?;
        let req = tl::functions::messages::ImportChatInvite {
            hash: hash.to_string(),
        };
        let body: Vec<u8> = self.rpc_call_raw(&req).await?;
        let mut cur = Cursor::from_slice(&body);
        let updates = tl::enums::Updates::deserialize(&mut cur)?;

        // Extract users and chats embedded in the Updates object
        let (users, chats) = updates_entities(&updates);
        self.cache_users_and_chats(&users, &chats).await;

        // Return the InputPeer of the first chat from the updates
        let cache: tokio::sync::RwLockReadGuard<'_, PeerCache> = self.inner.peer_cache.read().await;
        for chat in &chats {
            match chat {
                tl::enums::Chat::Channel(c) if !c.min => {
                    if let Some(&(hash, _)) = cache.channels.get(&c.id) {
                        return Ok(tl::enums::InputPeer::Channel(tl::types::InputPeerChannel {
                            channel_id: c.id,
                            access_hash: hash,
                        }));
                    }
                }
                tl::enums::Chat::Chat(c) => {
                    return Ok(tl::enums::InputPeer::Chat(tl::types::InputPeerChat {
                        chat_id: c.id,
                    }));
                }
                _ => {}
            }
        }

        Err(InvocationError::Deserialize(
            "importChatInvite: no chat returned".into(),
        ))
    }

    /// Peek at an invite link without joining.
    ///
    /// Returns the title and participant count of the chat the link points to.
    pub async fn check_invite(&self, link: &str) -> Result<tl::enums::ChatInvite, InvocationError> {
        let hash = PeerRef::parse_invite_hash(link)
            .ok_or_else(|| InvocationError::Deserialize(format!("invalid invite link: {link}")))?;
        let req = tl::functions::messages::CheckChatInvite {
            hash: hash.to_string(),
        };
        let body: Vec<u8> = self.rpc_call_raw(&req).await?;
        let mut cur = Cursor::from_slice(&body);
        Ok(tl::enums::ChatInvite::deserialize(&mut cur)?)
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
                        "[ferogram] rpc_call_raw: {} - evicting stale peer from cache",
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
            Ok(result) => result.map(|_| ()),
            Err(_) => Err(InvocationError::Deserialize(
                "rpc_write channel closed (sender task died?)".into(),
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
                "[ferogram] initConnection ✓  ({} DCs, ipv6={})",
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
        tracing::info!("[ferogram] Migrating to DC{new_dc_id} ({addr}) …");

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

    /// Mark the session snapshot as dirty so the periodic full-snapshot saver
    /// knows a flush is needed on the next interval tick.
    #[inline]
    fn mark_session_snapshot_dirty(&self) {
        self.inner
            .session_snapshot_dirty
            .store(true, std::sync::atomic::Ordering::Release);
    }

    /// Sync the internal pts/qts/seq/date state with the Telegram server.
    ///
    /// Called automatically on `connect()`. Call it manually if you
    /// need to reset the update gap-detection counters, e.g. after resuming
    /// from a long hibernation.
    async fn cache_user(&self, user: &tl::enums::User) {
        self.remember_self_id(user);
        self.inner.peer_cache.write().await.cache_user(user);
        self.mark_session_snapshot_dirty();
    }

    /// If `user` is a `User::User` with `is_self == true`, cache its numeric ID
    /// as the logged-in account's own ID (see `Inner::self_user_id`).
    ///
    /// Telegram sets this flag on the account's own `User` object wherever it
    /// appears (sign-in response, `users.getUsers(InputUser::UserSelf)`,
    /// contact lists, etc.), so this captures it for free the first time any
    /// such object is cached - no dedicated network round-trip required.
    fn remember_self_id(&self, user: &tl::enums::User) {
        if let tl::enums::User::User(u) = user
            && u.is_self
        {
            self.inner
                .self_user_id
                .store(u.id, std::sync::atomic::Ordering::Relaxed);
        }
    }

    pub(crate) async fn cache_users_slice(&self, users: &[tl::enums::User]) {
        for u in users {
            self.remember_self_id(u);
        }
        let mut cache: tokio::sync::RwLockWriteGuard<'_, PeerCache> =
            self.inner.peer_cache.write().await;
        cache.cache_users(users);
        drop(cache);
        self.mark_session_snapshot_dirty();
    }

    pub(crate) async fn cache_chats_slice(&self, chats: &[tl::enums::Chat]) {
        let mut cache: tokio::sync::RwLockWriteGuard<'_, PeerCache> =
            self.inner.peer_cache.write().await;
        cache.cache_chats(chats);
        drop(cache);
        self.mark_session_snapshot_dirty();
    }

    /// Cache users and chats in a single write-lock acquisition.
    pub(crate) async fn cache_users_and_chats(
        &self,
        users: &[tl::enums::User],
        chats: &[tl::enums::Chat],
    ) {
        let mut cache: tokio::sync::RwLockWriteGuard<'_, PeerCache> =
            self.inner.peer_cache.write().await;
        cache.cache_users(users);
        cache.cache_chats(chats);
        drop(cache);
        self.mark_session_snapshot_dirty();
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
                tracing::debug!("[ferogram] bulk_peer_hydration: cooldown active; skipping");
                return;
            }
        }
        *self.inner.last_bulk_hydration.lock() = Some(std::time::Instant::now());
        tracing::info!("[ferogram] peer cache miss burst; running bulk dialog hydration");
        if let Err(e) = self.warm_peer_cache_from_dialogs().await {
            tracing::warn!("[ferogram] bulk_peer_hydration: getDialogs failed: {e}");
        }
    }

    /// Look up the cached [`ChannelKind`] for a raw channel ID.
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
            "[ferogram] warm_peer_cache_from_dialogs: body len={} ctor=0x{:08x}",
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
                    "[ferogram] warm_peer_cache_from_dialogs: dialogs={}, users={}, chats={}",
                    d.dialogs.len(),
                    d.users.len(),
                    d.chats.len()
                );
                self.cache_chats_slice(&d.chats).await;
                self.cache_users_slice(&d.users).await;
            }
            tl::enums::messages::Dialogs::Slice(d) => {
                tracing::debug!(
                    "[ferogram] warm_peer_cache_from_dialogs: slice_count={}, dialogs={}, users={}, chats={}",
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
                    "[ferogram] warm_peer_cache_from_dialogs: GetDialogs returned NotModified"
                );
            }
        }
        tracing::debug!(
            "[ferogram] warm_peer_cache_from_dialogs: peer cache refreshed from GetDialogs"
        );
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
                    "[ferogram] Transfer: home auth key reuse for DC{target_dc} (home={home})"
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
                        "[ferogram] Transfer initConnection for DC{target_dc} failed: {e}  - evicting"
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
                        "[ferogram] Transfer: cached key for foreign DC{target_dc}  - still need importAuth"
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
                            "[ferogram] Transfer importAuth (cached key) DC{target_dc} failed: {e}  - evicting"
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
                        "[ferogram] Transfer: fresh DH for DC{target_dc} (home={home})"
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
                            "[ferogram] Transfer auth export/import DC{target_dc} failed: {e}  - evicting"
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
            ) {
                tracing::warn!(
                    "[ferogram] Transfer DC{target_dc} auth error ({})  - evicting and clearing cached key",
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
        tracing::debug!("[ferogram] Transfer initConnection for DC{dc_id} ✓");
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
        tracing::debug!("[ferogram] Transfer initConnection+importAuth to DC{dc_id} ✓");
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
            tracing::debug!("[ferogram] worker conn to DC{target_dc} (home key) ready");
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
                    "[ferogram] worker conn DC{target_dc} (foreign, cached key)  - skipping DH+export"
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
                        "[ferogram] worker conn to DC{target_dc} (foreign, cached key, auth re-imported) ready"
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
                        "[ferogram] worker conn to DC{target_dc} (foreign, cached key, auth already imported) ready"
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
                tracing::debug!(
                    "[ferogram] worker conn to DC{target_dc} (foreign, fresh DH) ready"
                );
                Ok(conn)
            }
        }
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
                                "[ferogram] invoke_on_dc: DC{dc_id} (cached key, auth re-imported) ready"
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
                                "[ferogram] invoke_on_dc: DC{dc_id} (cached key, already imported) ready"
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
                            "[ferogram] invoke_on_dc: DC{dc_id} (fresh DH, auth imported) ready"
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

    pub async fn answer_precheckout_query(
        &self,
        query_id: i64,
        ok: bool,
        error_message: Option<String>,
    ) -> Result<(), InvocationError> {
        let req = tl::functions::messages::SetBotPrecheckoutResults {
            success: ok,
            query_id,
            error: error_message,
        };
        self.rpc_write(&req).await
    }

    pub async fn answer_shipping_query(
        &self,
        query_id: i64,
        error: Option<String>,
        shipping_options: Option<Vec<tl::enums::ShippingOption>>,
    ) -> Result<(), InvocationError> {
        let req = tl::functions::messages::SetBotShippingResults {
            query_id,
            error,
            shipping_options,
        };
        self.rpc_write(&req).await
    }

    pub async fn start_bot(
        &self,
        bot_user_id: i64,
        peer: impl Into<PeerRef>,
        start_param: impl Into<String>,
    ) -> Result<(), InvocationError> {
        let bot_hash = self
            .inner
            .peer_cache
            .read()
            .await
            .users
            .get(&bot_user_id)
            .copied()
            .unwrap_or(0);
        let peer = peer.into().resolve(self).await?;
        let input_peer = self.inner.peer_cache.read().await.peer_to_input(&peer)?;
        let req = tl::functions::messages::StartBot {
            bot: tl::enums::InputUser::InputUser(tl::types::InputUser {
                user_id: bot_user_id,
                access_hash: bot_hash,
            }),
            peer: input_peer,
            random_id: random_i64(),
            start_param: start_param.into(),
        };
        self.rpc_write(&req).await
    }

    pub async fn set_game_score(
        &self,
        peer: impl Into<PeerRef>,
        msg_id: i32,
        user_id: i64,
        score: i32,
        force: bool,
        edit_message: bool,
    ) -> Result<(), InvocationError> {
        let peer = peer.into().resolve(self).await?;
        let input_peer = self.inner.peer_cache.read().await.peer_to_input(&peer)?;
        let user_hash = self
            .inner
            .peer_cache
            .read()
            .await
            .users
            .get(&user_id)
            .copied()
            .unwrap_or(0);
        let req = tl::functions::messages::SetGameScore {
            edit_message,
            force,
            peer: input_peer,
            id: msg_id,
            user_id: tl::enums::InputUser::InputUser(tl::types::InputUser {
                user_id,
                access_hash: user_hash,
            }),
            score,
        };
        self.rpc_write(&req).await
    }

    pub async fn get_game_high_scores(
        &self,
        peer: impl Into<PeerRef>,
        msg_id: i32,
        user_id: i64,
    ) -> Result<Vec<tl::types::HighScore>, InvocationError> {
        let peer = peer.into().resolve(self).await?;
        let input_peer = self.inner.peer_cache.read().await.peer_to_input(&peer)?;
        let user_hash = self
            .inner
            .peer_cache
            .read()
            .await
            .users
            .get(&user_id)
            .copied()
            .unwrap_or(0);
        let req = tl::functions::messages::GetGameHighScores {
            peer: input_peer,
            id: msg_id,
            user_id: tl::enums::InputUser::InputUser(tl::types::InputUser {
                user_id,
                access_hash: user_hash,
            }),
        };
        let body: Vec<u8> = self.rpc_call_raw(&req).await?;
        let mut cur = Cursor::from_slice(&body);
        let tl::enums::messages::HighScores::HighScores(result) =
            tl::enums::messages::HighScores::deserialize(&mut cur)?;
        self.cache_users_slice(&result.users).await;
        Ok(result
            .scores
            .into_iter()
            .map(|s| match s {
                tl::enums::HighScore::HighScore(h) => h,
            })
            .collect())
    }

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
            rich_message: None,
        };
        let body: Vec<u8> = self.rpc_call_raw(&req).await?;
        Ok(body.len() >= 4 && u32::from_le_bytes(body[..4].try_into().unwrap()) == 0x997275b5)
    }

    /// Send a dice/dart/basketball/etc animated emoji and return the sent message.
    ///
    /// The rolled value is in the returned message's media:
    ///
    /// ```rust,no_run
    /// # use ferogram::{Client, tl};
    /// # async fn example(client: Client) -> anyhow::Result<()> {
    /// let msg = client.send_dice(123456789, "🎲").await?;
    /// if let Some(tl::enums::MessageMedia::Dice(d)) = msg.media() {
    ///     println!("rolled {}", d.value);
    /// }
    /// # Ok(()) }
    /// ```
    pub async fn send_dice(
        &self,
        peer: impl Into<PeerRef>,
        emoticon: impl Into<String>,
    ) -> Result<update::IncomingMessage, InvocationError> {
        use ferogram_tl_types as tl;
        let peer = peer.into().resolve(self).await?;
        let input_peer = self.inner.peer_cache.read().await.peer_to_input(&peer)?;
        let media = tl::enums::InputMedia::Dice(tl::types::InputMediaDice {
            emoticon: emoticon.into(),
        });
        let req = tl::functions::messages::SendMedia {
            silent: false,
            background: false,
            clear_draft: false,
            noforwards: false,
            update_stickersets_order: false,
            invert_media: false,
            allow_paid_floodskip: false,
            peer: input_peer,
            reply_to: None,
            media,
            message: String::new(),
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
        let body: Vec<u8> = self.rpc_call_raw(&req).await?;
        Ok(self
            .parse_send_response(&body, &InputMessage::text(""), &peer)
            .await)
    }

    pub async fn send_poll(
        &self,
        peer: impl Into<PeerRef>,
        poll: crate::poll::PollBuilder,
    ) -> Result<(), InvocationError> {
        let peer = peer.into().resolve(self).await?;
        let input_peer = self.inner.peer_cache.read().await.peer_to_input(&peer)?;
        let media = poll.into_input_media();
        let req = tl::functions::messages::SendMedia {
            silent: false,
            background: false,
            clear_draft: false,
            noforwards: false,
            update_stickersets_order: false,
            invert_media: false,
            allow_paid_floodskip: false,
            peer: input_peer,
            reply_to: None,
            media,
            message: String::new(),
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
        self.rpc_call_raw(&req).await?;
        Ok(())
    }

    pub async fn set_bot_commands(
        &self,
        commands: &[(&str, &str)],
        scope: Option<tl::enums::BotCommandScope>,
        lang_code: &str,
    ) -> Result<bool, InvocationError> {
        let bot_commands: Vec<tl::enums::BotCommand> = commands
            .iter()
            .map(|(cmd, desc)| {
                tl::enums::BotCommand::BotCommand(tl::types::BotCommand {
                    command: cmd.to_string(),
                    description: desc.to_string(),
                })
            })
            .collect();
        let req = tl::functions::bots::SetBotCommands {
            scope: scope.unwrap_or(tl::enums::BotCommandScope::Default),
            lang_code: lang_code.to_string(),
            commands: bot_commands,
        };
        let body = self.rpc_call_raw(&req).await?;
        Ok(is_bool_true(&body))
    }

    pub async fn delete_bot_commands(
        &self,
        scope: Option<tl::enums::BotCommandScope>,
        lang_code: &str,
    ) -> Result<bool, InvocationError> {
        let req = tl::functions::bots::ResetBotCommands {
            scope: scope.unwrap_or(tl::enums::BotCommandScope::Default),
            lang_code: lang_code.to_string(),
        };
        let body = self.rpc_call_raw(&req).await?;
        Ok(is_bool_true(&body))
    }

    /// Set bot profile info.
    ///
    /// `bot`: pass the bot's peer when calling from a userbot that owns the
    /// bot. Pass `None` when calling from the bot session itself.
    ///
    /// All text fields are optional; only the ones you supply are changed.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # async fn f(client: ferogram::Client) -> Result<(), Box<dyn std::error::Error>> {
    /// // From a bot session: edit self.
    /// client.set_bot_info(None::<&str>, Some("My Bot"), Some("Short about."), Some("Start page text."), "en").await?;
    ///
    /// // From a userbot: edit an owned bot.
    /// client.set_bot_info(Some("@MyBot"), Some("My Bot"), None, None, "en").await?;
    /// # Ok(()) }
    /// ```
    pub async fn set_bot_info(
        &self,
        bot: Option<impl Into<PeerRef>>,
        name: Option<&str>,
        about: Option<&str>,
        description: Option<&str>,
        lang_code: &str,
    ) -> Result<bool, InvocationError> {
        let bot_input = if let Some(peer) = bot {
            let resolved = peer.into().resolve(self).await?;
            let input_peer = self
                .inner
                .peer_cache
                .read()
                .await
                .peer_to_input(&resolved)?;
            let input_user = match input_peer {
                tl::enums::InputPeer::User(u) => {
                    tl::enums::InputUser::InputUser(tl::types::InputUser {
                        user_id: u.user_id,
                        access_hash: u.access_hash,
                    })
                }
                tl::enums::InputPeer::PeerSelf => tl::enums::InputUser::UserSelf,
                _ => {
                    return Err(InvocationError::Deserialize(
                        "peer must resolve to a user (bot)".into(),
                    ));
                }
            };
            Some(input_user)
        } else {
            None
        };
        let req = tl::functions::bots::SetBotInfo {
            bot: bot_input,
            lang_code: lang_code.to_string(),
            name: name.map(|s| s.to_string()),
            about: about.map(|s| s.to_string()),
            description: description.map(|s| s.to_string()),
        };
        let body = self.rpc_call_raw(&req).await?;
        Ok(is_bool_true(&body))
    }

    /// Get bot profile info.
    ///
    /// `bot`: pass the bot's peer when calling from a userbot. Pass `None`
    /// when calling from the bot session itself.
    pub async fn get_bot_info(
        &self,
        bot: Option<impl Into<PeerRef>>,
        lang_code: &str,
    ) -> Result<tl::types::bots::BotInfo, InvocationError> {
        use ferogram_tl_types::{Cursor, Deserializable};
        let bot_input = if let Some(peer) = bot {
            let resolved = peer.into().resolve(self).await?;
            let input_peer = self
                .inner
                .peer_cache
                .read()
                .await
                .peer_to_input(&resolved)?;
            let input_user = match input_peer {
                tl::enums::InputPeer::User(u) => {
                    tl::enums::InputUser::InputUser(tl::types::InputUser {
                        user_id: u.user_id,
                        access_hash: u.access_hash,
                    })
                }
                tl::enums::InputPeer::PeerSelf => tl::enums::InputUser::UserSelf,
                _ => {
                    return Err(InvocationError::Deserialize(
                        "peer must resolve to a user (bot)".into(),
                    ));
                }
            };
            Some(input_user)
        } else {
            None
        };
        let req = tl::functions::bots::GetBotInfo {
            bot: bot_input,
            lang_code: lang_code.to_string(),
        };
        let body = self.rpc_call_raw(&req).await?;
        let mut cur = Cursor::from_slice(&body);
        let tl::enums::bots::BotInfo::BotInfo(result) =
            tl::enums::bots::BotInfo::deserialize(&mut cur)?;
        Ok(result)
    }

    pub async fn send_invoice(
        &self,
        peer: impl Into<crate::PeerRef>,
        title: impl Into<String>,
        description: impl Into<String>,
        payload: impl Into<String>,
        options: crate::InvoiceOptions,
    ) -> Result<crate::update::IncomingMessage, InvocationError> {
        let peer = peer.into().resolve(self).await?;
        let input_peer = self.inner.peer_cache.read().await.peer_to_input(&peer)?;

        let label_prices: Vec<tl::enums::LabeledPrice> = options
            .prices
            .iter()
            .map(|(label, amount)| {
                tl::enums::LabeledPrice::LabeledPrice(tl::types::LabeledPrice {
                    label: label.clone(),
                    amount: *amount,
                })
            })
            .collect();

        let invoice = tl::enums::Invoice::Invoice(tl::types::Invoice {
            test: false,
            name_requested: options.need_name,
            phone_requested: options.need_phone,
            email_requested: options.need_email,
            shipping_address_requested: options.need_shipping_address,
            flexible: options.is_flexible,
            phone_to_provider: false,
            email_to_provider: false,
            recurring: false,
            currency: options.currency.clone(),
            prices: label_prices,
            max_tip_amount: None,
            suggested_tip_amounts: None,
            terms_url: None,
            subscription_period: None,
        });

        let media = tl::enums::InputMedia::Invoice(Box::new(tl::types::InputMediaInvoice {
            title: title.into(),
            description: description.into(),
            photo: options.photo_url.map(|url| {
                tl::enums::InputWebDocument::InputWebDocument(tl::types::InputWebDocument {
                    url,
                    size: 0,
                    mime_type: "image/jpeg".into(),
                    attributes: vec![],
                })
            }),
            invoice,
            payload: payload.into().into_bytes(),
            provider: None,
            provider_data: tl::enums::DataJson::DataJson(tl::types::DataJson { data: "{}".into() }),
            start_param: None,
            extended_media: None,
        }));

        let req = tl::functions::messages::SendMedia {
            silent: false,
            background: false,
            clear_draft: false,
            noforwards: false,
            update_stickersets_order: false,
            invert_media: false,
            allow_paid_floodskip: false,
            peer: input_peer,
            reply_to: None,
            media,
            message: String::new(),
            random_id: crate::random_i64_pub(),
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
        Ok(self
            .parse_send_response(&body, &crate::InputMessage::text(""), &peer)
            .await)
    }

    #[allow(dead_code)]
    async fn set_default_banned_rights_raw(
        &self,
        peer: impl Into<PeerRef>,
        build: impl FnOnce(
            crate::participants::BannedRightsBuilder,
        ) -> crate::participants::BannedRightsBuilder,
    ) -> Result<(), InvocationError> {
        let peer = peer.into().resolve(self).await?;
        let input_peer = self.inner.peer_cache.read().await.peer_to_input(&peer)?;
        let rights = build(crate::participants::BannedRightsBuilder::new()).into_tl();
        let req = tl::functions::messages::EditChatDefaultBannedRights {
            peer: input_peer,
            banned_rights: rights,
        };
        self.rpc_write(&req).await
    }

    pub(crate) async fn get_dialogs_raw_with_count(
        &self,
        req: tl::functions::messages::GetDialogs,
    ) -> Result<(Vec<Dialog>, Option<i32>), InvocationError> {
        let body: Vec<u8> = self.rpc_call_raw(&req).await?;
        let mut cur = Cursor::from_slice(&body);
        let raw = tl::enums::messages::Dialogs::deserialize(&mut cur)?;
        let (dialogs_raw, messages, users, chats, count) = match raw {
            tl::enums::messages::Dialogs::Dialogs(d) => {
                (d.dialogs, d.messages, d.users, d.chats, None)
            }
            tl::enums::messages::Dialogs::Slice(d) => {
                (d.dialogs, d.messages, d.users, d.chats, Some(d.count))
            }
            tl::enums::messages::Dialogs::NotModified(d) => return Ok((vec![], Some(d.count))),
        };

        self.cache_users_and_chats(&users, &chats).await;

        let msg_map: std::collections::HashMap<i32, tl::enums::Message> = messages
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

        let user_map: std::collections::HashMap<i64, tl::enums::User> = users
            .into_iter()
            .filter_map(|u| {
                if let tl::enums::User::User(ref uu) = u {
                    Some((uu.id, u))
                } else {
                    None
                }
            })
            .collect();

        let chat_map: std::collections::HashMap<i64, tl::enums::Chat> = chats
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

        let dialogs: Vec<Dialog> = dialogs_raw
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

        Ok((dialogs, count))
    }

    #[allow(dead_code)]
    pub async fn get_chat_full(
        &self,
        peer: impl Into<PeerRef>,
    ) -> Result<tl::enums::messages::ChatFull, InvocationError> {
        let peer = peer.into().resolve(self).await?;
        let input_peer = self.inner.peer_cache.read().await.peer_to_input(&peer)?;
        let body = match &input_peer {
            tl::enums::InputPeer::Channel(c) => {
                let req = tl::functions::channels::GetFullChannel {
                    channel: tl::enums::InputChannel::InputChannel(tl::types::InputChannel {
                        channel_id: c.channel_id,
                        access_hash: c.access_hash,
                    }),
                };
                self.rpc_call_raw(&req).await?
            }
            tl::enums::InputPeer::Chat(c) => {
                let req = tl::functions::messages::GetFullChat { chat_id: c.chat_id };
                self.rpc_call_raw(&req).await?
            }
            _ => {
                return Err(InvocationError::Deserialize(
                    "get_chat_full: peer must be a chat or channel".into(),
                ));
            }
        };
        // Cache users/chats from the response so subsequent calls work.
        let mut cur = Cursor::from_slice(&body);
        let full = tl::enums::messages::ChatFull::deserialize(&mut cur)?;
        let tl::enums::messages::ChatFull::ChatFull(ref f) = full;
        self.cache_users_slice(&f.users).await;
        self.cache_chats_slice(&f.chats).await;
        Ok(full)
    }

    pub(crate) async fn get_messages_with_count(
        &self,
        peer: impl Into<PeerRef>,
        limit: i32,
        offset_id: i32,
    ) -> Result<(Vec<crate::update::IncomingMessage>, Option<i32>), InvocationError> {
        let peer = peer.into().resolve(self).await?;
        let input_peer = self.inner.peer_cache.read().await.peer_to_input(&peer)?;
        let req = tl::functions::messages::GetHistory {
            peer: input_peer,
            offset_id,
            offset_date: 0,
            add_offset: 0,
            limit,
            max_id: 0,
            min_id: 0,
            hash: 0,
        };
        let body: Vec<u8> = self.rpc_call_raw(&req).await?;
        let mut cur = Cursor::from_slice(&body);
        let raw = tl::enums::messages::Messages::deserialize(&mut cur)?;
        let (msgs, users, chats, count) = match raw {
            tl::enums::messages::Messages::Messages(m) => (m.messages, m.users, m.chats, None),
            tl::enums::messages::Messages::Slice(m) => {
                (m.messages, m.users, m.chats, Some(m.count))
            }
            tl::enums::messages::Messages::ChannelMessages(m) => {
                (m.messages, m.users, m.chats, None)
            }
            tl::enums::messages::Messages::NotModified(_) => return Ok((vec![], None)),
        };
        self.cache_users_and_chats(&users, &chats).await;
        let out = msgs
            .into_iter()
            .map(|m| crate::update::IncomingMessage::from_raw(m).with_client(self.clone()))
            .collect();
        Ok((out, count))
    }

    #[allow(dead_code)]
    pub(crate) async fn add_chat_members(
        &self,
        peer: impl Into<PeerRef>,
        user_ids: &[i64],
    ) -> Result<(), InvocationError> {
        if user_ids.is_empty() {
            return Ok(());
        }
        let peer = peer.into().resolve(self).await?;
        let input_peer = self.inner.peer_cache.read().await.peer_to_input(&peer)?;

        match &input_peer {
            tl::enums::InputPeer::Channel(c) => {
                let cache: tokio::sync::RwLockReadGuard<'_, PeerCache> =
                    self.inner.peer_cache.read().await;
                let users: Vec<tl::enums::InputUser> = user_ids
                    .iter()
                    .map(|&id| {
                        let hash = cache.users.get(&id).copied().unwrap_or(0);
                        tl::enums::InputUser::InputUser(tl::types::InputUser {
                            user_id: id,
                            access_hash: hash,
                        })
                    })
                    .collect();
                let req = tl::functions::channels::InviteToChannel {
                    channel: tl::enums::InputChannel::InputChannel(tl::types::InputChannel {
                        channel_id: c.channel_id,
                        access_hash: c.access_hash,
                    }),
                    users,
                };
                self.rpc_write(&req).await
            }
            tl::enums::InputPeer::Chat(c) => {
                // Legacy groups: add one at a time
                for &id in user_ids {
                    let hash = self
                        .inner
                        .peer_cache
                        .read()
                        .await
                        .users
                        .get(&id)
                        .copied()
                        .unwrap_or(0);
                    let req = tl::functions::messages::AddChatUser {
                        chat_id: c.chat_id,
                        user_id: tl::enums::InputUser::InputUser(tl::types::InputUser {
                            user_id: id,
                            access_hash: hash,
                        }),
                        fwd_limit: 0,
                    };
                    self.rpc_write(&req).await?;
                }
                Ok(())
            }
            _ => Err(InvocationError::Deserialize(
                "invite_users: peer must be a chat or channel".into(),
            )),
        }
    }

    pub async fn delete_chat_history(
        &self,
        peer: impl Into<PeerRef>,
        max_id: i32,
        revoke: bool,
    ) -> Result<(), InvocationError> {
        let peer = peer.into().resolve(self).await?;
        let input_peer = self.inner.peer_cache.read().await.peer_to_input(&peer)?;
        match &input_peer {
            tl::enums::InputPeer::Channel(c) => {
                let req = tl::functions::channels::DeleteHistory {
                    for_everyone: revoke,
                    channel: tl::enums::InputChannel::InputChannel(tl::types::InputChannel {
                        channel_id: c.channel_id,
                        access_hash: c.access_hash,
                    }),
                    max_id,
                };
                self.rpc_write(&req).await
            }
            _ => {
                // For regular chats the server may return an offset != 0, indicating
                // that more messages remain and we must call again.
                loop {
                    let req = tl::functions::messages::DeleteHistory {
                        just_clear: false,
                        revoke,
                        peer: input_peer.clone(),
                        max_id,
                        min_date: None,
                        max_date: None,
                    };
                    let body = self.rpc_call_raw(&req).await?;
                    let mut cur = Cursor::from_slice(&body);
                    let tl::enums::messages::AffectedHistory::AffectedHistory(result) =
                        tl::enums::messages::AffectedHistory::deserialize(&mut cur)?;
                    if result.offset == 0 {
                        break;
                    }
                }
                Ok(())
            }
        }
    }

    pub async fn save_draft(
        &self,
        peer: impl Into<PeerRef>,
        text: impl Into<String>,
    ) -> Result<(), InvocationError> {
        let peer = peer.into().resolve(self).await?;
        let input_peer = self.inner.peer_cache.read().await.peer_to_input(&peer)?;
        let req = tl::functions::messages::SaveDraft {
            no_webpage: false,
            invert_media: false,
            reply_to: None,
            peer: input_peer,
            message: text.into(),
            entities: None,
            media: None,
            effect: None,
            suggested_post: None,
            rich_message: None,
        };
        self.rpc_write(&req).await
    }

    pub async fn get_pinned_dialogs(
        &self,
        folder_id: i32,
    ) -> Result<Vec<tl::enums::Dialog>, InvocationError> {
        let req = tl::functions::messages::GetPinnedDialogs { folder_id };
        let body = self.rpc_call_raw(&req).await?;
        let mut cur = Cursor::from_slice(&body);
        let tl::enums::messages::PeerDialogs::PeerDialogs(result) =
            tl::enums::messages::PeerDialogs::deserialize(&mut cur)?;
        self.cache_users_slice(&result.users).await;
        self.cache_chats_slice(&result.chats).await;
        Ok(result.dialogs)
    }

    pub async fn mark_dialog_unread(
        &self,
        peer: impl Into<PeerRef>,
    ) -> Result<(), InvocationError> {
        self.set_dialog_unread_flag(peer, true).await
    }

    pub(crate) async fn set_dialog_unread_flag(
        &self,
        peer: impl Into<PeerRef>,
        unread: bool,
    ) -> Result<(), InvocationError> {
        let peer = peer.into().resolve(self).await?;
        let input_peer = self.inner.peer_cache.read().await.peer_to_input(&peer)?;
        let req = tl::functions::messages::MarkDialogUnread {
            unread,
            parent_peer: None,
            peer: tl::enums::InputDialogPeer::InputDialogPeer(tl::types::InputDialogPeer {
                peer: input_peer,
            }),
        };
        self.rpc_write(&req).await
    }

    #[allow(dead_code)]
    pub(crate) async fn download_media_to_file_on_dc(
        &self,
        location: tl::enums::InputFileLocation,
        dc_id: i32,
        path: impl AsRef<std::path::Path>,
    ) -> Result<(), InvocationError> {
        let mut file = tokio::fs::File::create(path)
            .await
            .map_err(|e| InvocationError::Deserialize(e.to_string()))?;
        // Get auth key for the target DC
        let opts: tokio::sync::MutexGuard<'_, std::collections::HashMap<i32, crate::DcEntry>> =
            self.inner.dc_options.lock().await;
        let entry = opts.get(&dc_id).cloned();
        drop(opts);
        let (addr, auth_key, first_salt) = match entry {
            Some(e) if e.auth_key.is_some() => (e.addr.clone(), e.auth_key.unwrap(), e.first_salt),
            _ => {
                return Err(InvocationError::Deserialize(format!(
                    "download_media: no auth key for DC {dc_id}"
                )));
            }
        };
        let _conn = ferogram_connect::Connection::connect_with_key(
            &addr,
            auth_key,
            first_salt,
            0,
            None,
            None,
            &ferogram_connect::TransportKind::Abridged,
            dc_id as i16,
            false,
        )
        .await
        .map_err(|e| InvocationError::Deserialize(e.to_string()))?;
        let _enc = ferogram_mtproto::EncryptedSession::new(auth_key, first_salt, 0);
        let req = tl::functions::upload::GetFile {
            precise: false,
            cdn_supported: false,
            location,
            offset: 0,
            limit: 1024 * 1024,
        };
        let body: Vec<u8> = self.rpc_call_raw(&req).await?;
        let mut cur = ferogram_tl_types::Cursor::from_slice(&body);
        use ferogram_tl_types::Deserializable;
        match tl::enums::upload::File::deserialize(&mut cur)? {
            tl::enums::upload::File::File(f) => {
                file.write_all(&f.bytes)
                    .await
                    .map_err(|e| InvocationError::Deserialize(e.to_string()))?;
            }
            tl::enums::upload::File::CdnRedirect(_) => {
                return Err(InvocationError::Deserialize(
                    "CDN redirect not supported".into(),
                ));
            }
        }
        Ok(())
    }

    pub async fn upload_media(
        &self,
        peer: impl Into<PeerRef>,
        media: tl::enums::InputMedia,
    ) -> Result<tl::enums::MessageMedia, InvocationError> {
        let peer = peer.into().resolve(self).await?;
        let input_peer = self.inner.peer_cache.read().await.peer_to_input(&peer)?;
        let req = tl::functions::messages::UploadMedia {
            business_connection_id: None,
            peer: input_peer,
            media,
        };
        let body = self.rpc_call_raw(&req).await?;
        let mut cur = Cursor::from_slice(&body);
        Ok(tl::enums::MessageMedia::deserialize(&mut cur)?)
    }

    pub async fn get_media_group(
        &self,
        peer: impl Into<PeerRef>,
        msg_id: i32,
    ) -> Result<Vec<update::IncomingMessage>, InvocationError> {
        use ferogram_tl_types as tl;
        let peer = peer.into().resolve(self).await?;
        let input_peer = self.inner.peer_cache.read().await.peer_to_input(&peer)?;

        // Fetch the seed message first to get grouped_id
        let seed_ids = vec![tl::enums::InputMessage::Id(tl::types::InputMessageId {
            id: msg_id,
        })];

        let seed_msgs = match &input_peer {
            tl::enums::InputPeer::Channel(c) => {
                let req = tl::functions::channels::GetMessages {
                    channel: tl::enums::InputChannel::InputChannel(tl::types::InputChannel {
                        channel_id: c.channel_id,
                        access_hash: c.access_hash,
                    }),
                    id: seed_ids,
                };
                let body = self.rpc_call_raw(&req).await?;
                let mut cur = Cursor::from_slice(&body);
                match tl::enums::messages::Messages::deserialize(&mut cur)? {
                    tl::enums::messages::Messages::Messages(m) => m.messages,
                    tl::enums::messages::Messages::Slice(m) => m.messages,
                    tl::enums::messages::Messages::ChannelMessages(m) => m.messages,
                    tl::enums::messages::Messages::NotModified(_) => vec![],
                }
            }
            _ => {
                let req = tl::functions::messages::GetMessages { id: seed_ids };
                let body = self.rpc_call_raw(&req).await?;
                let mut cur = Cursor::from_slice(&body);
                match tl::enums::messages::Messages::deserialize(&mut cur)? {
                    tl::enums::messages::Messages::Messages(m) => m.messages,
                    tl::enums::messages::Messages::Slice(m) => m.messages,
                    tl::enums::messages::Messages::ChannelMessages(m) => m.messages,
                    tl::enums::messages::Messages::NotModified(_) => vec![],
                }
            }
        };

        // Extract grouped_id from the seed message
        let grouped_id = seed_msgs.iter().find_map(|m| {
            if let tl::enums::Message::Message(msg) = m {
                msg.grouped_id
            } else {
                None
            }
        });

        // If there's no grouped_id, just return the single message
        let Some(gid) = grouped_id else {
            return Ok(seed_msgs
                .into_iter()
                .map(update::IncomingMessage::from_raw)
                .collect());
        };

        // Fetch a window of messages around msg_id to find all members of the group
        // Albums are always contiguous so a window of ±10 is more than enough
        let window_start = (msg_id - 9).max(1);
        let window_ids: Vec<tl::enums::InputMessage> = (window_start..=msg_id + 9)
            .map(|id| tl::enums::InputMessage::Id(tl::types::InputMessageId { id }))
            .collect();

        let window_msgs = match &input_peer {
            tl::enums::InputPeer::Channel(c) => {
                let req = tl::functions::channels::GetMessages {
                    channel: tl::enums::InputChannel::InputChannel(tl::types::InputChannel {
                        channel_id: c.channel_id,
                        access_hash: c.access_hash,
                    }),
                    id: window_ids,
                };
                let body = self.rpc_call_raw(&req).await?;
                let mut cur = Cursor::from_slice(&body);
                match tl::enums::messages::Messages::deserialize(&mut cur)? {
                    tl::enums::messages::Messages::Messages(m) => m.messages,
                    tl::enums::messages::Messages::Slice(m) => m.messages,
                    tl::enums::messages::Messages::ChannelMessages(m) => m.messages,
                    tl::enums::messages::Messages::NotModified(_) => vec![],
                }
            }
            _ => seed_msgs,
        };

        let group: Vec<update::IncomingMessage> = window_msgs
            .into_iter()
            .filter(|m| {
                if let tl::enums::Message::Message(msg) = m {
                    msg.grouped_id == Some(gid)
                } else {
                    false
                }
            })
            .map(update::IncomingMessage::from_raw)
            .collect();

        Ok(group)
    }

    pub async fn create_group(
        &self,
        title: impl Into<String>,
        user_ids: Vec<i64>,
    ) -> Result<tl::enums::Chat, InvocationError> {
        let cache = self.inner.peer_cache.read().await;
        let users: Vec<tl::enums::InputUser> = user_ids
            .into_iter()
            .map(|id| {
                let hash = cache.users.get(&id).copied().unwrap_or(0);
                tl::enums::InputUser::InputUser(tl::types::InputUser {
                    user_id: id,
                    access_hash: hash,
                })
            })
            .collect();

        let req = tl::functions::messages::CreateChat {
            users,
            title: title.into(),
            ttl_period: None,
        };
        let body = self.rpc_call_raw(&req).await?;
        let mut cur = Cursor::from_slice(&body);
        let updates = tl::enums::Updates::deserialize(&mut cur)?;
        // Extract the chat from updates
        let chats = match updates {
            tl::enums::Updates::Updates(u) => u.chats,
            tl::enums::Updates::Combined(u) => u.chats,
            _ => vec![],
        };
        chats
            .into_iter()
            .next()
            .ok_or_else(|| InvocationError::Deserialize("create_group: no chat in response".into()))
    }

    pub async fn create_channel(
        &self,
        title: impl Into<String>,
        about: impl Into<String>,
        broadcast: bool,
    ) -> Result<tl::enums::Chat, InvocationError> {
        let req = tl::functions::channels::CreateChannel {
            broadcast,
            megagroup: !broadcast,
            for_import: false,
            forum: false,
            title: title.into(),
            about: about.into(),
            geo_point: None,
            address: None,
            ttl_period: None,
        };
        let body = self.rpc_call_raw(&req).await?;
        let mut cur = Cursor::from_slice(&body);
        let updates = tl::enums::Updates::deserialize(&mut cur)?;
        let chats = match updates {
            tl::enums::Updates::Updates(u) => u.chats,
            tl::enums::Updates::Combined(u) => u.chats,
            _ => vec![],
        };
        chats.into_iter().next().ok_or_else(|| {
            InvocationError::Deserialize("create_channel: no chat in response".into())
        })
    }

    pub async fn edit_chat_default_banned_rights(
        &self,
        peer: impl Into<PeerRef>,
        build: impl FnOnce(
            crate::participants::BannedRightsBuilder,
        ) -> crate::participants::BannedRightsBuilder,
    ) -> Result<(), InvocationError> {
        let peer = peer.into().resolve(self).await?;
        let input_peer = self.inner.peer_cache.read().await.peer_to_input(&peer)?;
        let rights = build(crate::participants::BannedRightsBuilder::new()).into_tl();
        let req = tl::functions::messages::EditChatDefaultBannedRights {
            peer: input_peer,
            banned_rights: rights,
        };
        self.rpc_write(&req).await
    }

    pub async fn invite_users(
        &self,
        peer: impl Into<PeerRef>,
        user_ids: &[i64],
    ) -> Result<(), InvocationError> {
        if user_ids.is_empty() {
            return Ok(());
        }
        let peer = peer.into().resolve(self).await?;
        let input_peer = self.inner.peer_cache.read().await.peer_to_input(&peer)?;

        match &input_peer {
            tl::enums::InputPeer::Channel(c) => {
                let cache = self.inner.peer_cache.read().await;
                let users: Vec<tl::enums::InputUser> = user_ids
                    .iter()
                    .map(|&id| {
                        let hash = cache.users.get(&id).copied().unwrap_or(0);
                        tl::enums::InputUser::InputUser(tl::types::InputUser {
                            user_id: id,
                            access_hash: hash,
                        })
                    })
                    .collect();
                let req = tl::functions::channels::InviteToChannel {
                    channel: tl::enums::InputChannel::InputChannel(tl::types::InputChannel {
                        channel_id: c.channel_id,
                        access_hash: c.access_hash,
                    }),
                    users,
                };
                self.rpc_write(&req).await
            }
            tl::enums::InputPeer::Chat(c) => {
                // Legacy groups: add one at a time
                for &id in user_ids {
                    let hash = self
                        .inner
                        .peer_cache
                        .read()
                        .await
                        .users
                        .get(&id)
                        .copied()
                        .unwrap_or(0);
                    let req = tl::functions::messages::AddChatUser {
                        chat_id: c.chat_id,
                        user_id: tl::enums::InputUser::InputUser(tl::types::InputUser {
                            user_id: id,
                            access_hash: hash,
                        }),
                        fwd_limit: 0,
                    };
                    self.rpc_write(&req).await?;
                }
                Ok(())
            }
            _ => Err(InvocationError::Deserialize(
                "invite_users: peer must be a chat or channel".into(),
            )),
        }
    }

    pub async fn set_history_ttl(
        &self,
        peer: impl Into<PeerRef>,
        period: i32,
    ) -> Result<(), InvocationError> {
        let peer = peer.into().resolve(self).await?;
        let input_peer = self.inner.peer_cache.read().await.peer_to_input(&peer)?;
        let req = tl::functions::messages::SetHistoryTtl {
            peer: input_peer,
            period,
        };
        self.rpc_write(&req).await
    }

    pub async fn get_common_chats(
        &self,
        user_id: i64,
        max_id: i64,
        limit: i32,
    ) -> Result<Vec<tl::enums::Chat>, InvocationError> {
        let hash = self
            .inner
            .peer_cache
            .read()
            .await
            .users
            .get(&user_id)
            .copied()
            .unwrap_or(0);
        let req = tl::functions::messages::GetCommonChats {
            user_id: tl::enums::InputUser::InputUser(tl::types::InputUser {
                user_id,
                access_hash: hash,
            }),
            max_id,
            limit,
        };
        let body = self.rpc_call_raw(&req).await?;
        let mut cur = Cursor::from_slice(&body);
        let chats = tl::enums::messages::Chats::deserialize(&mut cur)?;
        Ok(match chats {
            tl::enums::messages::Chats::Chats(c) => c.chats,
            tl::enums::messages::Chats::Slice(c) => c.chats,
        })
    }

    pub async fn export_invite_link(
        &self,
        peer: impl Into<PeerRef>,
        expire_date: Option<i32>,
        usage_limit: Option<i32>,
        request_needed: bool,
        title: Option<String>,
    ) -> Result<tl::enums::ExportedChatInvite, InvocationError> {
        let peer = peer.into().resolve(self).await?;
        let input_peer = self.inner.peer_cache.read().await.peer_to_input(&peer)?;
        let req = tl::functions::messages::ExportChatInvite {
            legacy_revoke_permanent: false,
            request_needed,
            peer: input_peer,
            expire_date,
            usage_limit,
            title,
            subscription_pricing: None,
        };
        let body = self.rpc_call_raw(&req).await?;
        let mut cur = Cursor::from_slice(&body);
        Ok(tl::enums::ExportedChatInvite::deserialize(&mut cur)?)
    }

    pub async fn revoke_invite_link(
        &self,
        peer: impl Into<PeerRef>,
        link: impl Into<String>,
    ) -> Result<tl::enums::ExportedChatInvite, InvocationError> {
        let peer = peer.into().resolve(self).await?;
        let input_peer = self.inner.peer_cache.read().await.peer_to_input(&peer)?;
        let req = tl::functions::messages::EditExportedChatInvite {
            revoked: true,
            peer: input_peer,
            link: link.into(),
            expire_date: None,
            usage_limit: None,
            request_needed: None,
            title: None,
        };
        let body = self.rpc_call_raw(&req).await?;
        let mut cur = Cursor::from_slice(&body);
        let invite = tl::enums::messages::ExportedChatInvite::deserialize(&mut cur)?;
        let result = match invite {
            tl::enums::messages::ExportedChatInvite::ExportedChatInvite(i) => i,
            _ => {
                return Err(InvocationError::Deserialize(
                    "unexpected ExportedChatInvite variant".into(),
                ));
            }
        };
        Ok(result.invite)
    }

    pub async fn edit_invite_link(
        &self,
        peer: impl Into<PeerRef>,
        link: impl Into<String>,
        expire_date: Option<i32>,
        usage_limit: Option<i32>,
        request_needed: Option<bool>,
        title: Option<String>,
    ) -> Result<tl::enums::ExportedChatInvite, InvocationError> {
        let peer = peer.into().resolve(self).await?;
        let input_peer = self.inner.peer_cache.read().await.peer_to_input(&peer)?;
        let req = tl::functions::messages::EditExportedChatInvite {
            revoked: false,
            peer: input_peer,
            link: link.into(),
            expire_date,
            usage_limit,
            request_needed,
            title,
        };
        let body = self.rpc_call_raw(&req).await?;
        let mut cur = Cursor::from_slice(&body);
        let invite = tl::enums::messages::ExportedChatInvite::deserialize(&mut cur)?;
        let result = match invite {
            tl::enums::messages::ExportedChatInvite::ExportedChatInvite(i) => i,
            _ => {
                return Err(InvocationError::Deserialize(
                    "unexpected ExportedChatInvite variant".into(),
                ));
            }
        };
        Ok(result.invite)
    }

    pub async fn get_invite_links(
        &self,
        peer: impl Into<PeerRef>,
        admin_id: i64,
        revoked: bool,
        limit: i32,
        offset_date: Option<i32>,
        offset_link: Option<String>,
    ) -> Result<Vec<tl::enums::ExportedChatInvite>, InvocationError> {
        let peer = peer.into().resolve(self).await?;
        let input_peer = self.inner.peer_cache.read().await.peer_to_input(&peer)?;
        let admin_hash = self
            .inner
            .peer_cache
            .read()
            .await
            .users
            .get(&admin_id)
            .copied()
            .unwrap_or(0);
        let req = tl::functions::messages::GetExportedChatInvites {
            revoked,
            peer: input_peer,
            admin_id: tl::enums::InputUser::InputUser(tl::types::InputUser {
                user_id: admin_id,
                access_hash: admin_hash,
            }),
            offset_date,
            offset_link,
            limit,
        };
        let body = self.rpc_call_raw(&req).await?;
        let mut cur = Cursor::from_slice(&body);
        let invites = tl::enums::messages::ExportedChatInvites::deserialize(&mut cur)?;
        let tl::enums::messages::ExportedChatInvites::ExportedChatInvites(result) = invites;
        self.cache_users_slice(&result.users).await;
        Ok(result.invites)
    }

    pub async fn delete_invite_link(
        &self,
        peer: impl Into<PeerRef>,
        link: impl Into<String>,
    ) -> Result<(), InvocationError> {
        let peer = peer.into().resolve(self).await?;
        let input_peer = self.inner.peer_cache.read().await.peer_to_input(&peer)?;
        let req = tl::functions::messages::DeleteExportedChatInvite {
            peer: input_peer,
            link: link.into(),
        };
        self.rpc_write(&req).await
    }

    pub async fn delete_revoked_invite_links(
        &self,
        peer: impl Into<PeerRef>,
        admin_id: i64,
    ) -> Result<(), InvocationError> {
        let peer = peer.into().resolve(self).await?;
        let input_peer = self.inner.peer_cache.read().await.peer_to_input(&peer)?;
        let admin_hash = self
            .inner
            .peer_cache
            .read()
            .await
            .users
            .get(&admin_id)
            .copied()
            .unwrap_or(0);
        let req = tl::functions::messages::DeleteRevokedExportedChatInvites {
            peer: input_peer,
            admin_id: tl::enums::InputUser::InputUser(tl::types::InputUser {
                user_id: admin_id,
                access_hash: admin_hash,
            }),
        };
        self.rpc_write(&req).await
    }

    pub async fn join_request(
        &self,
        peer: impl Into<PeerRef>,
        user_id: i64,
        approve: bool,
    ) -> Result<(), InvocationError> {
        let peer = peer.into().resolve(self).await?;
        let input_peer = self.inner.peer_cache.read().await.peer_to_input(&peer)?;
        let user_hash = self
            .inner
            .peer_cache
            .read()
            .await
            .users
            .get(&user_id)
            .copied()
            .unwrap_or(0);
        let req = tl::functions::messages::HideChatJoinRequest {
            approved: approve,
            peer: input_peer,
            user_id: tl::enums::InputUser::InputUser(tl::types::InputUser {
                user_id,
                access_hash: user_hash,
            }),
        };
        self.rpc_write(&req).await
    }

    pub async fn all_join_requests(
        &self,
        peer: impl Into<PeerRef>,
        approve: bool,
        link: Option<String>,
    ) -> Result<(), InvocationError> {
        let peer = peer.into().resolve(self).await?;
        let input_peer = self.inner.peer_cache.read().await.peer_to_input(&peer)?;
        let req = tl::functions::messages::HideAllChatJoinRequests {
            approved: approve,
            peer: input_peer,
            link,
        };
        self.rpc_write(&req).await
    }

    pub async fn get_invite_link_members(
        &self,
        peer: impl Into<PeerRef>,
        link: Option<String>,
        requested: bool,
        limit: i32,
        offset_date: i32,
        offset_user_id: i64,
    ) -> Result<Vec<tl::types::ChatInviteImporter>, InvocationError> {
        let peer = peer.into().resolve(self).await?;
        let input_peer = self.inner.peer_cache.read().await.peer_to_input(&peer)?;
        let offset_hash = self
            .inner
            .peer_cache
            .read()
            .await
            .users
            .get(&offset_user_id)
            .copied()
            .unwrap_or(0);
        let req = tl::functions::messages::GetChatInviteImporters {
            requested,
            subscription_expired: false,
            peer: input_peer,
            link,
            q: None,
            offset_date,
            offset_user: tl::enums::InputUser::InputUser(tl::types::InputUser {
                user_id: offset_user_id,
                access_hash: offset_hash,
            }),
            limit,
        };
        let body = self.rpc_call_raw(&req).await?;
        let mut cur = Cursor::from_slice(&body);
        let tl::enums::messages::ChatInviteImporters::ChatInviteImporters(result) =
            tl::enums::messages::ChatInviteImporters::deserialize(&mut cur)?;
        self.cache_users_slice(&result.users).await;
        Ok(result
            .importers
            .into_iter()
            .map(|x| {
                let tl::enums::ChatInviteImporter::ChatInviteImporter(i) = x;
                i
            })
            .collect())
    }

    pub async fn get_admins_with_invites(
        &self,
        peer: impl Into<PeerRef>,
    ) -> Result<tl::types::messages::ChatAdminsWithInvites, InvocationError> {
        let peer = peer.into().resolve(self).await?;
        let input_peer = self.inner.peer_cache.read().await.peer_to_input(&peer)?;
        let req = tl::functions::messages::GetAdminsWithInvites { peer: input_peer };
        let body = self.rpc_call_raw(&req).await?;
        let mut cur = Cursor::from_slice(&body);
        let tl::enums::messages::ChatAdminsWithInvites::ChatAdminsWithInvites(result) =
            tl::enums::messages::ChatAdminsWithInvites::deserialize(&mut cur)?;
        self.cache_users_slice(&result.users).await;
        Ok(result)
    }

    pub async fn get_chat_administrators(
        &self,
        peer: impl Into<PeerRef>,
    ) -> Result<Vec<crate::participants::Participant>, InvocationError> {
        use ferogram_tl_types::{Cursor, Deserializable};
        let peer = peer.into().resolve(self).await?;
        match &peer {
            tl::enums::Peer::Channel(c) => {
                let access_hash = self
                    .inner
                    .peer_cache
                    .read()
                    .await
                    .channels
                    .get(&c.channel_id)
                    .map(|&(hash, _)| hash)
                    .unwrap_or(0);
                let req = tl::functions::channels::GetParticipants {
                    channel: tl::enums::InputChannel::InputChannel(tl::types::InputChannel {
                        channel_id: c.channel_id,
                        access_hash,
                    }),
                    filter: tl::enums::ChannelParticipantsFilter::ChannelParticipantsAdmins,
                    offset: 0,
                    limit: 200,
                    hash: 0,
                };
                let body = self.rpc_call_raw(&req).await?;
                let mut cur = Cursor::from_slice(&body);
                let raw = match tl::enums::channels::ChannelParticipants::deserialize(&mut cur)? {
                    tl::enums::channels::ChannelParticipants::ChannelParticipants(p) => p,
                    tl::enums::channels::ChannelParticipants::NotModified => return Ok(vec![]),
                };
                let user_map: std::collections::HashMap<i64, tl::types::User> = raw
                    .users
                    .into_iter()
                    .filter_map(|u| match u {
                        tl::enums::User::User(u) => Some((u.id, u)),
                        _ => None,
                    })
                    .collect();
                Ok(raw
                    .participants
                    .into_iter()
                    .filter_map(|p| {
                        crate::participants::Participant::from_channel_participant(p, &user_map)
                    })
                    .collect())
            }
            tl::enums::Peer::Chat(_) => {
                // For basic groups return all members; callers check is_admin flag.
                self.get_participants(peer, 0).await
            }
            _ => Err(InvocationError::Deserialize(
                "get_chat_administrators: peer must be a chat or channel".into(),
            )),
        }
    }

    pub async fn get_forum_topics(
        &self,
        peer: impl Into<PeerRef>,
        query: Option<String>,
        limit: i32,
        offset_date: i32,
        offset_id: i32,
        offset_topic: i32,
    ) -> Result<Vec<tl::enums::ForumTopic>, InvocationError> {
        let peer = peer.into().resolve(self).await?;
        let input_peer = self.inner.peer_cache.read().await.peer_to_input(&peer)?;
        let req = tl::functions::messages::GetForumTopics {
            peer: input_peer,
            q: query,
            offset_date,
            offset_id,
            offset_topic,
            limit,
        };
        let body = self.rpc_call_raw(&req).await?;
        let mut cur = Cursor::from_slice(&body);
        let tl::enums::messages::ForumTopics::ForumTopics(result) =
            tl::enums::messages::ForumTopics::deserialize(&mut cur)?;
        self.cache_users_slice(&result.users).await;
        self.cache_chats_slice(&result.chats).await;
        Ok(result.topics)
    }

    pub async fn get_forum_topics_by_id(
        &self,
        peer: impl Into<PeerRef>,
        topic_ids: Vec<i32>,
    ) -> Result<Vec<tl::enums::ForumTopic>, InvocationError> {
        let peer = peer.into().resolve(self).await?;
        let input_peer = self.inner.peer_cache.read().await.peer_to_input(&peer)?;
        let req = tl::functions::messages::GetForumTopicsById {
            peer: input_peer,
            topics: topic_ids,
        };
        let body = self.rpc_call_raw(&req).await?;
        let mut cur = Cursor::from_slice(&body);
        let tl::enums::messages::ForumTopics::ForumTopics(result) =
            tl::enums::messages::ForumTopics::deserialize(&mut cur)?;
        self.cache_users_slice(&result.users).await;
        self.cache_chats_slice(&result.chats).await;
        Ok(result.topics)
    }

    pub async fn create_forum_topic(
        &self,
        peer: impl Into<PeerRef>,
        title: impl Into<String>,
        icon_color: Option<i32>,
        icon_emoji_id: Option<i64>,
    ) -> Result<(), InvocationError> {
        let peer = peer.into().resolve(self).await?;
        let input_peer = self.inner.peer_cache.read().await.peer_to_input(&peer)?;
        let req = tl::functions::messages::CreateForumTopic {
            title_missing: false,
            peer: input_peer,
            title: title.into(),
            icon_color,
            icon_emoji_id,
            random_id: random_i64(),
            send_as: None,
        };
        self.rpc_write(&req).await
    }

    pub async fn edit_forum_topic(
        &self,
        peer: impl Into<PeerRef>,
        topic_id: i32,
        title: Option<String>,
        icon_emoji_id: Option<i64>,
        closed: Option<bool>,
        hidden: Option<bool>,
    ) -> Result<(), InvocationError> {
        let peer = peer.into().resolve(self).await?;
        let input_peer = self.inner.peer_cache.read().await.peer_to_input(&peer)?;
        let req = tl::functions::messages::EditForumTopic {
            peer: input_peer,
            topic_id,
            title,
            icon_emoji_id,
            closed,
            hidden,
        };
        self.rpc_write(&req).await
    }

    pub async fn delete_forum_topic_history(
        &self,
        peer: impl Into<PeerRef>,
        top_msg_id: i32,
    ) -> Result<(), InvocationError> {
        let peer = peer.into().resolve(self).await?;
        let input_peer = self.inner.peer_cache.read().await.peer_to_input(&peer)?;
        loop {
            let req = tl::functions::messages::DeleteTopicHistory {
                peer: input_peer.clone(),
                top_msg_id,
            };
            let body = self.rpc_call_raw(&req).await?;
            let mut cur = Cursor::from_slice(&body);
            let tl::enums::messages::AffectedHistory::AffectedHistory(result) =
                tl::enums::messages::AffectedHistory::deserialize(&mut cur)?;
            if result.offset == 0 {
                break;
            }
        }
        Ok(())
    }

    pub async fn toggle_forum(
        &self,
        peer: impl Into<PeerRef>,
        enabled: bool,
    ) -> Result<(), InvocationError> {
        let peer = peer.into().resolve(self).await?;
        let input_peer = self.inner.peer_cache.read().await.peer_to_input(&peer)?;
        let channel = match &input_peer {
            tl::enums::InputPeer::Channel(c) => {
                tl::enums::InputChannel::InputChannel(tl::types::InputChannel {
                    channel_id: c.channel_id,
                    access_hash: c.access_hash,
                })
            }
            _ => {
                return Err(InvocationError::Deserialize(
                    "toggle_forum: peer must be a supergroup channel".into(),
                ));
            }
        };
        let req = tl::functions::channels::ToggleForum {
            channel,
            enabled,
            tabs: false,
        };
        self.rpc_write(&req).await
    }

    pub async fn transfer_chat_ownership(
        &self,
        peer: impl Into<PeerRef>,
        new_owner_id: i64,
        password: tl::enums::InputCheckPasswordSrp,
    ) -> Result<(), InvocationError> {
        use ferogram_tl_types as tl;
        let peer = peer.into().resolve(self).await?;
        let input_peer = self.inner.peer_cache.read().await.peer_to_input(&peer)?;

        // Resolve the new owner to InputUser
        let owner_peer = tl::enums::Peer::User(tl::types::PeerUser {
            user_id: new_owner_id,
        });
        let owner_input = self
            .inner
            .peer_cache
            .read()
            .await
            .peer_to_input(&owner_peer)?;
        let user_id = match owner_input {
            tl::enums::InputPeer::User(u) => {
                tl::enums::InputUser::InputUser(tl::types::InputUser {
                    user_id: u.user_id,
                    access_hash: u.access_hash,
                })
            }
            _ => {
                return Err(InvocationError::Deserialize(
                    "transfer_chat_ownership: new owner must be a user".into(),
                ));
            }
        };

        let req = tl::functions::messages::EditChatCreator {
            peer: input_peer,
            user_id,
            password,
        };
        self.rpc_call_raw(&req).await?;
        Ok(())
    }

    pub async fn get_linked_channel(
        &self,
        peer: impl Into<PeerRef>,
    ) -> Result<Option<i64>, InvocationError> {
        use ferogram_tl_types as tl;
        let peer = peer.into().resolve(self).await?;
        let input_peer = self.inner.peer_cache.read().await.peer_to_input(&peer)?;
        let channel = match &input_peer {
            tl::enums::InputPeer::Channel(c) => {
                tl::enums::InputChannel::InputChannel(tl::types::InputChannel {
                    channel_id: c.channel_id,
                    access_hash: c.access_hash,
                })
            }
            _ => {
                return Err(InvocationError::Deserialize(
                    "get_linked_channel: peer must be a channel or supergroup".into(),
                ));
            }
        };
        let req = tl::functions::channels::GetFullChannel { channel };
        let body = self.rpc_call_raw(&req).await?;
        let mut cur = Cursor::from_slice(&body);
        let full = tl::enums::messages::ChatFull::deserialize(&mut cur)?;
        let linked = match full {
            tl::enums::messages::ChatFull::ChatFull(f) => match f.full_chat {
                tl::enums::ChatFull::ChannelFull(cf) => cf.linked_chat_id,
                _ => None,
            },
        };
        Ok(linked)
    }

    pub async fn add_contact(
        &self,
        user_id: i64,
        first_name: impl Into<String>,
        last_name: impl Into<String>,
        phone: impl Into<String>,
        add_phone_privacy_exception: bool,
    ) -> Result<(), InvocationError> {
        let hash = self
            .inner
            .peer_cache
            .read()
            .await
            .users
            .get(&user_id)
            .copied()
            .unwrap_or(0);
        let req = tl::functions::contacts::AddContact {
            add_phone_privacy_exception,
            id: tl::enums::InputUser::InputUser(tl::types::InputUser {
                user_id,
                access_hash: hash,
            }),
            first_name: first_name.into(),
            last_name: last_name.into(),
            phone: phone.into(),
            note: None,
        };
        self.rpc_write(&req).await
    }

    pub async fn import_contacts(
        &self,
        contacts: &[(&str, &str, &str)],
    ) -> Result<tl::types::contacts::ImportedContacts, InvocationError> {
        use ferogram_tl_types::{Cursor, Deserializable};
        let contacts_tl: Vec<tl::enums::InputContact> = contacts
            .iter()
            .enumerate()
            .map(|(i, (phone, first, last))| {
                tl::enums::InputContact::InputPhoneContact(tl::types::InputPhoneContact {
                    client_id: i as i64,
                    phone: phone.to_string(),
                    first_name: first.to_string(),
                    last_name: last.to_string(),
                    note: None,
                })
            })
            .collect();
        let req = tl::functions::contacts::ImportContacts {
            contacts: contacts_tl,
        };
        let body = self.rpc_call_raw(&req).await?;
        let mut cur = Cursor::from_slice(&body);
        let tl::enums::contacts::ImportedContacts::ImportedContacts(result) =
            tl::enums::contacts::ImportedContacts::deserialize(&mut cur)?;
        self.cache_users_slice(&result.users).await;
        Ok(result)
    }

    pub async fn get_blocked_users(
        &self,
        offset: i32,
        limit: i32,
    ) -> Result<Vec<tl::enums::Peer>, InvocationError> {
        let req = tl::functions::contacts::GetBlocked {
            my_stories_from: false,
            offset,
            limit,
        };
        let body = self.rpc_call_raw(&req).await?;
        let mut cur = Cursor::from_slice(&body);
        let (blocked, chats, users) = match tl::enums::contacts::Blocked::deserialize(&mut cur)? {
            tl::enums::contacts::Blocked::Blocked(b) => (b.blocked, b.chats, b.users),
            tl::enums::contacts::Blocked::Slice(b) => (b.blocked, b.chats, b.users),
        };
        self.cache_users_slice(&users).await;
        self.cache_chats_slice(&chats).await;
        Ok(blocked
            .into_iter()
            .map(|b| match b {
                tl::enums::PeerBlocked::PeerBlocked(pb) => pb.peer_id,
            })
            .collect())
    }

    pub async fn search_contacts(
        &self,
        query: impl Into<String>,
        limit: i32,
    ) -> Result<Vec<tl::enums::Peer>, InvocationError> {
        let req = tl::functions::contacts::Search {
            q: query.into(),
            limit,
            bots: false,
            broadcasts: false,
        };
        let body = self.rpc_call_raw(&req).await?;
        let mut cur = Cursor::from_slice(&body);
        let tl::enums::contacts::Found::Found(found) =
            tl::enums::contacts::Found::deserialize(&mut cur)?;
        self.cache_users_slice(&found.users).await;
        self.cache_chats_slice(&found.chats).await;
        // Combine my_results + results, deduplicated by position
        let mut peers = found.my_results;
        for p in found.results {
            if !peers.contains(&p) {
                peers.push(p);
            }
        }
        Ok(peers)
    }

    pub async fn delete_profile_photos(
        &self,
        photo_ids: Vec<(i64, i64, Vec<u8>)>,
    ) -> Result<Vec<i64>, InvocationError> {
        let id: Vec<tl::enums::InputPhoto> = photo_ids
            .into_iter()
            .map(|(id, access_hash, file_reference)| {
                tl::enums::InputPhoto::InputPhoto(tl::types::InputPhoto {
                    id,
                    access_hash,
                    file_reference,
                })
            })
            .collect();
        let req = tl::functions::photos::DeletePhotos { id };
        let body = self.rpc_call_raw(&req).await?;
        let mut cur = Cursor::from_slice(&body);
        // Returns Vector<long> - the IDs that were actually deleted.
        let v = Vec::<i64>::deserialize(&mut cur)?;
        Ok(v)
    }

    /// Update user or chat profile fields via a builder.
    ///
    /// Call `.send().await` to apply. Unset fields are left unchanged.
    ///
    /// # Example
    /// ```rust,no_run
    /// # use ferogram::Client;
    /// # async fn ex(client: Client) {
    /// client.set_profile("me").name("Alice", "").bio("Hello!").send().await.unwrap();
    /// # }
    /// ```
    pub fn set_profile(&self, peer: impl Into<PeerRef>) -> crate::SetProfileBuilder {
        crate::SetProfileBuilder::new(self.clone(), peer.into())
    }

    pub async fn get_authorizations(
        &self,
    ) -> Result<Vec<tl::types::Authorization>, InvocationError> {
        let req = tl::functions::account::GetAuthorizations {};
        let body = self.rpc_call_raw(&req).await?;
        let mut cur = Cursor::from_slice(&body);
        let tl::enums::account::Authorizations::Authorizations(result) =
            tl::enums::account::Authorizations::deserialize(&mut cur)?;
        Ok(result
            .authorizations
            .into_iter()
            .map(|x| {
                let tl::enums::Authorization::Authorization(a) = x;
                a
            })
            .collect())
    }

    pub async fn get_user_full(
        &self,
        user_id: i64,
    ) -> Result<tl::types::UserFull, InvocationError> {
        let hash = self
            .inner
            .peer_cache
            .read()
            .await
            .users
            .get(&user_id)
            .copied()
            .unwrap_or(0);
        let req = tl::functions::users::GetFullUser {
            id: tl::enums::InputUser::InputUser(tl::types::InputUser {
                user_id,
                access_hash: hash,
            }),
        };
        let body = self.rpc_call_raw(&req).await?;
        let mut cur = Cursor::from_slice(&body);
        let tl::enums::users::UserFull::UserFull(result) =
            tl::enums::users::UserFull::deserialize(&mut cur)?;
        self.cache_users_slice(&result.users).await;
        self.cache_chats_slice(&result.chats).await;
        let tl::enums::UserFull::UserFull(full_user) = result.full_user;
        Ok(full_user)
    }

    /// Retrieve channel or supergroup statistics.
    ///
    /// Auto-dispatches to `stats.getBroadcastStats` for channels and
    /// `stats.getMegagroupStats` for supergroups.
    pub async fn stats(
        &self,
        peer: impl Into<PeerRef>,
    ) -> Result<crate::ChannelStats, InvocationError> {
        use ferogram_tl_types as tl;
        let peer = peer.into().resolve(self).await?;
        let input_peer = self.inner.peer_cache.read().await.peer_to_input(&peer)?;
        let channel = match &input_peer {
            tl::enums::InputPeer::Channel(c) => {
                tl::enums::InputChannel::InputChannel(tl::types::InputChannel {
                    channel_id: c.channel_id,
                    access_hash: c.access_hash,
                })
            }
            _ => {
                return Err(InvocationError::Deserialize(
                    "stats: peer must be a channel or supergroup".into(),
                ));
            }
        };
        // Try broadcast stats first; fall back to megagroup stats on error.
        let broadcast_req = tl::functions::stats::GetBroadcastStats {
            dark: false,
            channel: channel.clone(),
        };
        if let Ok(body) = self.rpc_call_raw(&broadcast_req).await {
            let mut cur = Cursor::from_slice(&body);
            if let Ok(s) = tl::enums::stats::BroadcastStats::deserialize(&mut cur) {
                return Ok(crate::ChannelStats::Broadcast(s));
            }
        }
        let meg_req = tl::functions::stats::GetMegagroupStats {
            dark: false,
            channel,
        };
        let body = self.rpc_call_raw(&meg_req).await?;
        let mut cur = Cursor::from_slice(&body);
        Ok(crate::ChannelStats::Megagroup(
            tl::enums::stats::MegagroupStats::deserialize(&mut cur)?,
        ))
    }

    pub async fn send_scheduled_now(
        &self,
        peer: impl Into<PeerRef>,
        ids: &[i32],
    ) -> Result<(), InvocationError> {
        let peer = peer.into().resolve(self).await?;
        let input_peer = self.inner.peer_cache.read().await.peer_to_input(&peer)?;
        let req = tl::functions::messages::SendScheduledMessages {
            peer: input_peer,
            id: ids.to_vec(),
        };
        self.rpc_write(&req).await
    }

    pub async fn get_message_read_participants(
        &self,
        peer: impl Into<PeerRef>,
        msg_id: i32,
    ) -> Result<Vec<tl::types::ReadParticipantDate>, InvocationError> {
        let peer = peer.into().resolve(self).await?;
        let input_peer = self.inner.peer_cache.read().await.peer_to_input(&peer)?;
        let req = tl::functions::messages::GetMessageReadParticipants {
            peer: input_peer,
            msg_id,
        };
        let body = self.rpc_call_raw(&req).await?;
        let mut cur = Cursor::from_slice(&body);
        Ok(
            Vec::<tl::enums::ReadParticipantDate>::deserialize(&mut cur)?
                .into_iter()
                .map(|r| match r {
                    tl::enums::ReadParticipantDate::ReadParticipantDate(d) => d,
                })
                .collect(),
        )
    }

    pub async fn get_replies(
        &self,
        peer: impl Into<PeerRef>,
        msg_id: i32,
        limit: i32,
        offset_id: i32,
    ) -> Result<Vec<update::IncomingMessage>, InvocationError> {
        let peer = peer.into().resolve(self).await?;
        let input_peer = self.inner.peer_cache.read().await.peer_to_input(&peer)?;
        let req = tl::functions::messages::GetReplies {
            peer: input_peer,
            msg_id,
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

    pub async fn get_discussion_message(
        &self,
        peer: impl Into<PeerRef>,
        msg_id: i32,
    ) -> Result<tl::types::messages::DiscussionMessage, InvocationError> {
        let peer = peer.into().resolve(self).await?;
        let input_peer = self.inner.peer_cache.read().await.peer_to_input(&peer)?;
        let req = tl::functions::messages::GetDiscussionMessage {
            peer: input_peer,
            msg_id,
        };
        let body = self.rpc_call_raw(&req).await?;
        let mut cur = Cursor::from_slice(&body);
        let tl::enums::messages::DiscussionMessage::DiscussionMessage(result) =
            tl::enums::messages::DiscussionMessage::deserialize(&mut cur)?;
        self.cache_users_slice(&result.users).await;
        self.cache_chats_slice(&result.chats).await;
        Ok(result)
    }

    pub async fn read_discussion(
        &self,
        peer: impl Into<PeerRef>,
        msg_id: i32,
        read_max_id: i32,
    ) -> Result<(), InvocationError> {
        let peer = peer.into().resolve(self).await?;
        let input_peer = self.inner.peer_cache.read().await.peer_to_input(&peer)?;
        let req = tl::functions::messages::ReadDiscussion {
            peer: input_peer,
            msg_id,
            read_max_id,
        };
        self.rpc_write(&req).await
    }

    pub async fn get_web_page_preview(
        &self,
        text: impl Into<String>,
    ) -> Result<tl::enums::MessageMedia, InvocationError> {
        let req = tl::functions::messages::GetWebPagePreview {
            message: text.into(),
            entities: None,
        };
        let body = self.rpc_call_raw(&req).await?;
        let mut cur = Cursor::from_slice(&body);
        let tl::enums::messages::WebPagePreview::WebPagePreview(result) =
            tl::enums::messages::WebPagePreview::deserialize(&mut cur)?;
        Ok(result.media)
    }

    pub async fn get_reactions(
        &self,
        peer: impl Into<PeerRef>,
        msg_ids: Vec<i32>,
    ) -> Result<(), InvocationError> {
        let peer = peer.into().resolve(self).await?;
        let input_peer = self.inner.peer_cache.read().await.peer_to_input(&peer)?;
        let req = tl::functions::messages::GetMessagesReactions {
            peer: input_peer,
            id: msg_ids,
        };
        self.rpc_write(&req).await
    }

    /// Report (and request removal of) a specific user's reaction on a message.
    pub async fn delete_reaction(
        &self,
        peer: impl Into<PeerRef>,
        msg_id: i32,
        participant: impl Into<PeerRef>,
    ) -> Result<bool, InvocationError> {
        let peer = peer.into().resolve(self).await?;
        let input_peer = self.inner.peer_cache.read().await.peer_to_input(&peer)?;
        let part = participant.into().resolve(self).await?;
        let reaction_peer = self.inner.peer_cache.read().await.peer_to_input(&part)?;
        let req = tl::functions::messages::ReportReaction {
            peer: input_peer,
            id: msg_id,
            reaction_peer,
        };
        let body: Vec<u8> = self.rpc_call_raw(&req).await?;
        Ok(body.len() >= 4 && u32::from_le_bytes(body[..4].try_into().unwrap()) == 0x997275b5)
    }

    pub async fn iter_reaction_users(
        &self,
        peer: impl Into<PeerRef>,
        msg_id: i32,
        reaction: Option<tl::enums::Reaction>,
        limit: i32,
        offset: Option<String>,
    ) -> Result<tl::types::messages::MessageReactionsList, InvocationError> {
        let peer = peer.into().resolve(self).await?;
        let input_peer = self.inner.peer_cache.read().await.peer_to_input(&peer)?;
        let req = tl::functions::messages::GetMessageReactionsList {
            peer: input_peer,
            id: msg_id,
            reaction,
            offset,
            limit,
        };
        let body = self.rpc_call_raw(&req).await?;
        let mut cur = Cursor::from_slice(&body);
        let tl::enums::messages::MessageReactionsList::MessageReactionsList(result) =
            tl::enums::messages::MessageReactionsList::deserialize(&mut cur)?;
        self.cache_users_slice(&result.users).await;
        self.cache_chats_slice(&result.chats).await;
        Ok(result)
    }

    pub async fn send_paid_reaction(
        &self,
        peer: impl Into<PeerRef>,
        msg_id: i32,
        count: i32,
    ) -> Result<(), InvocationError> {
        let peer = peer.into().resolve(self).await?;
        let input_peer = self.inner.peer_cache.read().await.peer_to_input(&peer)?;
        let req = tl::functions::messages::SendPaidReaction {
            peer: input_peer,
            msg_id,
            count,
            random_id: random_i64(),
            private: None,
        };
        self.rpc_write(&req).await
    }

    pub async fn translate_messages(
        &self,
        peer: impl Into<PeerRef>,
        msg_ids: Vec<i32>,
        to_lang: impl Into<String>,
    ) -> Result<Vec<tl::types::TextWithEntities>, InvocationError> {
        let peer = peer.into().resolve(self).await?;
        let input_peer = self.inner.peer_cache.read().await.peer_to_input(&peer)?;
        let req = tl::functions::messages::TranslateText {
            peer: Some(input_peer),
            id: Some(msg_ids),
            text: None,
            to_lang: to_lang.into(),
            tone: None,
        };
        let body = self.rpc_call_raw(&req).await?;
        let mut cur = Cursor::from_slice(&body);
        let tl::enums::messages::TranslatedText::TranslateResult(result) =
            tl::enums::messages::TranslatedText::deserialize(&mut cur)?;
        Ok(result
            .result
            .into_iter()
            .map(|x| {
                let tl::enums::TextWithEntities::TextWithEntities(t) = x;
                t
            })
            .collect())
    }

    pub async fn transcribe_audio(
        &self,
        peer: impl Into<PeerRef>,
        msg_id: i32,
    ) -> Result<tl::types::messages::TranscribedAudio, InvocationError> {
        let peer = peer.into().resolve(self).await?;
        let input_peer = self.inner.peer_cache.read().await.peer_to_input(&peer)?;
        let req = tl::functions::messages::TranscribeAudio {
            peer: input_peer,
            msg_id,
        };
        let body = self.rpc_call_raw(&req).await?;
        let mut cur = Cursor::from_slice(&body);
        let tl::enums::messages::TranscribedAudio::TranscribedAudio(result) =
            tl::enums::messages::TranscribedAudio::deserialize(&mut cur)?;
        Ok(result)
    }

    pub async fn toggle_peer_translations(
        &self,
        peer: impl Into<PeerRef>,
        disabled: bool,
    ) -> Result<(), InvocationError> {
        let peer = peer.into().resolve(self).await?;
        let input_peer = self.inner.peer_cache.read().await.peer_to_input(&peer)?;
        let req = tl::functions::messages::TogglePeerTranslations {
            disabled,
            peer: input_peer,
        };
        self.rpc_write(&req).await
    }

    pub async fn get_admin_log(
        &self,
        peer: impl Into<PeerRef>,
        query: impl Into<String>,
        limit: i32,
        max_id: i64,
        min_id: i64,
    ) -> Result<Vec<tl::types::ChannelAdminLogEvent>, InvocationError> {
        let peer = peer.into().resolve(self).await?;
        let input_peer = self.inner.peer_cache.read().await.peer_to_input(&peer)?;
        let channel = match &input_peer {
            tl::enums::InputPeer::Channel(c) => {
                tl::enums::InputChannel::InputChannel(tl::types::InputChannel {
                    channel_id: c.channel_id,
                    access_hash: c.access_hash,
                })
            }
            _ => {
                return Err(InvocationError::Deserialize(
                    "get_admin_log: peer must be a channel or supergroup".into(),
                ));
            }
        };
        let req = tl::functions::channels::GetAdminLog {
            channel,
            q: query.into(),
            events_filter: None,
            admins: None,
            max_id,
            min_id,
            limit,
        };
        let body = self.rpc_call_raw(&req).await?;
        let mut cur = Cursor::from_slice(&body);
        let tl::enums::channels::AdminLogResults::AdminLogResults(result) =
            tl::enums::channels::AdminLogResults::deserialize(&mut cur)?;
        self.cache_users_slice(&result.users).await;
        self.cache_chats_slice(&result.chats).await;
        Ok(result
            .events
            .into_iter()
            .map(|e| match e {
                tl::enums::ChannelAdminLogEvent::ChannelAdminLogEvent(ev) => ev,
            })
            .collect())
    }

    pub async fn toggle_no_forwards(
        &self,
        peer: impl Into<PeerRef>,
        enabled: bool,
    ) -> Result<(), InvocationError> {
        let peer = peer.into().resolve(self).await?;
        let input_peer = self.inner.peer_cache.read().await.peer_to_input(&peer)?;
        let req = tl::functions::messages::ToggleNoForwards {
            peer: input_peer,
            enabled,
            request_msg_id: None,
        };
        self.rpc_write(&req).await
    }

    pub async fn set_chat_theme(
        &self,
        peer: impl Into<PeerRef>,
        emoticon: impl Into<String>,
    ) -> Result<(), InvocationError> {
        let peer = peer.into().resolve(self).await?;
        let input_peer = self.inner.peer_cache.read().await.peer_to_input(&peer)?;
        let req = tl::functions::messages::SetChatTheme {
            peer: input_peer,
            theme: tl::enums::InputChatTheme::InputChatTheme(tl::types::InputChatTheme {
                emoticon: emoticon.into(),
            }),
        };
        self.rpc_write(&req).await
    }

    pub async fn set_chat_reactions(
        &self,
        peer: impl Into<PeerRef>,
        reactions: tl::enums::ChatReactions,
    ) -> Result<(), InvocationError> {
        let peer = peer.into().resolve(self).await?;
        let input_peer = self.inner.peer_cache.read().await.peer_to_input(&peer)?;
        let req = tl::functions::messages::SetChatAvailableReactions {
            peer: input_peer,
            available_reactions: reactions,
            reactions_limit: None,
            paid_enabled: None,
        };
        self.rpc_write(&req).await
    }

    pub async fn export_message_link(
        &self,
        peer: impl Into<PeerRef>,
        msg_id: i32,
        kind: LinkKind,
    ) -> Result<String, InvocationError> {
        let (grouped, thread) = match kind {
            LinkKind::Normal => (false, false),
            LinkKind::Grouped => (true, false),
            LinkKind::Thread => (false, true),
        };
        self.export_message_link_raw(peer, msg_id, grouped, thread)
            .await
    }

    pub(crate) async fn export_message_link_raw(
        &self,
        peer: impl Into<PeerRef>,
        msg_id: i32,
        grouped: bool,
        thread: bool,
    ) -> Result<String, InvocationError> {
        let peer = peer.into().resolve(self).await?;
        let input_peer = self.inner.peer_cache.read().await.peer_to_input(&peer)?;
        let channel = match input_peer {
            tl::enums::InputPeer::Channel(c) => {
                tl::enums::InputChannel::InputChannel(tl::types::InputChannel {
                    channel_id: c.channel_id,
                    access_hash: c.access_hash,
                })
            }
            _ => {
                return Err(InvocationError::Deserialize(
                    "export_message_link requires a channel".into(),
                ));
            }
        };
        let req = tl::functions::channels::ExportMessageLink {
            grouped,
            thread,
            channel,
            id: msg_id,
        };
        let body: Vec<u8> = self.rpc_call_raw(&req).await?;
        let mut cur = Cursor::from_slice(&body);
        let link = tl::enums::ExportedMessageLink::deserialize(&mut cur)?;
        match link {
            tl::enums::ExportedMessageLink::ExportedMessageLink(l) => Ok(l.link),
        }
    }

    /// Get notification settings for a peer.
    #[allow(dead_code)]
    pub(crate) async fn get_notify_settings_raw(
        &self,
        peer: impl Into<PeerRef>,
    ) -> Result<tl::enums::PeerNotifySettings, InvocationError> {
        let peer = peer.into().resolve(self).await?;
        let input_peer = self.inner.peer_cache.read().await.peer_to_input(&peer)?;
        let req = tl::functions::account::GetNotifySettings {
            peer: tl::enums::InputNotifyPeer::InputNotifyPeer(tl::types::InputNotifyPeer {
                peer: input_peer,
            }),
        };
        let body: Vec<u8> = self.rpc_call_raw(&req).await?;
        let mut cur = Cursor::from_slice(&body);
        Ok(tl::enums::PeerNotifySettings::deserialize(&mut cur)?)
    }

    pub async fn get_send_as_peers(
        &self,
        peer: impl Into<PeerRef>,
    ) -> Result<Vec<tl::enums::Peer>, InvocationError> {
        let peer = peer.into().resolve(self).await?;
        let input_peer = self.inner.peer_cache.read().await.peer_to_input(&peer)?;
        let req = tl::functions::channels::GetSendAs {
            for_paid_reactions: false,
            for_live_stories: false,
            peer: input_peer,
        };
        let body = self.rpc_call_raw(&req).await?;
        let mut cur = Cursor::from_slice(&body);
        let tl::enums::channels::SendAsPeers::SendAsPeers(result) =
            tl::enums::channels::SendAsPeers::deserialize(&mut cur)?;
        self.cache_users_slice(&result.users).await;
        self.cache_chats_slice(&result.chats).await;
        Ok(result
            .peers
            .into_iter()
            .map(|p| match p {
                tl::enums::SendAsPeer::SendAsPeer(sp) => sp.peer,
            })
            .collect())
    }

    pub async fn set_default_send_as(
        &self,
        peer: impl Into<PeerRef>,
        send_as_peer: impl Into<PeerRef>,
    ) -> Result<(), InvocationError> {
        let peer = peer.into().resolve(self).await?;
        let input_peer = self.inner.peer_cache.read().await.peer_to_input(&peer)?;
        let send_as = send_as_peer.into().resolve(self).await?;
        let send_as_input = self.inner.peer_cache.read().await.peer_to_input(&send_as)?;
        let req = tl::functions::messages::SaveDefaultSendAs {
            peer: input_peer,
            send_as: send_as_input,
        };
        self.rpc_write(&req).await
    }

    pub async fn send_vote(
        &self,
        peer: impl Into<PeerRef>,
        msg_id: i32,
        options: Vec<Vec<u8>>,
    ) -> Result<(), InvocationError> {
        let peer = peer.into().resolve(self).await?;
        let input_peer = self.inner.peer_cache.read().await.peer_to_input(&peer)?;
        let req = tl::functions::messages::SendVote {
            peer: input_peer,
            msg_id,
            options,
        };
        self.rpc_write(&req).await
    }

    /// Get statistics for a poll message.
    pub async fn poll_results(
        &self,
        peer: impl Into<PeerRef>,
        msg_id: i32,
    ) -> Result<tl::types::stats::PollStats, InvocationError> {
        let peer = peer.into().resolve(self).await?;
        let input_peer = self.inner.peer_cache.read().await.peer_to_input(&peer)?;
        let req = tl::functions::stats::GetPollStats {
            dark: false,
            peer: input_peer,
            msg_id,
        };
        let body = self.rpc_call_raw(&req).await?;
        let mut cur = Cursor::from_slice(&body);
        let tl::enums::stats::PollStats::PollStats(result) =
            tl::enums::stats::PollStats::deserialize(&mut cur)?;
        Ok(result)
    }

    pub async fn get_poll_votes(
        &self,
        peer: impl Into<PeerRef>,
        msg_id: i32,
        option: Option<Vec<u8>>,
        limit: i32,
        offset: Option<String>,
    ) -> Result<tl::types::messages::VotesList, InvocationError> {
        let peer = peer.into().resolve(self).await?;
        let input_peer = self.inner.peer_cache.read().await.peer_to_input(&peer)?;
        let req = tl::functions::messages::GetPollVotes {
            peer: input_peer,
            id: msg_id,
            option,
            offset,
            limit,
        };
        let body = self.rpc_call_raw(&req).await?;
        let mut cur = Cursor::from_slice(&body);
        let tl::enums::messages::VotesList::VotesList(result) =
            tl::enums::messages::VotesList::deserialize(&mut cur)?;
        self.cache_users_slice(&result.users).await;
        self.cache_chats_slice(&result.chats).await;
        Ok(result)
    }

    pub async fn get_sticker_set(
        &self,
        stickerset: tl::enums::InputStickerSet,
    ) -> Result<tl::types::messages::StickerSet, InvocationError> {
        let req = tl::functions::messages::GetStickerSet {
            stickerset,
            hash: 0,
        };
        let body = self.rpc_call_raw(&req).await?;
        let mut cur = Cursor::from_slice(&body);
        let tl::enums::messages::StickerSet::StickerSet(result) =
            tl::enums::messages::StickerSet::deserialize(&mut cur)?
        else {
            return Err(InvocationError::Deserialize(
                "unexpected StickerSet variant".into(),
            ));
        };
        Ok(result)
    }

    /// Install or uninstall a sticker set. `install: true` installs, `install: false` uninstalls.
    pub async fn toggle_stickers(
        &self,
        stickerset: tl::enums::InputStickerSet,
        install: bool,
    ) -> Result<Option<tl::enums::messages::StickerSetInstallResult>, InvocationError> {
        if install {
            let req = tl::functions::messages::InstallStickerSet {
                stickerset,
                archived: false,
            };
            let body = self.rpc_call_raw(&req).await?;
            let mut cur = Cursor::from_slice(&body);
            Ok(Some(
                tl::enums::messages::StickerSetInstallResult::deserialize(&mut cur)?,
            ))
        } else {
            let req = tl::functions::messages::UninstallStickerSet { stickerset };
            self.rpc_write(&req).await?;
            Ok(None)
        }
    }

    pub async fn get_all_stickers(
        &self,
        hash: i64,
    ) -> Result<Option<Vec<tl::types::StickerSet>>, InvocationError> {
        let req = tl::functions::messages::GetAllStickers { hash };
        let body = self.rpc_call_raw(&req).await?;
        let mut cur = Cursor::from_slice(&body);
        match tl::enums::messages::AllStickers::deserialize(&mut cur)? {
            tl::enums::messages::AllStickers::AllStickers(s) => Ok(Some(
                s.sets
                    .into_iter()
                    .map(|s| match s {
                        tl::enums::StickerSet::StickerSet(ss) => ss,
                    })
                    .collect(),
            )),
            tl::enums::messages::AllStickers::NotModified => Ok(None),
        }
    }

    pub async fn get_custom_emoji_documents(
        &self,
        document_ids: Vec<i64>,
    ) -> Result<Vec<tl::enums::Document>, InvocationError> {
        let req = tl::functions::messages::GetCustomEmojiDocuments {
            document_id: document_ids,
        };
        let body = self.rpc_call_raw(&req).await?;
        let mut cur = Cursor::from_slice(&body);
        Ok(Vec::<tl::enums::Document>::deserialize(&mut cur)?)
    }

    pub async fn get_privacy(
        &self,
        key: tl::enums::InputPrivacyKey,
    ) -> Result<Vec<tl::enums::PrivacyRule>, InvocationError> {
        let req = tl::functions::account::GetPrivacy { key };
        let body = self.rpc_call_raw(&req).await?;
        let mut cur = Cursor::from_slice(&body);
        let tl::enums::account::PrivacyRules::PrivacyRules(result) =
            tl::enums::account::PrivacyRules::deserialize(&mut cur)?;
        self.cache_users_slice(&result.users).await;
        self.cache_chats_slice(&result.chats).await;
        Ok(result.rules)
    }

    pub async fn set_privacy(
        &self,
        key: tl::enums::InputPrivacyKey,
        rules: Vec<tl::enums::InputPrivacyRule>,
    ) -> Result<Vec<tl::enums::PrivacyRule>, InvocationError> {
        let req = tl::functions::account::SetPrivacy { key, rules };
        let body = self.rpc_call_raw(&req).await?;
        let mut cur = Cursor::from_slice(&body);
        let tl::enums::account::PrivacyRules::PrivacyRules(result) =
            tl::enums::account::PrivacyRules::deserialize(&mut cur)?;
        self.cache_users_slice(&result.users).await;
        self.cache_chats_slice(&result.chats).await;
        Ok(result.rules)
    }

    pub async fn get_notify_settings(
        &self,
        peer: impl Into<PeerRef>,
    ) -> Result<tl::enums::PeerNotifySettings, InvocationError> {
        let peer = peer.into().resolve(self).await?;
        let input_peer = self.inner.peer_cache.read().await.peer_to_input(&peer)?;
        let req = tl::functions::account::GetNotifySettings {
            peer: tl::enums::InputNotifyPeer::InputNotifyPeer(tl::types::InputNotifyPeer {
                peer: input_peer,
            }),
        };
        let body = self.rpc_call_raw(&req).await?;
        let mut cur = Cursor::from_slice(&body);
        Ok(tl::enums::PeerNotifySettings::deserialize(&mut cur)?)
    }

    pub async fn update_notify_settings(
        &self,
        peer: impl Into<PeerRef>,
        settings: tl::enums::InputPeerNotifySettings,
    ) -> Result<(), InvocationError> {
        let peer = peer.into().resolve(self).await?;
        let input_peer = self.inner.peer_cache.read().await.peer_to_input(&peer)?;
        let req = tl::functions::account::UpdateNotifySettings {
            peer: tl::enums::InputNotifyPeer::InputNotifyPeer(tl::types::InputNotifyPeer {
                peer: input_peer,
            }),
            settings,
        };
        self.rpc_write(&req).await
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

impl Client {
    /// Upload a single part for experimental resumable upload.
    #[cfg(feature = "experimental")]
    pub(crate) async fn upload_part_pub(
        &self,
        big: bool,
        file_id: i64,
        part: i32,
        total_parts: i32,
        data: &[u8],
    ) -> Result<bool, InvocationError> {
        if big {
            self.rpc_call(tl::functions::upload::SaveBigFilePart {
                file_id,
                file_part: part,
                file_total_parts: total_parts,
                bytes: data.to_vec(),
            })
            .await
        } else {
            self.rpc_call(tl::functions::upload::SaveFilePart {
                file_id,
                file_part: part,
                bytes: data.to_vec(),
            })
            .await
        }
    }
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
