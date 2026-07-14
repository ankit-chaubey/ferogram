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

use std::sync::Arc;

use crate::{
    Client, Config, ExperimentalFeatures, InvocationError, ShutdownToken, TransferLimits,
    TransportKind,
    restart::{ConnectionRestartPolicy, NeverRestart},
    retry::{AutoSleep, RetryPolicy},
    session_backend::{BinaryFileBackend, InMemoryBackend, SessionBackend, StringSessionBackend},
    socks5::Socks5Config,
};

/// Fluent builder for [`Config`] + [`Client::connect`].
///
/// Obtain one via [`Client::builder()`].
pub struct ClientBuilder {
    api_id: i32,
    api_hash: String,
    dc_addr: Option<String>,
    dc_id_override: Option<i32>,
    retry_policy: Arc<dyn RetryPolicy>,
    restart_policy: Arc<dyn ConnectionRestartPolicy>,
    socks5: Option<Socks5Config>,
    mtproxy: Option<crate::proxy::MtProxyConfig>,
    allow_ipv6: bool,
    transport: TransportKind,
    session_backend: Arc<dyn SessionBackend>,
    catch_up: bool,
    device_model: String,
    system_version: String,
    app_version: String,
    system_lang_code: String,
    lang_pack: String,
    lang_code: String,
    probe_transport: bool,
    resilient_connect: bool,
    experimental_features: ExperimentalFeatures,
    use_pfs: bool,
    update_config: crate::update_config::UpdateConfig,
    future_auth_token: Option<Vec<u8>>,
    transfer_limits: TransferLimits,
    transfer_safety: crate::transfer_safety::TransferSafety,
}

impl Default for ClientBuilder {
    fn default() -> Self {
        Self {
            api_id: 0,
            api_hash: String::new(),
            dc_addr: None,
            dc_id_override: None,
            retry_policy: Arc::new(AutoSleep::default()),
            restart_policy: Arc::new(NeverRestart),
            socks5: None,
            mtproxy: None,
            allow_ipv6: false,
            transport: TransportKind::Full,
            session_backend: Arc::new(BinaryFileBackend::new("ferogram.session")),
            catch_up: false,
            device_model: "Linux".to_string(),
            system_version: "1.0".to_string(),
            app_version: env!("CARGO_PKG_VERSION").to_string(),
            system_lang_code: "en".to_string(),
            lang_pack: String::new(),
            lang_code: "en".to_string(),
            probe_transport: false,
            resilient_connect: false,
            experimental_features: ExperimentalFeatures::default(),
            use_pfs: false,
            update_config: crate::update_config::UpdateConfig::default(),
            future_auth_token: None,
            transfer_limits: TransferLimits::default(),
            transfer_safety: crate::transfer_safety::TransferSafety::default(),
        }
    }
}

impl ClientBuilder {
    // Credentials

    /// Set the Telegram API ID (from <https://my.telegram.org>).
    pub fn api_id(mut self, id: i32) -> Self {
        self.api_id = id;
        self
    }

    /// Set the Telegram API hash (from <https://my.telegram.org>).
    pub fn api_hash(mut self, hash: impl Into<String>) -> Self {
        self.api_hash = hash.into();
        self
    }

    // Session

    /// Use a binary file session at `path`.
    ///
    /// Mutually exclusive with [`session_string`](Self::session_string) and
    /// `session_string("")`: last call wins.
    pub fn session(mut self, path: impl AsRef<std::path::Path>) -> Self {
        self.session_backend = Arc::new(BinaryFileBackend::new(path.as_ref()));
        self
    }

    /// Use a string session.
    ///
    /// Accepts two formats:
    ///
    /// - **Compact V1/V2**: exported by [`Client::export_session_string`].
    ///   Encodes dc_id, ip, port, auth_key, user_id.
    /// - **Native**: exported by [`Client::export_native_session_string`].
    ///   Full state: DC table, update counters, peer cache.
    ///
    /// Pass `""` to start a fresh in-memory session.
    ///
    /// Mutually exclusive with [`session`](Self::session) and
    /// `session_string("")`: last call wins.
    pub fn session_string(mut self, s: impl Into<String>) -> Self {
        let s: String = s.into();

        if let Some(persisted) = crate::builder_util::detect_compact_session(&s) {
            let backend = InMemoryBackend::new();
            let _ = crate::session_backend::SessionBackend::save(&backend, &persisted);
            self.session_backend = Arc::new(backend);
            return self;
        }

        self.session_backend = Arc::new(StringSessionBackend::new(s));
        self
    }

