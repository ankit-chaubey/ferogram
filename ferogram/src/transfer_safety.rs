//! Hard ceilings for transfer concurrency, enforced independently of
//! whatever [`TransferLimits`](crate::TransferLimits) or the experimental
//! [`TransferConfig`](crate::client::files::TransferConfig) asks for.
//!
//! Where [`TransferLimits`] expresses *desired* tuning (how hard a transfer
//! should try to go), [`TransferSafety`] expresses what it's *allowed* to
//! do, no matter what any config requests. The effective ceiling at any
//! call site is always `min(requested, safety)`.
//!
//! Two mechanisms, both new relative to what ferogram enforced before:
//!
//! - **Weighted in-flight bytes**: unlike the existing worker/connection
//!   count ceiling (which treats a 512 KB chunk and a 128 KB chunk as
//!   equally "expensive"), this caps total *unacknowledged chunk data*,
//!   in bytes, across every concurrent transfer.
//! - **Requests/sec**: a token-bucket ceiling on how many chunk RPCs may
//!   fire per second, client-wide. This is the actual `FLOOD_WAIT` defense
//!   - nothing in ferogram rate-limited chunk requests before this.
//!
//! **Not currently applied to [`Client::upload_exp`]/[`Client::download_exp`]**
//! (`experimental` feature) - those paths are documented as bypassing
//! ferogram's safety limits entirely, and stay that way. `TransferSafety`
//! only governs the normal, auto-tuned transfer paths.
//!
//! [`Client::upload_exp`]: crate::Client::upload_exp
//! [`Client::download_exp`]: crate::Client::download_exp

use governor::{Quota, RateLimiter};
use std::num::NonZeroU32;
use std::sync::Arc;
use tokio::sync::Semaphore;

/// User-tunable hard ceilings for transfer concurrency.
///
/// Set client-wide via
/// [`ClientBuilder::transfer_safety`](crate::builder::ClientBuilder::transfer_safety),
/// or pass a per-call override to the `_with_safety` variant of a transfer
/// method (e.g.
/// [`download_media_with_safety`](crate::Client::download_media_with_safety))
/// to use a tighter (or looser) policy than the client default for just
/// that call.
///
/// # Example
/// ```rust,no_run
/// use ferogram::{Client, TransferSafety};
///
/// # #[tokio::main] async fn main() -> Result<(), Box<dyn std::error::Error>> {
/// let (client, _sd) = Client::builder()
///     .api_id(12345)
///     .api_hash("abc")
///     .transfer_safety(TransferSafety {
///         max_connections: Some(4),
///         max_in_flight_bytes: 4 * 1024 * 1024, // 4 MB of chunks outstanding at once
///         max_requests_per_sec: Some(20),
///     })
///     .connect().await?;
/// # Ok(()) }
/// ```
#[derive(Clone, Copy, Debug)]
pub struct TransferSafety {
    /// Hard ceiling on concurrent connections, across every transfer
    /// running at once. `None` means this mechanism is off - the existing
    /// [`TransferLimits::max_tcp_connections`](crate::TransferLimits::max_tcp_connections)
    /// ceiling still applies on its own.
    ///
    /// If set, the *effective* connection count at any call site is
    /// `min(requested_workers, max_connections)` - this can only make the
    /// ceiling tighter than `TransferLimits`, never looser.
    ///
    /// Default: `None`.
    pub max_connections: Option<usize>,

    /// Hard ceiling on total unacknowledged chunk data, in bytes, across
    /// every concurrent transfer client-wide. A chunk RPC blocks until
    /// enough of this budget frees up (previous chunks completing) before
    /// it's allowed to fire.
    ///
    /// Unlike a connection-count limit, this is weighted by actual chunk
    /// size, so a transfer using 512 KB chunks counts for more against
    /// this ceiling than one using 128 KB chunks.
    ///
    /// Default: `16 MiB` (`16 * 1024 * 1024`).
    pub max_in_flight_bytes: usize,

    /// Hard ceiling on chunk RPCs per second, client-wide, enforced with a
    /// token bucket (via the `governor` crate). `None` disables rate
    /// limiting entirely - only the connection/byte ceilings apply.
    ///
    /// This is the actual defense against `FLOOD_WAIT`: nothing else in
    /// ferogram paces request *rate*, only concurrent request *count*.
    ///
    /// Default: `None`.
    pub max_requests_per_sec: Option<u32>,
}

impl Default for TransferSafety {
    fn default() -> Self {
        Self {
            max_connections: None,
            max_in_flight_bytes: 16 * 1024 * 1024,
            max_requests_per_sec: None,
        }
    }
}

type Limiter = RateLimiter<
    governor::state::NotKeyed,
    governor::state::InMemoryState,
    governor::clock::DefaultClock,
>;

/// Runtime governor built from a [`TransferSafety`] config. Lives on
/// `ClientInner` (client-wide default) and can also be built ad hoc for a
/// per-call override - either way, every chunk RPC on a normal transfer
/// path acquires a permit here before it's allowed to fire.
///
/// Cheap to clone: internally Arc-wrapped.
#[derive(Clone)]
pub struct TransferSafetyGovernor {
    config: TransferSafety,
    /// One permit per byte of `max_in_flight_bytes`. A chunk of size N
    /// acquires N permits before sending, releasing them when the response
    /// arrives (or the attempt is abandoned).
    in_flight_bytes: Arc<Semaphore>,
    rate_limiter: Option<Arc<Limiter>>,
}

/// Held for the duration of one in-flight chunk RPC. Releases its share of
/// `max_in_flight_bytes` back to the governor on drop, whether the RPC
/// succeeded, failed, or was retried - so a slow or erroring chunk can't
/// permanently eat into the budget.
pub struct SafetyPermit<'a> {
    _permit: tokio::sync::SemaphorePermit<'a>,
}

impl TransferSafetyGovernor {
    pub fn new(config: TransferSafety) -> Self {
        let rate_limiter = config
            .max_requests_per_sec
            .and_then(NonZeroU32::new)
            .map(|rps| Arc::new(RateLimiter::direct(Quota::per_second(rps))));
        Self {
            config,
            in_flight_bytes: Arc::new(Semaphore::new(config.max_in_flight_bytes)),
            rate_limiter,
        }
    }

    /// Cap `requested_workers` down to `max_connections`, if that safety
    /// field is set. Never raises the count - only ever tightens it.
    pub fn cap_workers(&self, requested_workers: usize) -> usize {
        match self.config.max_connections {
            Some(cap) => requested_workers.min(cap.max(1)),
            None => requested_workers,
        }
    }

    /// Block until both the requests/sec bucket and the in-flight byte
    /// budget allow one more chunk RPC of `chunk_len` bytes, then return a
    /// guard that releases the byte budget on drop.
    ///
    /// `chunk_len` is clamped to `max_in_flight_bytes` so a single chunk
    /// larger than the entire configured budget doesn't deadlock waiting
    /// for permits that can never exist - it just consumes the whole
    /// budget for its own duration instead.
    pub async fn acquire(&self, chunk_len: usize) -> SafetyPermit<'_> {
        if let Some(limiter) = &self.rate_limiter {
            limiter.until_ready().await;
        }
        let permits = chunk_len.clamp(1, self.config.max_in_flight_bytes) as u32;
        let permit = self
            .in_flight_bytes
            .acquire_many(permits)
            .await
            .expect("transfer safety semaphore unexpectedly closed");
        SafetyPermit { _permit: permit }
    }
}
