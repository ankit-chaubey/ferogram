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
    Client, Config, ExperimentalFeatures, InvocationError, ShutdownToken, TransportKind,
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
}

impl Default for ClientBuilder {
    fn default() -> Self {
        Self {
            api_id: 0,
            api_hash: String::new(),
            dc_addr: None,
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
    /// [`in_memory`](Self::in_memory): last call wins.
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
    /// [`in_memory`](Self::in_memory): last call wins.
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
    /// Useful for [`LibSqlBackend`] (bundled SQLite, no system dep) or any
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

    /// Route all connections through a SOCKS5 proxy.
    pub fn socks5(mut self, addr: impl Into<String>) -> Self {
        self.socks5 = Some(crate::socks5::Socks5Config::new(addr));
        self
    }

    /// Route all connections through an MTProxy.
    ///
    /// The proxy `transport` is set automatically from the secret prefix;
    /// you do not need to also call `.transport()`.
    /// Build the [`MtProxyConfig`] with [`crate::parse_proxy_link`].
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
        self.mtproxy = Some(cfg);
        self
    }

    /// Allow IPv6 DC addresses (default: `false`).
    pub fn allow_ipv6(mut self, allow: bool) -> Self {
        self.allow_ipv6 = allow;
        self
    }

    /// Choose the MTProto transport framing (default: [`TransportKind::Full`]).
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
        Ok(Config {
            api_id: self.api_id,
            api_hash: self.api_hash,
            dc_addr: self.dc_addr,
            retry_policy: self.retry_policy,
            restart_policy: self.restart_policy,
            socks5: self.socks5,
            mtproxy: self.mtproxy,
            allow_ipv6: self.allow_ipv6,
            transport: self.transport,
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
        })
    }

    /// Build and connect in one step.
    ///
    /// Returns `Err(BuilderError::MissingApiId)` / `Err(BuilderError::MissingApiHash)`
    /// before attempting any network I/O if the required fields are absent.
    pub async fn connect(self) -> Result<(Client, ShutdownToken), BuilderError> {
        let cfg = self.build()?;
        Client::connect(cfg).await.map_err(BuilderError::Connect)
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