    /// Inject a fully custom [`SessionBackend`] implementation.
    ///
    /// Useful for `LibSqlBackend` (bundled SQLite, no system dep) or any
    /// custom persistence layer:
    /// ```rust,no_run
    /// # use ferogram::{Client};
    /// # #[cfg(feature = "libsql-session")] {
    /// # use ferogram::LibSqlBackend;
    /// use std::sync::Arc;
    /// let (client, _) = Client::builder()
    /// .api_id(12345).api_hash("abc")
    /// .session_backend(Arc::new(LibSqlBackend::new("my.db")))
    /// .connect().await?;
    /// # }
    /// ```
    pub fn session_backend(mut self, backend: Arc<dyn SessionBackend>) -> Self {
        self.session_backend = backend;
        self
    }

    /// Seed a `future_auth_token` for fast re-login, bypassing code entry on
    /// the next `request_login_code` call if Telegram still recognizes it.
    ///
    /// Normally captured automatically by `sign_out()` and persisted in the
    /// session file, only set this directly for stateless setups (e.g. a
    /// server storing the token itself instead of a session file), or to
    /// import a token obtained elsewhere. Overrides any token already in the
    /// loaded session.
    pub fn future_auth_token(mut self, token: Vec<u8>) -> Self {
        self.future_auth_token = Some(token);
        self
    }

    // Update catch-up

    /// When `true`, replay missed updates via `updates.getDifference` on connect.
    ///
    /// Default: `false`.
    pub fn catch_up(mut self, enabled: bool) -> Self {
        self.catch_up = enabled;
        self
    }

    /// Enable Perfect Forward Secrecy via `auth.bindTempAuthKey`.
    ///
    /// Adds one extra DH round-trip per connection. Off by default.
    /// Enable only if your threat model requires it.
    ///
    /// Default: `false`.
    pub fn pfs(mut self, enabled: bool) -> Self {
        self.use_pfs = enabled;
        self
    }

    // Network

    /// Override the first DC address (e.g. `"149.154.167.51:443"`).
    pub fn dc_addr(mut self, addr: impl Into<String>) -> Self {
        self.dc_addr = Some(addr.into());
        self
    }

    /// Override which `dc_id` the fresh-connect address is registered under.
    ///
    /// By default, a fresh connection (no saved session yet) always dials
    /// DC2 and labels the resulting auth key as belonging to DC2. Combined
    /// with [`dc_addr`](Self::dc_addr), this lets you dial any address and
    /// have it tracked as a specific `dc_id` instead, e.g. for pointing at a
    /// test DC or a non-default DC entirely:
    ///
    /// ```rust,no_run
    /// # use ferogram::Client;
    /// # const ID: i32 = 0;
    /// # const HASH: &str = "";
    /// # #[tokio::main] async fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// let (client, _) = Client::builder()
    ///     .api_id(ID).api_hash(HASH)
    ///     .dc_addr("149.154.175.53:443")
    ///     .dc_id_override(1)
    ///     .connect().await?;
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// Has no effect once a saved session already exists: the session's own
    /// `home_dc_id` takes over from that point on.
    pub fn dc_id_override(mut self, dc_id: i32) -> Self {
        self.dc_id_override = Some(dc_id);
        self
    }

