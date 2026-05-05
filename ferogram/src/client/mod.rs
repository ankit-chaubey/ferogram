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
use ferogram_connect::SALT_USE_DELAY;
use ferogram_mtproto::EncryptedSession;
use ferogram_tl_types as tl;
use ferogram_tl_types::{Cursor, Deserializable, RemoteCall};

use tokio::io::AsyncWriteExt;
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio::sync::{Mutex, RwLock, mpsc, oneshot};
use tokio::time::sleep;
use tokio_util::sync::CancellationToken;

const ID_RPC_RESULT: u32 = 0xf35c6d01;
const ID_RPC_ERROR: u32 = 0x2144ca19;
const ID_MSG_CONTAINER: u32 = 0x73f1f8dc;
const ID_GZIP_PACKED: u32 = 0x3072cfa1;
const ID_PONG: u32 = 0x347773c5;
const _ID_MSGS_ACK: u32 = 0x62d6b459;
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
    /// to the reader task after a reconnect. Capacity 4: at most one reconnect
    /// is in flight at any time; the bound prevents unbounded memory growth.
    reconnect_tx: mpsc::Sender<(OwnedReadHalf, FrameKind, [u8; 256], i64)>,
    /// Send `()` here to wake the reader's reconnect backoff loop immediately.
    /// Used by [`Client::signal_network_restored`]. Capacity 4: hints are
    /// best-effort; a full channel means a hint is already pending.
    network_hint_tx: mpsc::Sender<()>,
    /// Cancelled to signal graceful shutdown to the reader task.
    #[allow(dead_code)]
    shutdown_token: CancellationToken,
    /// Whether to replay missed updates via getDifference on connect.
    #[allow(dead_code)]
    catch_up: bool,
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
    session_backend: Arc<dyn crate::session_backend::SessionBackend>,
    dc_pool: Mutex<dc_pool::DcPool>,
    /// Dedicated pool for file transfer connections (upload/download).
    /// Isolated from the main session to prevent crypto state contamination.
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
    /// Prevents spawning more than one proactive GetFutureSalts at a time.
    /// Without this guard every bad_server_salt spawns a new task, which causes
    /// an exponential storm when many messages are queued with a stale salt.
    salt_request_in_flight: std::sync::atomic::AtomicBool,
    /// Prevents spawning more than one `run_pending_differences` task at a time.
    /// Without this guard, every deadline tick while a diff is already running
    /// spawns an additional task.  The underlying MessageBoxes state machine
    /// already serialises the actual RPC calls, but redundant spawns waste resources.
    diff_in_flight: std::sync::atomic::AtomicBool,
    /// Prevents two concurrent fresh-DH handshakes racing each other.
    /// A double-DH results in one key being unregistered on Telegram's servers,
    /// causing AUTH_KEY_UNREGISTERED immediately after reconnect.
    dh_in_progress: std::sync::atomic::AtomicBool,

    /// Whether PFS (bindTempAuthKey) is enabled for new connections.
    pfs_enabled: bool,
    /// Guards sync_state_after_dh: the function is a no-op while false so that
    /// reconnect-triggered DH completions don't fire GetState before the client
    /// is actually authorised.
    pub signed_in: std::sync::atomic::AtomicBool,

    /// Persistent seen-msg_id dedup ring shared with the reader task.
    /// Outlives individual EncryptedSession objects so replayed frames
    /// from prior connections are still rejected after reconnect.
    seen_msg_ids: ferogram_mtproto::SeenMsgIds,

    /// Tracks which foreign DC IDs have had `auth.importAuthorization` called
    /// successfully in the current process session (in-memory only, not persisted).
    ///
    /// Tracks which foreign DCs have had `auth.importAuthorization` called
    /// successfully in this session.  The account authorization binding is
    /// session-scoped and must be re-established each process run.
    pub(crate) auth_imported: parking_lot::Mutex<std::collections::HashSet<i32>>,

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

        // Capacity: 2048 updates. If the consumer falls behind, excess updates
        // are dropped with a warning rather than growing RAM without bound.
        let (update_tx, update_rx) = mpsc::channel(2048);

        // Load or fresh-connect
        let socks5 = config.socks5.clone();
        let mtproxy = config.mtproxy.clone();
        let transport = config.transport.clone();
        let probe_transport = config.probe_transport;
        let resilient_connect = config.resilient_connect;

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

        // Split the TCP stream immediately.
        // The writer (write half + EncryptedSession) stays in ClientInner.
        // The read half goes to the reader task which we spawn right now so
        // that RPC calls during init_connection work correctly.
        let (writer, write_half, read_half, frame_kind): (
            ferogram_connect::ConnectionWriter,
            tokio::net::tcp::OwnedWriteHalf,
            tokio::net::tcp::OwnedReadHalf,
            ferogram_connect::connection::FrameKind,
        ) = conn.into_writer();
        let auth_key = writer.enc.auth_key_bytes();
        let session_id = writer.enc.session_id();

        #[allow(clippy::type_complexity)]
        let pending: Arc<
            Mutex<HashMap<i64, oneshot::Sender<Result<Vec<u8>, InvocationError>>>>,
        > = Arc::new(Mutex::new(HashMap::new()));

        // Channel the reconnect logic uses to hand a new read half to the reader task.
        // Capacity 4: only one reconnect can be in flight; bound prevents unbounded growth.
        let (reconnect_tx, reconnect_rx) =
            mpsc::channel::<(OwnedReadHalf, FrameKind, [u8; 256], i64)>(4);

        // Channel for external "network restored" hints: lets Android/iOS callbacks
        // skip the reconnect backoff and attempt immediately.
        // Capacity 4: hints are best-effort; a full channel means one is already queued.
        let (network_hint_tx, network_hint_rx) = mpsc::channel::<()>(4);

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
            transport: config.transport,
            session_backend: config.session_backend,
            dc_pool: Mutex::new(pool),
            transfer_pool: Mutex::new(transfer_pool),
            update_tx,
            is_bot: std::sync::atomic::AtomicBool::new(false),
            worker_semaphore: Arc::new(tokio::sync::Semaphore::new(
                crate::media::MAX_GLOBAL_SENDERS,
            )),
            stream_active: std::sync::atomic::AtomicBool::new(false),
            salt_request_in_flight: std::sync::atomic::AtomicBool::new(false),
            diff_in_flight: std::sync::atomic::AtomicBool::new(false),
            dh_in_progress: std::sync::atomic::AtomicBool::new(false),
            pfs_enabled: config.use_pfs,
            signed_in: std::sync::atomic::AtomicBool::new(false),
            dc_connect_gates: parking_lot::Mutex::new(std::collections::HashMap::new()),
            auth_import_gates: parking_lot::Mutex::new(std::collections::HashMap::new()),
            auth_imported: parking_lot::Mutex::new(std::collections::HashSet::new()),
            // Persistent dedup ring for the main connection reader task.
            seen_msg_ids: ferogram_mtproto::new_seen_msg_ids(),
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

        // +: Background ack flush task  - drains pending_ack every 500 ms so that
        // content-message acks are never held indefinitely waiting for an outgoing
        // RPC.  Without this, a bot that receives update bursts without sending any
        // RPCs will eventually exhaust Telegram's un-acked-message threshold (~512)
        // causing the server to close the connection.
        {
            let client_ack = client.clone();
            let shutdown_ack = shutdown_token.clone();
            tokio::spawn(async move {
                let mut interval = tokio::time::interval(Duration::from_millis(500));
                interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
                loop {
                    tokio::select! {
                        _ = shutdown_ack.cancelled() => break,
                        _ = interval.tick() => {}
                    }
                    // Drain under writer lock; skip the send entirely if empty.
                    let acks: Vec<i64> = {
                        let mut w = client_ack.inner.writer.lock().await;
                        if w.pending_ack.is_empty() {
                            continue;
                        }
                        w.pending_ack.drain(..).collect()
                    };
                    // Pack a standalone msgs_ack frame (non-content-related, no
                    // sent_bodies entry needed  - the server never acks an ack).
                    let (wire, fk) = {
                        let mut w = client_ack.inner.writer.lock().await;
                        let ack_body = build_msgs_ack_body(&acks);
                        let (wire, _msg_id) = w.enc.pack_body_with_msg_id(&ack_body, false);
                        (wire, w.frame_kind.clone())
                    };
                    send_frame_write(&mut *client_ack.inner.write_half.lock().await, &wire, &fk)
                        .await
                        .ok(); // TCP error here will surface on the next real send
                }
            });
        }

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
                client.inner.pending.lock().await.clear();

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

                // Split first so we can read the new key/salt from the writer.
                let (new_writer, new_wh, new_read, new_fk): (
                    ferogram_connect::ConnectionWriter,
                    tokio::net::tcp::OwnedWriteHalf,
                    tokio::net::tcp::OwnedReadHalf,
                    ferogram_connect::connection::FrameKind,
                ) = new_conn.into_writer();
                // Update ONLY the home DC entry: all other DC keys are preserved.
                {
                    let mut opts_guard: tokio::sync::MutexGuard<
                        '_,
                        std::collections::HashMap<i32, DcEntry>,
                    > = client.inner.dc_options.lock().await;
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
                    .try_send((new_read, new_fk, new_ak, new_sid));
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
                        cache.channels.entry(p.id).or_insert(p.access_hash);
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
            let c = client.clone();
            tokio::spawn(async move {
                tracing::info!("[ferogram] catch_up: driving MessageBoxes diff");
                c.run_pending_differences().await;
            });
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
        tracing::debug!("[ferogram] Fresh connect to DC{dc_id} …");
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

        let writer_guard = self.inner.writer.lock().await;
        let home_dc_id = *self.inner.home_dc_id.lock().await;
        let dc_options: tokio::sync::MutexGuard<'_, std::collections::HashMap<i32, DcEntry>> =
            self.inner.dc_options.lock().await;

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
                });
            }
            for (&id, &hash) in &cache.channels {
                v.push(CachedPeer {
                    id,
                    access_hash: hash,
                    is_channel: true,
                    is_chat: false,
                });
            }
            for &id in &cache.chats {
                v.push(CachedPeer {
                    id,
                    access_hash: 0,
                    is_channel: false,
                    is_chat: true,
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
                Ok(false)
            }
            Err(e) => Err(e),
        }
    }

    /// Return an [`UpdateStream`] that yields incoming [`Update`]s.
    ///
    /// The reader task (started inside `connect()`) sends all updates to
    /// `inner.update_tx`. This method proxies those updates into a fresh
    /// caller-owned channel: typically called once per bot/app loop.
    pub fn stream_updates(&self) -> UpdateStream {
        // Guard: only one UpdateStream is supported per Client clone group.
        // A second call would compete with the first for updates, causing
        // non-deterministic splitting. Log an error and return a closed stream
        // so the caller's `while let Some(upd) = stream.next().await` exits
        // immediately rather than panicking the process.
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
            // Immediately drop the sender so the receiver yields None on first poll.
            let (_dead_tx, rx) = mpsc::channel::<update::Update>(1);
            return UpdateStream { rx };
        }
        // Bounded at 2048: if the handler is slower than the server sends,
        // back-pressure propagates to the internal reader instead of OOM-ing.
        let (caller_tx, rx) = mpsc::channel::<update::Update>(2048);
        let internal_rx = self._update_rx.clone();
        tokio::spawn(async move {
            let mut guard = internal_rx.lock().await;
            while let Some(upd) = guard.recv().await {
                // try_send: if the caller channel is full, drop the update and
                // log a warning rather than blocking or OOM-ing.
                if caller_tx.try_send(upd).is_err() {
                    tracing::warn!(
                        "[ferogram] update dropped: UpdateStream consumer is too slow \
                         (channel full at capacity 2048). Consider processing updates \
                         faster or spawning handlers."
                    );
                    metrics::counter!("ferogram.updates_dropped").increment(1);
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
    async fn run_reader_task(
        &self,
        read_half: OwnedReadHalf,

        frame_kind: FrameKind,
        auth_key: [u8; 256],

        session_id: i64,
        mut new_conn_rx: mpsc::Receiver<(OwnedReadHalf, FrameKind, [u8; 256], i64)>,

        mut network_hint_rx: mpsc::Receiver<()>,
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
            {
                let mut w = self.inner.writer.lock().await;
                w.sent_bodies.clear();
                w.container_map.clear();
            }

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
            let _utx = self.inner.update_tx.clone();
            tokio::spawn(async move {
                // Respect FLOOD_WAIT (same as do_reconnect_loop).
                let result = loop {
                    match c.init_connection().await {
                        Ok(()) => break Ok(()),
                        Err(InvocationError::Rpc(ref r)) if r.flood_wait_seconds().is_some() => {
                            let secs = r.flood_wait_seconds().expect("is FLOOD_WAIT error");
                            tracing::warn!(
                                "[ferogram] Supervisor init_connection FLOOD_WAIT_{secs}: waiting"
                            );
                            sleep(Duration::from_secs(secs + 1)).await;
                        }
                        Err(e) => break Err(e),
                    }
                };
                if result.is_ok() {
                    // After fresh DH, retry GetState with backoff instead of a fixed 2 s sleep.
                    if c.inner
                        .dh_in_progress
                        .load(std::sync::atomic::Ordering::SeqCst)
                    {
                        c.sync_state_after_dh().await;
                    }
                    // Signal reconnect to message_box so it queues getDifference.
                    {
                        let mut mb = c.inner.message_box.lock().await;
                        let _ = mb.process_updates(message_box::UpdatesLike::ConnectionClosed);
                    }
                    c.run_pending_differences().await;
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
        initial_init_rx: Option<oneshot::Receiver<Result<(), InvocationError>>>,
        new_conn_rx: &mut mpsc::Receiver<(OwnedReadHalf, FrameKind, [u8; 256], i64)>,

        network_hint_rx: &mut mpsc::Receiver<()>,
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

                // check_deadlines() returns now() when difference is pending, so this
                // arm fires immediately when a gap was detected; otherwise it sleeps
                // until the next no-update timeout (15 min default).
                _ = tokio::time::sleep_until({
                    let mut mb = self.inner.message_box.lock().await;
                    mb.check_deadlines().into()
                }) => {
                    // IMPORTANT: never await run_pending_differences() directly here.
                    // This arm runs inside the reader_loop select!, which is the only
                    // task reading TCP frames and routing RPC responses.  If we await
                    // run_pending_differences() inline, the getDifference RPC is sent
                    // but the response frame can never arrive (reader_loop is blocked)
                    // → 30-second self-deadlock.  Spawn a separate task instead, exactly
                    // like the Keepalive arm below.  run_pending_differences() owns the
                    // diff_in_flight guard internally, so concurrent spawns are safe.
                    //
                    // Guard: when diff_in_flight is already true (a getDifference RPC is
                    // in progress), skip the spawn entirely.  Without this guard, the
                    // deadline arm fires on every select iteration (check_deadlines()
                    // returns Instant::now() while getting_diff_for is non-empty) and
                    // spawns hundreds of tasks per second that all hit the in-flight flag
                    // and log "diff already in flight".
                    if !self.inner.diff_in_flight.load(std::sync::atomic::Ordering::Acquire) {
                        let c = self.clone();
                        tokio::spawn(async move {
                            c.run_pending_differences().await;
                        });
                    }
                }

                _ = async {
                    if let Some(ref mut i) = restart_interval { i.tick().await; }
                    else { std::future::pending::<()>().await; }
                } => {
                    tracing::info!("[ferogram] scheduled restart: reconnecting");
                    let _ = self.inner.write_half.lock().await.shutdown().await;
                    let _ = self.inner.network_hint_tx.try_send(());
                }
                // Normal frame (or application-level keepalive timeout)
                outcome = recv_frame_with_keepalive(&mut rh, &fk, &self.inner.writer, &self.inner.write_half) => {
                    match outcome {
                        FrameOutcome::Frame(mut raw) => {
                            let msg = match EncryptedSession::decrypt_frame_dedup(&ak, sid, &mut raw, &self.inner.seen_msg_ids) {
                                Ok(m)  => m,
                                Err(e) => {
                                    // A decrypt failure (e.g. Crypto(InvalidBuffer) from a
                                    // 4-byte transport error that slipped through) means our
                                    // auth key is stale or the framing is broken. Treat it as
                                    // Fatal: unblock pending RPCs immediately.
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
                                    {
                                        let mut w = self.inner.writer.lock().await;
                                        w.sent_bodies.clear();
                                        w.container_map.clear();
                                    }
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
                            let ie: InvocationError = InvocationError::from(e);
                            tracing::warn!("[ferogram] Reader: connection error: {ie}");
                            drop(init_rx.take()); // discard any in-flight init

                            // Detect definitive auth-key rejection.  Telegram signals
                            // this with a -404 transport error (now surfaced as Rpc(-404)
                            // by recv_frame_read).  ONLY in that case do we clear the saved
                            // key so do_reconnect_loop falls through to connect_raw (fresh DH).
                            //
                            // DO NOT treat UnexpectedEof or ConnectionReset as stale-key:
                            // those are normal TCP disconnects (server-side timeout, network
                            // blip, download finishing on a transfer conn, etc.).  Auth keys
                            // live for months  - clearing them on every TCP drop destroys the
                            // session and produces AUTH_KEY_UNREGISTERED on the next connect.
                            let key_is_stale = matches!(&ie, InvocationError::Rpc(r) if r.code == -404);
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
                                let mut opts: tokio::sync::MutexGuard<'_, std::collections::HashMap<i32, DcEntry>> = self.inner.dc_options.lock().await;
                                if let Some(entry) = opts.get_mut(&home_dc_id) {
                                    tracing::warn!(
                                        "[ferogram] Stale auth key on DC{home_dc_id} ({ie}) \
                                        : clearing for fresh DH"
                                    );
                                    entry.auth_key = None;
                                }
                            }

                            // Fail all in-flight RPCs immediately so AutoSleep
                            // retries them as soon as we reconnect.
                            {
                                let mut pending = self.inner.pending.lock().await;
                                let msg = ie.to_string();
                                for (_, tx) in pending.drain() {
                                    let _ = tx.send(Err(InvocationError::Io(
                                        std::io::Error::new(
                                            std::io::ErrorKind::ConnectionReset, msg.clone()))));
                                }
                            }
                            // drain sent_bodies so it doesn't grow unbounded under loss.
                            {
                                let mut w = self.inner.writer.lock().await;
                                w.sent_bodies.clear();
                                w.container_map.clear();
                            }

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
                            // Drive any pending diff on keepalive (gap may have been buffered).
                            let c = self.clone();
                            tokio::spawn(async move {
                                c.run_pending_differences().await;
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
                init_result = async { init_rx.as_mut().expect("guarded by is_some()").await }, if init_rx.is_some() => {
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
                            let key_is_stale = matches!(&e, InvocationError::Rpc(r) if r.code == -404);
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
                                let mut opts: tokio::sync::MutexGuard<'_, std::collections::HashMap<i32, DcEntry>> = self.inner.dc_options.lock().await;
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
                            // Return the first body to the invoke() caller so it can
                            // deserialize the Updates response (fixes "unexpected end of
                            // buffer" on EditMessage, ForwardMessages, SendMedia, etc.).
                            // Concurrently dispatch ALL bodies through pts/seq tracking so
                            // gaps are filled and the update stream stays consistent.
                            let caller_body = bodies.first().cloned().unwrap_or_default();
                            let c = self.clone();
                            tokio::spawn(async move {
                                for body in bodies {
                                    c.dispatch_updates(&body).await;
                                }
                            });
                            Ok(caller_body)
                        }
                        Ok(EnvelopeResult::Pts(_pts, _pts_count)) => {
                            // updateShortSentMessage as RPC response: signal message_box
                            // that state may have advanced and let getDifference reconcile.
                            let c = self.clone();
                            tokio::spawn(async move {
                                {
                                    let mut mb = c.inner.message_box.lock().await;
                                    let _ = mb.process_updates(
                                        message_box::UpdatesLike::ConnectionClosed,
                                    );
                                }
                                c.run_pending_differences().await;
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
                    let _ = tx.send(to_send.map_err(InvocationError::from));
                }
            }
            ID_RPC_ERROR => {
                tracing::warn!("[ferogram] Unexpected top-level rpc_error (no pending target)");
            }
            ID_MSG_CONTAINER => {
                if body.len() < 8 {
                    return;
                }
                // MTProto spec max: 1020 items per container.
                const MAX_CONTAINER_ITEMS: usize = 1020;
                let count = (u32::from_le_bytes(body[4..8].try_into().unwrap()) as usize)
                    .min(MAX_CONTAINER_ITEMS);
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
                    // MTProto spec forbids nested containers; drop silently.
                    if inner.len() >= 4 {
                        let inner_cid = u32::from_le_bytes(inner[..4].try_into().unwrap());
                        if inner_cid == ID_MSG_CONTAINER {
                            tracing::warn!(
                                "[ferogram] dropping nested msg_container (proto violation)"
                            );
                            metrics::counter!("ferogram.updates_dropped").increment(1);
                            continue;
                        }
                    }
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
                    // Propagate to dc_options snapshot so future worker opens see
                    // the fresh salt immediately (not just after the next reconnect).
                    {
                        let home_id = *self.inner.home_dc_id.lock().await;
                        let mut opts: tokio::sync::MutexGuard<
                            '_,
                            std::collections::HashMap<i32, DcEntry>,
                        > = self.inner.dc_options.lock().await;
                        if let Some(e) = opts.get_mut(&home_id) {
                            e.first_salt = new_salt;
                        }
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
                        // Wire format per TL schema (bare FutureSalt, no constructor):
                        // [+0..+4]   valid_since (int)
                        // [+4..+8]   valid_until (int)
                        // [+8..+16]  salt        (long)
                        // This matches the official TL definition:
                        //   futureSalt#0949d9dc valid_since:int valid_until:int salt:long
                        // futureSalt layout: valid_since, valid_until, salt
                        new_salts.push(FutureSalt {
                            valid_since: i32::from_le_bytes(
                                body[base..base + 4].try_into().unwrap(),
                            ),
                            valid_until: i32::from_le_bytes(
                                body[base + 4..base + 8].try_into().unwrap(),
                            ),
                            salt: i64::from_le_bytes(body[base + 8..base + 16].try_into().unwrap()),
                        });
                    }

                    if !new_salts.is_empty() {
                        // Sort newest-last (mirrors  sort_by_key(|s| -s.valid_since)
                        // which in ascending order puts highest valid_since at the end).
                        new_salts.sort_by_key(|s| s.valid_since);
                        let active_salt;
                        {
                            let mut w = self.inner.writer.lock().await;
                            w.salts = new_salts;
                            w.start_salt_time = Some((server_now, std::time::Instant::now()));

                            // Pick the best currently-usable salt.
                            // A salt is usable after valid_since + SALT_USE_DELAY (60 s)
                            // AND must not yet be expired (valid_until > server_now).
                            //
                            // CRITICAL: do NOT fall back to an expired salt via
                            // `.or_else(|| w.salts.first())`.  When the server returns
                            // an all-expired pool (e.g. stale DC handoff), enc.salt
                            // already holds the server-canonical value from
                            // new_session_created or bad_server_salt and must be kept.
                            // Overwriting it with an expired salt causes every subsequent
                            // message to be rejected → bad_server_salt → GetFutureSalts
                            // → same expired pool → infinite loop.
                            let use_salt = w
                                .salts
                                .iter()
                                .rev()
                                .find(|s| {
                                    s.valid_since + SALT_USE_DELAY <= server_now
                                        && s.valid_until > server_now
                                })
                                .map(|s| s.salt);
                            if let Some(salt) = use_salt {
                                w.enc.salt = salt;
                                tracing::debug!(
                                    "[ferogram] FutureSalts: stored {} salts, \
                                     active salt={salt:#x}",
                                    w.salts.len()
                                );
                            } else {
                                tracing::debug!(
                                    "[ferogram] FutureSalts: stored {} salts but all \
                                     expired  - keeping current enc.salt={:#x}",
                                    w.salts.len(),
                                    w.enc.salt
                                );
                            }
                            active_salt = use_salt;
                        }
                        // Propagate the newly-active salt to dc_options so that any
                        // worker conn opened after this FutureSalts rotation starts
                        // with the correct salt rather than the pre-rotation snapshot.
                        if let Some(salt) = active_salt {
                            let home_id = *self.inner.home_dc_id.lock().await;
                            let mut opts: tokio::sync::MutexGuard<
                                '_,
                                std::collections::HashMap<i32, DcEntry>,
                            > = self.inner.dc_options.lock().await;
                            if let Some(e) = opts.get_mut(&home_id) {
                                e.first_salt = salt;
                            }
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
                //   body[4..12]  = first_msg_id  <- msgs with msg_id < this were NOT received
                //   body[12..20] = unique_id
                //   body[20..28] = server_salt
                if body.len() >= 28 {
                    let first_msg_id = i64::from_le_bytes(body[4..12].try_into().unwrap());
                    let server_salt = i64::from_le_bytes(body[20..28].try_into().unwrap());
                    {
                        let mut w = self.inner.writer.lock().await;
                        // new_session_created has odd seq_no -> must ack.
                        w.pending_ack.push(msg_id);
                        w.salts.clear();
                        w.salts.push(FutureSalt {
                            valid_since: 0,
                            valid_until: i32::MAX,
                            salt: server_salt,
                        });
                        w.enc.salt = server_salt;
                        tracing::debug!(
                            "[ferogram] new_session_created: salt={server_salt:#x}                              first_msg_id={first_msg_id}"
                        );
                        // MTProto: msgs with msg_id < first_msg_id were not received
                        // by the server and must be re-sent.
                        let stale_ids: Vec<i64> = w
                            .sent_bodies
                            .keys()
                            .filter(|&&id| id < first_msg_id)
                            .copied()
                            .collect();
                        if !stale_ids.is_empty() {
                            tracing::debug!(
                                "[ferogram] new_session_created: re-queuing {} stale msg(s)                                  with msg_id < {first_msg_id}",
                                stale_ids.len()
                            );
                        }
                        for old_id in stale_ids {
                            if let Some(body_bytes) = w.sent_bodies.remove(&old_id) {
                                let (wire, new_id) = w.enc.pack_body_with_msg_id(&body_bytes, true);
                                w.sent_bodies.insert(new_id, body_bytes);
                                // Defer TCP send to after lock drop.
                                w.new_session_resend_queue.push((old_id, new_id, wire));
                            }
                        }
                    }
                    // Ship resends outside the writer lock (no TCP I/O under lock).
                    let resend_queue: Vec<(i64, i64, Vec<u8>)> = {
                        let mut w = self.inner.writer.lock().await;
                        std::mem::take(
                            &mut w.new_session_resend_queue as &mut Vec<(i64, i64, Vec<u8>)>,
                        )
                    };
                    if !resend_queue.is_empty() {
                        let fk = self.inner.writer.lock().await.frame_kind.clone();
                        for (old_id, new_id, wire) in resend_queue {
                            {
                                let mut pending = self.inner.pending.lock().await;
                                if let Some(tx) = pending.remove(&old_id) {
                                    pending.insert(new_id, tx);
                                }
                            }
                            if let Err(e) = send_frame_write(
                                &mut *self.inner.write_half.lock().await,
                                &wire,
                                &fk,
                            )
                            .await
                            {
                                tracing::warn!(
                                    "[ferogram] new_session resend {old_id}->{new_id} failed: {e}"
                                );
                                self.inner.writer.lock().await.sent_bodies.remove(&new_id);
                            } else {
                                tracing::debug!(
                                    "[ferogram] new_session resend {old_id}->{new_id} ok"
                                );
                            }
                        }
                    }
                    // Propagate to dc_options snapshot so future worker opens use
                    // this session's salt, not the stale pre-session value.
                    {
                        let home_id = *self.inner.home_dc_id.lock().await;
                        let mut opts: tokio::sync::MutexGuard<
                            '_,
                            std::collections::HashMap<i32, DcEntry>,
                        > = self.inner.dc_options.lock().await;
                        if let Some(e) = opts.get_mut(&home_id) {
                            e.first_salt = server_salt;
                        }
                    }
                    // Signal message_box that the connection closed (gap may exist).
                    {
                        let mut mb = self.inner.message_box.lock().await;
                        let _ = mb.process_updates(message_box::UpdatesLike::ConnectionClosed);
                    }
                    let c = self.clone();
                    let _handle = tokio::spawn(async move {
                        c.sync_state_after_dh().await;
                    });
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
                // writer→pending lock-order deadlock.
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
                        // On TCP send failure, remove the orphaned sent_bodies entry.
                        if let Err(e) =
                            send_frame_write(&mut *self.inner.write_half.lock().await, &wire, &fk)
                                .await
                        {
                            self.inner.writer.lock().await.sent_bodies.remove(&new_id);
                            if let Some(tx) = self.inner.pending.lock().await.remove(&new_id) {
                                let _ = tx.send(Err(e.into())); // convert ConnectError → InvocationError
                            }
                            tracing::warn!(
                                "[ferogram] MsgResendReq: TCP send failed for {resend_id} -> {new_id}"
                            );
                        } else {
                            tracing::debug!(
                                "[ferogram] MsgResendReq: resent {resend_id} -> {new_id}"
                            );
                        }
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
                // Route through pts/qts/seq gap-checkers.
                self.dispatch_updates(&body).await;
            }
            _ => {}
        }
    }

    /// Extract the pts-sort key for a single update: `pts - pts_count`.
    ///
    ///sorts every update batch by this key before processing.
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
                c.run_pending_differences().await;
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
                    }
                    // Convert and emit each approved update.
                    for raw in raw_updates {
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

    /// Retry `sync_pts_state` with exponential backoff after a fresh DH exchange.
    ///
    /// After PFS re-keying the server may return `AUTH_KEY_UNREGISTERED` for a
    /// short window while it propagates the new key.  Retry up to five times.
    pub(crate) async fn sync_state_after_dh(&self) {
        if !self
            .inner
            .signed_in
            .load(std::sync::atomic::Ordering::SeqCst)
        {
            tracing::debug!("[ferogram] sync_state_after_dh: not signed in yet - skipping");
            return;
        }
        for delay_ms in [0u64, 100, 300, 700, 1500] {
            if delay_ms > 0 {
                tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;
            }
            match self.sync_pts_state().await {
                Ok(()) => return,
                Err(ref e) if matches!(e, InvocationError::Rpc(r) if r.code == 401) => {
                    tracing::debug!(
                        "[ferogram] sync_state_after_dh: AUTH_KEY_UNREGISTERED \
                         (delay={delay_ms}ms), retrying"
                    );
                    continue;
                }
                Err(e) => {
                    tracing::warn!("[ferogram] sync_state_after_dh failed: {e}");
                    return;
                }
            }
        }
        tracing::warn!("[ferogram] sync_state_after_dh: all retries exhausted");
    }

    /// Drive all pending `getDifference` / `getChannelDifference` calls queued by
    /// [`MessageBoxes`].  Called from the deadline select arm and after reconnect.
    ///
    /// Internally acquires `diff_in_flight` so concurrent callers (reconnect, catch-up,
    /// deadline arm) are serialised - only one getDifference is ever in flight at a time.
    async fn run_pending_differences(&self) {
        use crate::message_box::PrematureEndReason;

        // Acquire the single-flight guard.  Any concurrent caller (direct or spawned)
        // exits immediately rather than issuing a second concurrent getDifference.
        if self
            .inner
            .diff_in_flight
            .compare_exchange(
                false,
                true,
                std::sync::atomic::Ordering::AcqRel,
                std::sync::atomic::Ordering::Acquire,
            )
            .is_err()
        {
            tracing::debug!("[ferogram] diff already in flight, skipping concurrent call");
            return;
        }
        // RAII guard: always resets diff_in_flight=false when we exit, including on panic.
        struct DiffGuard(crate::Client);
        impl Drop for DiffGuard {
            fn drop(&mut self) {
                self.0
                    .inner
                    .diff_in_flight
                    .store(false, std::sync::atomic::Ordering::Release);
            }
        }
        let _guard = DiffGuard(self.clone());

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
                        tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                        continue; // let channel diffs run even when common diff RPC fails
                    }
                }
                continue;
            }

            let get_chan_req = self.inner.message_box.lock().await.get_channel_difference();
            if let Some((channel_id, mut req)) = get_chan_req {
                let access_hash = {
                    let cache: tokio::sync::RwLockReadGuard<'_, PeerCache> =
                        self.inner.peer_cache.read().await;
                    cache.channels.get(&channel_id).copied().unwrap_or(0)
                };
                if access_hash == 0 {
                    tracing::warn!(
                        "[ferogram] no access_hash for channel {channel_id}; ending diff (Banned)"
                    );
                    self.inner
                        .message_box
                        .lock()
                        .await
                        .end_channel_difference(PrematureEndReason::Banned);
                    continue;
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

        network_hint_rx: &mut mpsc::Receiver<()>,
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
            metrics::counter!("ferogram.reconnects_total").increment(1);
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
                    let _utx = self.inner.update_tx.clone();
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
                                    let secs = r.flood_wait_seconds().expect("is FLOOD_WAIT error");
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
                            // After fresh DH, retry GetState with backoff instead of a fixed 2 s sleep.
                            if c.inner
                                .dh_in_progress
                                .load(std::sync::atomic::Ordering::SeqCst)
                            {
                                c.sync_state_after_dh().await;
                            }
                            // Signal reconnect to message_box then drive diff.
                            {
                                let mut mb = c.inner.message_box.lock().await;
                                let _ =
                                    mb.process_updates(message_box::UpdatesLike::ConnectionClosed);
                            }
                            c.run_pending_differences().await;
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
            let opts: tokio::sync::MutexGuard<'_, std::collections::HashMap<i32, DcEntry>> =
                self.inner.dc_options.lock().await;
            match opts.get(&home_dc_id) {
                Some(e) => (e.addr.clone(), e.auth_key, e.first_salt, e.time_offset),
                None => (fallback_dc_addr(home_dc_id).to_string(), None, 0, 0),
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
                mtproxy.as_ref(),
                &transport,
                home_dc_id as i16,
                self.inner.pfs_enabled,
            )
            .await
            {
                Ok(c) => c,
                Err(e) => {
                    return Err(e.into());
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

        let (new_writer, new_wh, new_read, new_fk): (
            ferogram_connect::ConnectionWriter,
            tokio::net::tcp::OwnedWriteHalf,
            tokio::net::tcp::OwnedReadHalf,
            ferogram_connect::connection::FrameKind,
        ) = new_conn.into_writer();
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
            let mut opts: tokio::sync::MutexGuard<'_, std::collections::HashMap<i32, DcEntry>> =
                self.inner.dc_options.lock().await;
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

    pub(crate) async fn send_message_impl(
        &self,
        peer: impl Into<PeerRef>,

        msg: impl Into<InputMessage>,
    ) -> Result<update::IncomingMessage, InvocationError> {
        let msg = msg.into();
        let msg = &msg;
        let peer = peer.into().resolve(self).await?;
        let input_peer = self.inner.peer_cache.read().await.peer_to_input(&peer)?;
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
            let body: Vec<u8> = self.rpc_call_raw(&req).await?;
            return Ok(self.parse_send_response(&body, msg, &peer).await);
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
        let body: Vec<u8> = self.rpc_call_raw(&req).await?;
        Ok(self.parse_send_response(&body, msg, &peer).await)
    }

    pub async fn send_message(
        &self,
        peer: impl Into<PeerRef>,
        msg: impl Into<InputMessage>,
    ) -> Result<update::IncomingMessage, InvocationError> {
        self.send_message_impl(peer, msg).await
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
        if cid == 0x9015e101 {
            let mut cur = Cursor::from_slice(&body[4..]);
            if let Ok(sent) = tl::types::UpdateShortSentMessage::deserialize(&mut cur) {
                return self.synthetic_sent_from_short(input, peer, sent.id, sent.date);
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

    fn synthetic_sent_from_short(
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

    #[allow(dead_code)]
    async fn synthetic_sent(
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

    pub async fn open_mini_app(
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
        };
        let body: Vec<u8> = self.rpc_call_raw(&req).await?;
        let self_peer = tl::enums::Peer::User(tl::types::PeerUser { user_id: 0 });
        Ok(self.parse_send_response(&body, &msg, &self_peer).await)
    }

    pub async fn send_to_self(
        &self,
        peer: impl Into<PeerRef>,

        message_id: i32,
        msg: impl Into<InputMessage>,
    ) -> Result<(), InvocationError> {
        let msg = msg.into();
        let peer = peer.into().resolve(self).await?;
        let input_peer = self.inner.peer_cache.read().await.peer_to_input(&peer)?;
        let req = tl::functions::messages::EditMessage {
            no_webpage: msg.no_webpage,
            invert_media: msg.invert_media,
            peer: input_peer,
            id: message_id,
            message: Some(msg.text),
            media: msg.media,
            reply_markup: msg.reply_markup,
            entities: msg.entities,
            schedule_date: msg.schedule_date,
            schedule_repeat_period: None,
            quick_reply_shortcut_id: None,
        };
        self.rpc_write(&req).await
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
    pub async fn get_message_by_id(
        &self,
        peer: impl Into<PeerRef>,
        id: i32,
    ) -> Result<Option<update::IncomingMessage>, InvocationError> {
        let peer = peer.into().resolve(self).await?;
        let input_peer = self.inner.peer_cache.read().await.peer_to_input(&peer)?;
        let ids = vec![tl::enums::InputMessage::Id(tl::types::InputMessageId {
            id,
        })];
        let body: Vec<u8> = match &input_peer {
            tl::enums::InputPeer::Channel(c) => {
                let req = tl::functions::channels::GetMessages {
                    channel: tl::enums::InputChannel::InputChannel(tl::types::InputChannel {
                        channel_id: c.channel_id,
                        access_hash: c.access_hash,
                    }),
                    id: ids,
                };
                self.rpc_call_raw(&req).await?
            }
            _ => {
                let req = tl::functions::messages::GetMessages { id: ids };
                self.rpc_call_raw(&req).await?
            }
        };
        let mut cur = Cursor::from_slice(&body);
        let msgs = match tl::enums::messages::Messages::deserialize(&mut cur) {
            Ok(tl::enums::messages::Messages::Messages(m)) => m.messages,
            Ok(tl::enums::messages::Messages::Slice(m)) => m.messages,
            Ok(tl::enums::messages::Messages::ChannelMessages(m)) => m.messages,
            _ => return Ok(None),
        };
        Ok(msgs
            .into_iter()
            .next()
            .map(|m| update::IncomingMessage::from_raw(m).with_client(self.clone())))
    }

    pub async fn get_messages_by_id(
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

    pub async fn pin_message(
        &self,
        peer: impl Into<PeerRef>,
        message_id: i32,
        silent: bool,
    ) -> Result<(), InvocationError> {
        self.update_pinned_message(peer, message_id, silent, false, false)
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

    pub async fn unpin_message(
        &self,
        peer: impl Into<PeerRef>,
        message_id: i32,
    ) -> Result<(), InvocationError> {
        self.update_pinned_message(peer, message_id, true, true, false)
            .await
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

    pub async fn download_file(
        &self,
        location: tl::enums::InputFileLocation,

        path: impl AsRef<std::path::Path>,
    ) -> Result<(), InvocationError> {
        self.download_media_to_file_on_dc(location, 0, path).await
    }

    pub async fn download_file_to_path(
        &self,
        location: tl::enums::InputFileLocation,

        dc_id: i32,
        path: impl AsRef<std::path::Path>,
    ) -> Result<(), InvocationError> {
        let bytes = self.download_media_on_dc(location, dc_id).await?;
        tokio::fs::write(path, &bytes)
            .await
            .map_err(InvocationError::Io)?;
        Ok(())
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

    /// Resolve a peer string to a [`tl::enums::Peer`].
    ///
    /// Accepts: `"me"`, `"self"`, `"@username"`, `"username"`, numeric strings
    /// (Bot-API encoded), `t.me/` URLs, E.164 phones (`+digits`), and invite
    /// links.  Cache-first for usernames and phones; RPC only on miss.
    ///
    /// Prefer [`Client::resolve`] when the input may not be a string.
    pub async fn resolve_peer(&self, peer: &str) -> Result<tl::enums::Peer, InvocationError> {
        PeerRef::from(peer).resolve(self).await
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
    pub async fn join_by_invite(
        &self,
        link: &str,
    ) -> Result<tl::enums::InputPeer, InvocationError> {
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
                    if let Some(&hash) = cache.channels.get(&c.id) {
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

        // Split the new connection and replace writer + read half.
        let (new_writer, new_wh, new_read, new_fk): (
            ferogram_connect::ConnectionWriter,
            tokio::net::tcp::OwnedWriteHalf,
            tokio::net::tcp::OwnedReadHalf,
            ferogram_connect::connection::FrameKind,
        ) = conn.into_writer();
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
            .try_send((new_read, new_fk, new_ak, new_sid));

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

    /// Sync the internal pts/qts/seq/date state with the Telegram server.
    ///
    /// Called automatically on `connect()`. Call it manually if you
    /// need to reset the update gap-detection counters, e.g. after resuming
    /// from a long hibernation.
    async fn cache_user(&self, user: &tl::enums::User) {
        self.inner.peer_cache.write().await.cache_user(user);
    }

    pub(crate) async fn cache_users_slice(&self, users: &[tl::enums::User]) {
        let mut cache: tokio::sync::RwLockWriteGuard<'_, PeerCache> =
            self.inner.peer_cache.write().await;
        cache.cache_users(users);
    }

    pub(crate) async fn cache_chats_slice(&self, chats: &[tl::enums::Chat]) {
        let mut cache: tokio::sync::RwLockWriteGuard<'_, PeerCache> =
            self.inner.peer_cache.write().await;
        cache.cache_chats(chats);
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
                    let w = self.inner.writer.lock().await;
                    (w.first_salt(), w.time_offset())
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
                    {
                        {
                            let pool = self.inner.transfer_pool.lock().await;
                            if let Some(slots) = pool.conns.get(&target_dc)
                                && let Some(slot) = slots.first()
                                && let Ok(conn) = slot.conn.try_lock()
                            {
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
                                entry.auth_key = Some(conn.auth_key_bytes());
                                entry.first_salt = conn.first_salt();
                                entry.time_offset = conn.time_offset();
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
        // Evict dead connections on IO error or fatal RPC errors.
        match &result {
            Err(InvocationError::Io(_)) => {
                tracing::debug!(
                    "[ferogram] Transfer DC{target_dc} IO error  - evicting broken connection from pool"
                );
                self.inner.transfer_pool.lock().await.evict(target_dc);
            }
            Err(InvocationError::Rpc(rpc))
                if matches!(
                    rpc.name.as_str(),
                    "AUTH_KEY_UNREGISTERED"
                        | "SESSION_EXPIRED"
                        | "AUTH_KEY_INVALID"
                        | "AUTH_KEY_PERM_EMPTY"
                ) =>
            {
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
            _ => {}
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
                let w = self.inner.writer.lock().await;
                (w.first_salt(), w.time_offset())
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
                let opts: tokio::sync::MutexGuard<'_, std::collections::HashMap<i32, DcEntry>> =
                    self.inner.dc_options.lock().await;
                opts.get(&dc_id)
                    .map(|e| e.addr.clone())
                    .unwrap_or_else(|| fallback_dc_addr(dc_id).to_string())
            };

            let socks5 = self.inner.socks5.clone();
            let mtproxy = self.inner.mtproxy.clone();
            let transport = self.inner.transport.clone();
            let saved_key = {
                let opts: tokio::sync::MutexGuard<'_, std::collections::HashMap<i32, DcEntry>> =
                    self.inner.dc_options.lock().await;
                opts.get(&dc_id).and_then(|e| e.auth_key)
            };

            let dc_conn = if let Some(key) = saved_key {
                dc_pool::DcConnection::connect_with_key(
                    &addr,
                    key,
                    0,
                    0,
                    socks5.as_ref(),
                    mtproxy.as_ref(),
                    &transport,
                    dc_id as i16,
                    self.inner.pfs_enabled,
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
                let mut opts: tokio::sync::MutexGuard<'_, std::collections::HashMap<i32, DcEntry>> =
                    self.inner.dc_options.lock().await;
                if let Some(e) = opts.get_mut(&dc_id) {
                    e.auth_key = Some(key);
                }
            }
            self.inner.dc_pool.lock().await.insert(dc_id, dc_conn);
        }

        let dc_entries: Vec<crate::DcEntry> = self
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
        let body: Vec<u8> = self.rpc_call_raw(&export_req).await?;
        let mut cur = Cursor::from_slice(&body);
        let tl::enums::auth::ExportedAuthorization::ExportedAuthorization(exported) =
            tl::enums::auth::ExportedAuthorization::deserialize(&mut cur)?;

        // Import into the target DC via the pool
        let import_req = tl::functions::auth::ImportAuthorization {
            id: exported.id,
            bytes: exported.bytes,
        };
        let dc_entries: Vec<crate::DcEntry> = self
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
        };
        let body: Vec<u8> = self.rpc_call_raw(&req).await?;
        Ok(body.len() >= 4 && u32::from_le_bytes(body[..4].try_into().unwrap()) == 0x997275b5)
    }

    pub async fn send_dice(
        &self,
        peer: impl Into<PeerRef>,
        emoticon: impl Into<String>,
    ) -> Result<(), InvocationError> {
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
        self.rpc_call_raw(&req).await?;
        Ok(())
    }

    pub async fn send_poll(
        &self,
        peer: impl Into<PeerRef>,
        question: impl Into<String>,
        answers: &[impl AsRef<str>],
        quiz: bool,
        correct_index: Option<usize>,
        multiple_choice: bool,
    ) -> Result<(), InvocationError> {
        use ferogram_tl_types as tl;
        let peer = peer.into().resolve(self).await?;
        let input_peer = self.inner.peer_cache.read().await.peer_to_input(&peer)?;
        let poll_answers: Vec<tl::enums::PollAnswer> = answers
            .iter()
            .enumerate()
            .map(|(i, a)| {
                tl::enums::PollAnswer::PollAnswer(tl::types::PollAnswer {
                    text: tl::enums::TextWithEntities::TextWithEntities(
                        tl::types::TextWithEntities {
                            text: a.as_ref().to_owned(),
                            entities: vec![],
                        },
                    ),
                    option: vec![i as u8],
                    media: None,
                    added_by: None,
                    date: None,
                })
            })
            .collect();
        let correct_answers: Option<Vec<i32>> = if quiz {
            correct_index.map(|i| vec![i as i32])
        } else {
            None
        };
        let poll = tl::enums::Poll::Poll(tl::types::Poll {
            id: 0,
            closed: false,
            public_voters: false,
            multiple_choice: multiple_choice && !quiz,
            quiz,
            open_answers: false,
            revoting_disabled: false,
            shuffle_answers: false,
            hide_results_until_close: false,
            creator: false,
            question: tl::enums::TextWithEntities::TextWithEntities(tl::types::TextWithEntities {
                text: question.into(),
                entities: vec![],
            }),
            answers: poll_answers,
            close_period: None,
            close_date: None,
            hash: 0,
        });
        let media = tl::enums::InputMedia::Poll(Box::new(tl::types::InputMediaPoll {
            poll,
            correct_answers,
            attached_media: None,
            solution: None,
            solution_entities: None,
            solution_media: None,
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

    pub async fn set_bot_info(
        &self,
        name: Option<&str>,
        about: Option<&str>,
        description: Option<&str>,
        lang_code: &str,
    ) -> Result<bool, InvocationError> {
        let req = tl::functions::bots::SetBotInfo {
            bot: None,
            lang_code: lang_code.to_string(),
            name: name.map(|s| s.to_string()),
            about: about.map(|s| s.to_string()),
            description: description.map(|s| s.to_string()),
        };
        let body = self.rpc_call_raw(&req).await?;
        Ok(is_bool_true(&body))
    }

    pub async fn get_bot_info(
        &self,
        lang_code: &str,
    ) -> Result<tl::types::bots::BotInfo, InvocationError> {
        use ferogram_tl_types::{Cursor, Deserializable};
        let req = tl::functions::bots::GetBotInfo {
            bot: None,
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
    async fn get_dialogs_raw(
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
        let (dialogs_raw, users, chats, count) = match raw {
            tl::enums::messages::Dialogs::Dialogs(d) => (d.dialogs, d.users, d.chats, None),
            tl::enums::messages::Dialogs::Slice(d) => (d.dialogs, d.users, d.chats, Some(d.count)),
            tl::enums::messages::Dialogs::NotModified(d) => return Ok((vec![], Some(d.count))),
        };
        self.cache_users_and_chats(&users, &chats).await;
        let dialogs: Vec<Dialog> = dialogs_raw
            .into_iter()
            .map(|d| Dialog {
                raw: d,
                message: None,
                entity: None,
                chat: None,
            })
            .collect();
        Ok((dialogs, count))
    }

    #[allow(dead_code)]
    pub(crate) async fn get_chat_full(
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

    pub async fn download_media_to_file(
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

    pub(crate) async fn download_media_to_file_on_dc(
        &self,
        location: tl::enums::InputFileLocation,
        dc_id: i32,
        path: impl AsRef<std::path::Path>,
    ) -> Result<(), InvocationError> {
        use tokio::io::AsyncWriteExt;
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

    pub async fn edit_chat_title(
        &self,
        peer: impl Into<PeerRef>,
        title: impl Into<String>,
    ) -> Result<(), InvocationError> {
        let peer = peer.into().resolve(self).await?;
        let input_peer = self.inner.peer_cache.read().await.peer_to_input(&peer)?;
        let title = title.into();
        match &input_peer {
            tl::enums::InputPeer::Channel(c) => {
                let req = tl::functions::channels::EditTitle {
                    channel: tl::enums::InputChannel::InputChannel(tl::types::InputChannel {
                        channel_id: c.channel_id,
                        access_hash: c.access_hash,
                    }),
                    title,
                };
                self.rpc_write(&req).await
            }
            tl::enums::InputPeer::Chat(c) => {
                let req = tl::functions::messages::EditChatTitle {
                    chat_id: c.chat_id,
                    title,
                };
                self.rpc_write(&req).await
            }
            _ => Err(InvocationError::Deserialize(
                "edit_chat_title: peer must be a chat or channel".into(),
            )),
        }
    }

    pub async fn edit_chat_about(
        &self,
        peer: impl Into<PeerRef>,
        about: impl Into<String>,
    ) -> Result<(), InvocationError> {
        let peer = peer.into().resolve(self).await?;
        let input_peer = self.inner.peer_cache.read().await.peer_to_input(&peer)?;
        let req = tl::functions::messages::EditChatAbout {
            peer: input_peer,
            about: about.into(),
        };
        self.rpc_write(&req).await
    }

    pub async fn edit_chat_photo(
        &self,
        peer: impl Into<PeerRef>,
        photo: tl::enums::InputChatPhoto,
    ) -> Result<(), InvocationError> {
        let peer = peer.into().resolve(self).await?;
        let input_peer = self.inner.peer_cache.read().await.peer_to_input(&peer)?;
        match &input_peer {
            tl::enums::InputPeer::Channel(c) => {
                let req = tl::functions::channels::EditPhoto {
                    channel: tl::enums::InputChannel::InputChannel(tl::types::InputChannel {
                        channel_id: c.channel_id,
                        access_hash: c.access_hash,
                    }),
                    photo,
                };
                self.rpc_write(&req).await
            }
            tl::enums::InputPeer::Chat(c) => {
                let req = tl::functions::messages::EditChatPhoto {
                    chat_id: c.chat_id,
                    photo,
                };
                self.rpc_write(&req).await
            }
            _ => Err(InvocationError::Deserialize(
                "edit_chat_photo: peer must be a chat or channel".into(),
            )),
        }
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

    pub async fn upload_profile_photo(
        &self,
        file: crate::media::UploadedFile,
    ) -> Result<tl::enums::Photo, InvocationError> {
        let req = tl::functions::photos::UploadProfilePhoto {
            fallback: false,
            bot: None,
            file: Some(file.inner),
            video: None,
            video_start_ts: None,
            video_emoji_markup: None,
        };
        let body: Vec<u8> = self.rpc_call_raw(&req).await?;
        let mut cur = Cursor::from_slice(&body);
        let tl::enums::photos::Photo::Photo(result) =
            tl::enums::photos::Photo::deserialize(&mut cur)?;
        Ok(result.photo)
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
                    .copied()
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

    pub async fn set_profile_photo(
        &self,
        file: crate::media::UploadedFile,
    ) -> Result<tl::enums::Photo, InvocationError> {
        let req = tl::functions::photos::UploadProfilePhoto {
            fallback: false,
            bot: None,
            file: Some(file.inner),
            video: None,
            video_start_ts: None,
            video_emoji_markup: None,
        };
        let body = self.rpc_call_raw(&req).await?;
        let mut cur = Cursor::from_slice(&body);
        let tl::enums::photos::Photo::Photo(result) =
            tl::enums::photos::Photo::deserialize(&mut cur)?;
        Ok(result.photo)
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

    pub async fn set_profile(
        &self,
        first_name: Option<String>,
        last_name: Option<String>,
        about: Option<String>,
    ) -> Result<tl::enums::User, InvocationError> {
        let req = tl::functions::account::UpdateProfile {
            first_name,
            last_name,
            about,
        };
        let body = self.rpc_call_raw(&req).await?;
        let mut cur = Cursor::from_slice(&body);
        Ok(tl::enums::User::deserialize(&mut cur)?)
    }

    pub async fn set_username(
        &self,
        username: impl Into<String>,
    ) -> Result<tl::enums::User, InvocationError> {
        let req = tl::functions::account::UpdateUsername {
            username: username.into(),
        };
        let body = self.rpc_call_raw(&req).await?;
        let mut cur = Cursor::from_slice(&body);
        Ok(tl::enums::User::deserialize(&mut cur)?)
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

    pub async fn set_emoji_status(
        &self,
        document_id: Option<i64>,
        until: Option<i32>,
    ) -> Result<(), InvocationError> {
        use ferogram_tl_types as tl;
        let emoji_status = match document_id {
            None => tl::enums::EmojiStatus::Empty,
            Some(id) => tl::enums::EmojiStatus::EmojiStatus(tl::types::EmojiStatus {
                document_id: id,
                until,
            }),
        };
        let req = tl::functions::account::UpdateEmojiStatus { emoji_status };
        self.rpc_write(&req).await
    }

    pub async fn get_broadcast_stats(
        &self,
        peer: impl Into<PeerRef>,
    ) -> Result<tl::enums::stats::BroadcastStats, InvocationError> {
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
                    "get_broadcast_stats: peer must be a channel".into(),
                ));
            }
        };
        let req = tl::functions::stats::GetBroadcastStats {
            dark: false,
            channel,
        };
        let body = self.rpc_call_raw(&req).await?;
        let mut cur = Cursor::from_slice(&body);
        Ok(tl::enums::stats::BroadcastStats::deserialize(&mut cur)?)
    }

    pub async fn get_megagroup_stats(
        &self,
        peer: impl Into<PeerRef>,
    ) -> Result<tl::enums::stats::MegagroupStats, InvocationError> {
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
                    "get_megagroup_stats: peer must be a supergroup".into(),
                ));
            }
        };
        let req = tl::functions::stats::GetMegagroupStats {
            dark: false,
            channel,
        };
        let body = self.rpc_call_raw(&req).await?;
        let mut cur = Cursor::from_slice(&body);
        Ok(tl::enums::stats::MegagroupStats::deserialize(&mut cur)?)
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

    pub async fn get_poll_results(
        &self,
        peer: impl Into<PeerRef>,
        msg_id: i32,
        poll_hash: i64,
    ) -> Result<(), InvocationError> {
        let peer = peer.into().resolve(self).await?;
        let input_peer = self.inner.peer_cache.read().await.peer_to_input(&peer)?;
        let req = tl::functions::messages::GetPollResults {
            peer: input_peer,
            msg_id,
            poll_hash,
        };
        self.rpc_write(&req).await
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

    #[allow(clippy::too_many_arguments)]
    pub async fn install_sticker_set(
        &self,
        stickerset: tl::enums::InputStickerSet,
        archived: bool,
    ) -> Result<tl::enums::messages::StickerSetInstallResult, InvocationError> {
        let req = tl::functions::messages::InstallStickerSet {
            stickerset,
            archived,
        };
        let body = self.rpc_call_raw(&req).await?;
        let mut cur = Cursor::from_slice(&body);
        Ok(tl::enums::messages::StickerSetInstallResult::deserialize(
            &mut cur,
        )?)
    }

    pub async fn uninstall_sticker_set(
        &self,
        stickerset: tl::enums::InputStickerSet,
    ) -> Result<(), InvocationError> {
        let req = tl::functions::messages::UninstallStickerSet { stickerset };
        self.rpc_write(&req).await
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

pub(crate) fn is_bool_true(body: &[u8]) -> bool {
    body.len() == 4 && u32::from_le_bytes(body[0..4].try_into().unwrap_or([0u8; 4])) == 0x997275b5
}

// Wire-layer types and helpers moved to ferogram-connect.
pub(crate) use ferogram_connect::connection::FrameKind;
pub(crate) use ferogram_connect::envelope::{chat_to_peer, updates_entities};
pub(crate) use ferogram_connect::frame::send_frame_write;
pub(crate) use ferogram_connect::util::{
    build_container_body, build_msgs_ack_body, jitter_delay, maybe_gz_pack,
};
pub(crate) use ferogram_connect::{
    Connection, ConnectionWriter, EnvelopeResult, FrameOutcome, FutureSalt, gz_inflate, random_i64,
    recv_frame_with_keepalive, tl_read_bytes, unwrap_envelope,
};

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