    /// Route all connections through a SOCKS5 proxy (no authentication).
    ///
    /// The default [`TransportKind::Full`] transport is used unless you also
    /// call `.transport()`.
    ///
    /// # Example
    /// ```rust,no_run
    /// # use ferogram::Client;
    /// # const ID: i32 = 0;
    /// # const HASH: &str = "";
    /// # #[tokio::main] async fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// let (client, _) = Client::builder()
    ///     .session("my.session").api_id(ID).api_hash(HASH)
    ///     .socks5("127.0.0.1:1080")
    ///     .connect().await?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn socks5(mut self, addr: impl Into<String>) -> Self {
        self.socks5 = Some(crate::socks5::Socks5Config::new(addr));
        self
    }

    /// Route all connections through an authenticated SOCKS5 proxy.
    ///
    /// # Example
    /// ```rust,no_run
    /// # use ferogram::Client;
    /// # const ID: i32 = 0;
    /// # const HASH: &str = "";
    /// # #[tokio::main] async fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// let (client, _) = Client::builder()
    ///     .session("my.session").api_id(ID).api_hash(HASH)
    ///     .socks5_auth("proxy.example.com:1080", "user", "pass")
    ///     .connect().await?;
    /// # Ok(())
    /// # }
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

    /// Route all connections through an MTProxy using raw fields.
    ///
    /// `secret` is a hex or base64 string. Transport is auto-selected from
    /// the secret prefix (`dd` → PaddedIntermediate, `ee` → FakeTls, plain → Obfuscated).
    /// You do not need to call `.transport()`.
    ///
    /// # Example
    /// ```rust,no_run
    /// # use ferogram::Client;
    /// # const ID: i32 = 0;
    /// # const HASH: &str = "";
    /// # #[tokio::main] async fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// let (client, _) = Client::builder()
    ///     .session("my.session").api_id(ID).api_hash(HASH)
    ///     .proxy("proxy.example.com", 443, "dd1234abcdef...")
    ///     .connect().await?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn proxy(self, host: impl Into<String>, port: u16, secret: &str) -> Self {
        let host = host.into();
        let url = format!("https://t.me/proxy?server={host}&port={port}&secret={secret}");
        self.proxy_link(&url)
    }

    /// Route all connections through an MTProxy from a pre-built [`crate::MtProxyConfig`].
    ///
    /// The proxy `transport` is set automatically from the secret prefix;
    /// you do not need to also call `.transport()`.
    /// Build the [`crate::MtProxyConfig`] with [`crate::parse_proxy_link`].
    pub fn mtproxy(mut self, proxy: crate::proxy::MtProxyConfig) -> Self {
        // Override transport to match what the proxy requires.
        self.transport = proxy.transport.clone();
        self.mtproxy = Some(proxy);
        self
    }

    /// Set an MTProxy from a `https://t.me/proxy?...` or `tg://proxy?...` link.
    ///
    /// Empty string is a no-op; proxy stays unset.
    /// Transport is selected from the secret prefix automatically.
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

    /// Allow IPv6 DC addresses (default: `false`).
    pub fn allow_ipv6(mut self, allow: bool) -> Self {
        self.allow_ipv6 = allow;
        self
    }

    /// Choose the MTProto transport framing.
    ///
    /// **Default: [`TransportKind::Full`]** - recommended for most users.
    /// Full transport adds seqno + CRC32 integrity on every frame and works
    /// on all standard Telegram DCs without any special handshake.
    ///
    /// Other options:
    /// - [`TransportKind::Abridged`] - lightest overhead, no integrity check
    /// - [`TransportKind::Intermediate`] - 4-byte length prefix, no CRC
    /// - [`TransportKind::Obfuscated`] - AES-256-CTR, bypasses basic DPI
    /// - [`TransportKind::PaddedIntermediate`] - required for `0xDD` MTProxy secrets
    /// - [`TransportKind::FakeTls`] - most DPI-resistant, required for `0xEE` MTProxy secrets
    ///
    /// **Note:** if you also call `.mtproxy()`, `.proxy()`, or `.proxy_link()`,
    /// the transport is forced to whatever the proxy secret requires - this call
    /// is ignored for MTProxy connections.
    pub fn transport(mut self, kind: TransportKind) -> Self {
        self.transport = kind;
        self
    }

    // Retry

    /// Override the flood-wait / retry policy.
    pub fn retry_policy(mut self, policy: Arc<dyn RetryPolicy>) -> Self {
        self.retry_policy = policy;
        self
    }

    pub fn restart_policy(mut self, policy: Arc<dyn ConnectionRestartPolicy>) -> Self {
        self.restart_policy = policy;
        self
    }

    // InitConnection identity

    /// Set the device model string sent in `InitConnection` (default: `"Linux"`).
    ///
    /// This shows up in Telegram's active sessions list as the device name.
    pub fn device_model(mut self, model: impl Into<String>) -> Self {
        self.device_model = model.into();
        self
    }

    /// Set the system/OS version string sent in `InitConnection` (default: `"1.0"`).
    pub fn system_version(mut self, version: impl Into<String>) -> Self {
        self.system_version = version.into();
        self
    }

    /// Set the app version string sent in `InitConnection` (default: crate version from `CARGO_PKG_VERSION`).
    pub fn app_version(mut self, version: impl Into<String>) -> Self {
        self.app_version = version.into();
        self
    }

    /// Set the system language code sent in `InitConnection` (default: `"en"`).
    pub fn system_lang_code(mut self, code: impl Into<String>) -> Self {
        self.system_lang_code = code.into();
        self
    }

    /// Set the language pack name sent in `InitConnection` (default: `""`).
    pub fn lang_pack(mut self, pack: impl Into<String>) -> Self {
        self.lang_pack = pack.into();
        self
    }

    /// Set the language code sent in `InitConnection` (default: `"en"`).
    pub fn lang_code(mut self, code: impl Into<String>) -> Self {
        self.lang_code = code.into();
        self
    }

    /// Race Obfuscated / Abridged / HTTP transports in parallel and pick the
    /// fastest.  Ideal when you don't know which transport your network allows.
    /// Incompatible with MTProxy (proxy enforces a specific transport).
    /// Default: `false`.
    pub fn probe_transport(mut self, enabled: bool) -> Self {
        self.probe_transport = enabled;
        self
    }

    /// If direct TCP fails, retry via DNS-over-HTTPS (Mozilla + Google DoH),
    /// then fall back to Firebase / Google special-config.
    /// Default: `false`.
    pub fn resilient_connect(mut self, enabled: bool) -> Self {
        self.resilient_connect = enabled;
        self
    }

    /// Opt in to experimental behaviours.
    ///
    /// All flags default to `false`.  Read [`ExperimentalFeatures`] docs before
    /// enabling anything here.
    ///
    /// # Example
    /// ```rust,no_run
    /// use ferogram::{Client, ExperimentalFeatures};
    ///
    /// # #[tokio::main] async fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// let (client, _sd) = Client::builder()
    ///     .api_id(12345)
    ///     .api_hash("abc")
    ///     .experimental_features(ExperimentalFeatures {
    ///         allow_zero_hash: true,  // bots only
    ///         ..Default::default()
    ///     })
    ///     .connect().await?;
    /// # Ok(()) }
    /// ```
    pub fn experimental_features(mut self, features: ExperimentalFeatures) -> Self {
        self.experimental_features = features;
        self
    }

    // Transfer concurrency

    /// Set all transfer concurrency ceilings at once.
    ///
    /// See [`TransferLimits`] for the highway/trucks model this controls -
    /// in short, [`TransferLimits::download_tcp_connections`] /
    /// [`upload_tcp_connections`](TransferLimits::upload_tcp_connections)
    /// are how many parallel connections one download or upload may open
    /// for itself, and [`TransferLimits::max_tcp_connections`] is how many
    /// the whole client may have open across every transfer at once.
    ///
    /// Defaults match Ferogram's built-in tuning, so most users never need
    /// this. Prefer the [`download_tcp_connections`](Self::download_tcp_connections) /
    /// [`upload_tcp_connections`](Self::upload_tcp_connections) /
    /// [`max_tcp_connections`](Self::max_tcp_connections)
    /// shorthands if you only want to change one field.
    ///
    /// # Example
    /// ```rust,no_run
    /// use ferogram::{Client, TransferLimits};
    /// # const ID: i32 = 0;
    /// # const HASH: &str = "";
    /// # #[tokio::main] async fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// let (client, _) = Client::builder()
    ///     .api_id(ID).api_hash(HASH)
    ///     .transfer_limits(TransferLimits {
    ///         download_tcp_connections: 2,
    ///         upload_tcp_connections: 4,
    ///         max_tcp_connections: 6,
    ///         download_pipeline_depth: 2,
    ///         upload_pipeline_depth: 2,
    ///         bypass_tcp_allotments: false,
    ///     })
    ///     .connect().await?;
    /// # Ok(()) }
    /// ```
    pub fn transfer_limits(mut self, limits: TransferLimits) -> Self {
        self.transfer_limits = limits;
        self
    }

    /// Hard safety ceilings for file transfers - a weighted in-flight-bytes
    /// cap and a requests/sec limiter - enforced independently of whatever
    /// [`transfer_limits`](Self::transfer_limits) requests. See
    /// [`TransferSafety`](crate::TransferSafety) for the full explanation
    /// of why this is a separate mechanism from `transfer_limits`.
    ///
    /// Not applied to `upload_exp`/`download_exp` (`experimental` feature)
    /// - those stay fully unprotected, as documented.
    ///
    /// # Example
    /// ```rust,no_run
    /// use ferogram::{Client, TransferSafety};
    /// # const ID: i32 = 0;
    /// # const HASH: &str = "";
    /// # #[tokio::main] async fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// let (client, _) = Client::builder()
    ///     .api_id(ID).api_hash(HASH)
    ///     .transfer_safety(TransferSafety {
    ///         allow_pipelining: true,
    ///         allow_multiple_connections: true,
    ///         max_connections: Some(4),
    ///         max_in_flight_bytes: 4 * 1024 * 1024,
    ///         max_requests_per_sec: Some(20),
    ///     })
    ///     .connect().await?;
    /// # Ok(()) }
    /// ```
    pub fn transfer_safety(mut self, safety: crate::transfer_safety::TransferSafety) -> Self {
        self.transfer_safety = safety;
        self
    }

    /// Y: max parallel connections a single download may open for itself.
    /// Small files always use 1 regardless of this value. Tuned separately
    /// from uploads since a link's download and upload bandwidth often
    /// differ - see [`upload_tcp_connections`](Self::upload_tcp_connections).
    ///
    /// Clamped to [`media::MAX_WORKERS_PER_FILE`](crate::media::MAX_WORKERS_PER_FILE)
    /// on connect - exceeding it causes Telegram to shed connections with
    /// early-EOF, so raising this past the ceiling has no effect rather than
    /// making transfers faster.
    ///
    /// Raising this above the default is your call to make, and Telegram's
    /// `FLOOD_WAIT` is how it tells you when you've pushed too far for the
    /// current account or DC. If that starts happening, lower this (and
    /// the pipeline depth) before anything else - see the
    /// [`transfer_limits` module docs](crate::transfer_limits) for the full
    /// explanation.
    ///
    /// Default: 4.
    pub fn download_tcp_connections(mut self, n: usize) -> Self {
        self.transfer_limits.download_tcp_connections = n;
        self
    }

    /// Y for uploads. See [`download_tcp_connections`](Self::download_tcp_connections).
    ///
    /// Default: 4.
    pub fn upload_tcp_connections(mut self, n: usize) -> Self {
        self.transfer_limits.upload_tcp_connections = n;
        self
    }

    /// Total transfer connections ("highways") available to the whole
    /// client at once, shared across every concurrent upload and download.
    ///
    /// Lower this on memory- or socket-constrained devices. Never clamped
    /// below 1.
    ///
    /// Default: 12.
    pub fn max_tcp_connections(mut self, n: usize) -> Self {
        self.transfer_limits.max_tcp_connections = n;
        self
    }

    /// X: how many `GetFile` requests a single download connection keeps in
    /// flight at once ("trucks on the highway"), instead of waiting for
    /// each response before sending the next.
    ///
    /// Clamped to [`media::MAX_PIPELINE_DEPTH`](crate::media::MAX_PIPELINE_DEPTH)
    /// on connect - each in-flight request holds a full chunk buffer in
    /// memory, so this bounds memory rather than protecting the server.
    ///
    /// Default: 4.
    pub fn download_pipeline_depth(mut self, n: usize) -> Self {
        self.transfer_limits.download_pipeline_depth = n;
        self
    }

    /// X for uploads. See [`download_pipeline_depth`](Self::download_pipeline_depth).
    ///
    /// Default: 4.
    pub fn upload_pipeline_depth(mut self, n: usize) -> Self {
        self.transfer_limits.upload_pipeline_depth = n;
        self
    }

    /// Skip the size-based Y lookup tables entirely and always use
    /// [`download_tcp_connections`](Self::download_tcp_connections) /
    /// [`upload_tcp_connections`](Self::upload_tcp_connections) directly,
    /// regardless of file size.
    ///
    /// Off by default: Y is normally chosen from a fixed size-tiered table,
    /// clamped to your configured ceiling. Turning this on removes the
    /// table and always opens exactly your configured ceiling worth of
    /// connections, even for a file that would otherwise only need one.
    /// Does not change chunk size or X (pipeline depth).
    ///
    /// This is an override, not just a tuning knob - it means small files
    /// now push as much concurrency as large ones. If you start seeing
    /// `FLOOD_WAIT` after turning this on, turn it back off (or lower your
    /// connection ceilings) before anything else. See the
    /// [`transfer_limits` module docs](crate::transfer_limits) for the full
    /// responsibility note.
    ///
    /// Default: `false`.
    pub fn bypass_tcp_allotments(mut self, enabled: bool) -> Self {
        self.transfer_limits.bypass_tcp_allotments = enabled;
        self
    }

    // Update dispatch configuration

    /// Set the maximum number of updates held in the user-facing dispatch
    /// buffer.
    ///
    /// A smaller value uses less RAM at the cost of more frequent evictions
    /// under burst load. A larger value absorbs longer bursts before any
    /// update is dropped.
    ///
    /// Internal MTProto state (pts, qts, getDifference) is unaffected: it
    /// always runs in the reader task regardless of this value.
    ///
    /// Default: `2048`.
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// # use ferogram::Client;
    /// # #[tokio::main] async fn main() -> anyhow::Result<()> {
    /// let (client, _) = Client::builder()
    ///     .api_id(12345)
    ///     .api_hash("abc")
    ///     .session("bot.session")
    ///     .update_queue_capacity(512)
    ///     .connect().await?;
    /// # Ok(()) }
    /// ```
    pub fn update_queue_capacity(mut self, capacity: usize) -> Self {
        self.update_config.queue_capacity = capacity.max(1);
        self
    }

    /// Set what happens when the update buffer is full and a new update arrives.
    ///
    /// * [`OverflowStrategy::DropOldest`] (default) - evicts the stalest
    ///   ephemeral update (typing, online status) first, then the oldest
    ///   normal update. The incoming update is always buffered.
    /// * [`OverflowStrategy::DropNewest`] - the incoming update is discarded
    ///   and the existing queue is untouched.
    ///
    /// [`OverflowStrategy::DropOldest`]: crate::update_config::OverflowStrategy::DropOldest
    /// [`OverflowStrategy::DropNewest`]: crate::update_config::OverflowStrategy::DropNewest
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// # use ferogram::{Client, OverflowStrategy};
    /// # #[tokio::main] async fn main() -> anyhow::Result<()> {
    /// let (client, _) = Client::builder()
    ///     .api_id(12345)
    ///     .api_hash("abc")
    ///     .session("bot.session")
    ///     .update_overflow_strategy(OverflowStrategy::DropOldest)
    ///     .connect().await?;
    /// # Ok(()) }
    /// ```
    pub fn update_overflow_strategy(
        mut self,
        strategy: crate::update_config::OverflowStrategy,
    ) -> Self {
        self.update_config.overflow_strategy = strategy;
        self
    }

    /// Drops the queue to 256 slots and sets `DropOldest` eviction.
    ///
    /// Shorthand for
    /// `.update_queue_capacity(256).update_overflow_strategy(OverflowStrategy::DropOldest)`.
    ///
    /// Useful on Termux, small VPS, or any host where RAM is tight.
    /// When `false` (the default) this is a no-op.
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// # use ferogram::Client;
    /// # #[tokio::main] async fn main() -> anyhow::Result<()> {
    /// let (client, _) = Client::builder()
    ///     .api_id(12345)
    ///     .api_hash("abc")
    ///     .session("bot.session")
    ///     .low_memory_mode(true)
    ///     .connect().await?;
    /// # Ok(()) }
    /// ```
    pub fn low_memory_mode(mut self, enable: bool) -> Self {
        if enable {
            self.update_config = crate::update_config::UpdateConfig::low_memory();
        }
        self
    }

    // Terminal

    /// Build the [`Config`] without connecting.
    pub fn build(self) -> Result<Config, BuilderError> {
        if self.api_id == 0 {
            return Err(BuilderError::MissingApiId);
        }
        if self.api_hash.is_empty() {
            return Err(BuilderError::MissingApiHash);
        }
        // Enforce transport consistency: mtproxy always dictates its own transport
        // regardless of what the user may have called `.transport()` with.
        // For socks5-only, Full is the correct default (no obfuscation layer needed).
        let transport = if let Some(ref proxy) = self.mtproxy {
            proxy.transport.clone()
        } else {
            self.transport
        };
        Ok(Config {
            api_id: self.api_id,
            api_hash: self.api_hash,
            dc_addr: self.dc_addr,
            dc_id_override: self.dc_id_override,
            retry_policy: self.retry_policy,
            restart_policy: self.restart_policy,
            socks5: self.socks5,
            mtproxy: self.mtproxy,
            allow_ipv6: self.allow_ipv6,
            transport,
            session_backend: self.session_backend,
            catch_up: self.catch_up,
            device_model: self.device_model,
            system_version: self.system_version,
            app_version: self.app_version,
            system_lang_code: self.system_lang_code,
            lang_pack: self.lang_pack,
            lang_code: self.lang_code,
            probe_transport: self.probe_transport,
            resilient_connect: self.resilient_connect,
            experimental_features: self.experimental_features,
            use_pfs: self.use_pfs,
            update_config: self.update_config,
            future_auth_token: self.future_auth_token,
            transfer_limits: self.transfer_limits.normalized(),
            transfer_safety: self.transfer_safety,
        })
    }

    /// Build and connect in one step.
    ///
    /// Returns `Err(BuilderError::MissingApiId)` / `Err(BuilderError::MissingApiHash)`
    /// before attempting any network I/O if the required fields are absent.
    ///
    /// This method only establishes the connection. It never prompts for
    /// credentials. Auth is the caller's responsibility:
    ///
    /// ```rust,no_run
    /// # use ferogram::Client;
    /// # #[tokio::main] async fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// let (client, _) = Client::builder()
    ///     .api_id(12345)
    ///     .api_hash("abc")
    ///     .session("bot.session")
    ///     .connect().await?;
    ///
    /// if !client.is_authorized().await? {
    ///     client.bot_sign_in("123456:TOKEN").await?;
    ///     client.save_session().await?;
    /// }
    /// # Ok(()) }
    /// ```
    ///
    /// If you want an interactive stdin prompt for first-time auth use
    /// [`connect_and_login`](Self::connect_and_login) instead.
    pub async fn connect(self) -> Result<(Client, ShutdownToken), crate::QuickConnectError> {
        let cfg = self.build()?;
        let (client, shutdown) = Client::connect(cfg).await.map_err(BuilderError::Connect)?;
        Ok((client, shutdown))
    }

    /// Build, connect, and interactively authenticate if not already signed in.
    ///
    /// Prompts via stdin for a phone number or bot token, drives the full
    /// auth flow (login code, 2FA password if required), and saves the
    /// session. If the session is already authorized the prompt is skipped.
    ///
    /// Use this only for interactive tools or first-time setup scripts.
    /// For bots and production code use [`connect`](Self::connect) and
    /// call `bot_sign_in` / `sign_in` yourself.
    pub async fn connect_and_login(
        self,
    ) -> Result<(Client, ShutdownToken), crate::QuickConnectError> {
        let cfg = self.build()?;
        let (client, shutdown) = Client::connect(cfg).await.map_err(BuilderError::Connect)?;
        crate::quick_connect::login_interactive(&client).await?;
        Ok((client, shutdown))
    }
}

// BuilderError

/// Errors that can be returned by [`ClientBuilder::build`] or
/// [`ClientBuilder::connect`].
#[derive(Debug)]
pub enum BuilderError {
    /// `api_id` was not set (or left at 0).
    MissingApiId,
    /// `api_hash` was not set (or left empty).
    MissingApiHash,
    /// The underlying [`Client::connect`] call failed.
    Connect(InvocationError),
}

impl std::fmt::Display for BuilderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingApiId => f.write_str("ClientBuilder: api_id not set"),
            Self::MissingApiHash => f.write_str("ClientBuilder: api_hash not set"),
            Self::Connect(e) => write!(f, "ClientBuilder: connect failed: {e}"),
        }
    }
}

impl std::error::Error for BuilderError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Connect(e) => Some(e),
            _ => None,
        }
    }
}
