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

//! User-tunable transfer concurrency: how hard Ferogram is allowed to push
//! when uploading or downloading a file.
//!
//! Telegram's own file API docs describe two independent ways to speed up a
//! transfer: open more connections, and keep more than one request in
//! flight per connection. This module is Ferogram's implementation of both,
//! plus a global cap so a client running several transfers at once doesn't
//! open an unbounded number of sockets.
//!
//! ## The highway model
//!
//! It helps to picture a file transfer as trucks travelling on highways.
//!
//! - A **truck** is one in-flight chunk request: `upload.saveFilePart`,
//!   `upload.saveBigFilePart`, or `upload.getFile`.
//! - The **load a truck carries** is the chunk size: how many bytes one
//!   request moves. This is decided by file size alone (see the tables in
//!   [`crate::media`]), not by anything in this module - it isn't part of
//!   what you configure here, but it's worth naming since Y and X both
//!   multiply against it to get actual throughput.
//! - The **length of the highway** is the round-trip time (RTT): how long
//!   one truck takes to reach Telegram and come back with a response. This
//!   is set by the network path between you and the datacenter, not by
//!   Ferogram or by anything you configure - a highway to a nearby DC is
//!   short, a highway to one on the other side of the world is long. You
//!   cannot shorten the road. What you *can* do is put more trucks on it
//!   at once, which is exactly what X is for.
//! - A **highway** is one dedicated MTProto connection (TCP socket) opened
//!   just for this transfer. Trucks travel on a highway one after another,
//!   but a highway can hold several trucks moving at once if you let it.
//! - **Y**, i.e. [`TransferLimits::download_tcp_connections`] and
//!   [`TransferLimits::upload_tcp_connections`], is how many highways a
//!   *single* transfer is allowed to build for itself. A small file might
//!   only need one highway. A large file can split itself across several,
//!   each one carrying a different slice of the file at the same time.
//! - **X**, i.e. [`TransferLimits::download_pipeline_depth`] and
//!   [`TransferLimits::upload_pipeline_depth`], is how many trucks are
//!   allowed on one highway at once. Without this, a connection sends one
//!   truck, waits for it to travel the whole road and come back (one full
//!   RTT), then sends the next - so on a long highway (high RTT), most of
//!   the connection's time is spent watching an empty road instead of
//!   moving trucks. With X > 1, several trucks are on the road at the same
//!   time, so the road is never empty while a response is in transit.
//! - [`TransferLimits::max_tcp_connections`] is a city-wide speed limit on
//!   *total* highways: every transfer's highways, upload and download
//!   alike, draw from this one shared pool. A big download and a big
//!   upload running at the same time both count against it.
//!
//! Put together: **total trucks in flight for one transfer = Y × X**, and
//! **total bytes in flight = Y × X × chunk size**. A transfer using 3
//! highways at a pipeline depth of 4 can have up to 12 chunks moving
//! simultaneously. That is the whole tuning surface - there
//! is no third secret dial. Everything Ferogram does to make a transfer
//! faster or slower comes down to moving one of these two numbers.
//!
//! ## Why Y and X are not the same knob
//!
//! It would be simpler to have one "concurrency" number, but Y and X cost
//! different things and help with different problems, so they are kept
//! separate on purpose:
//!
//! - **Y costs the server something.** Each additional highway is a real
//!   socket and a real MTProto session Telegram has to track. Open too
//!   many for one file and Telegram starts shedding connections with an
//!   early-EOF instead of politely refusing - that is why Y has a hard
//!   ceiling ([`media::MAX_WORKERS_PER_FILE`](crate::media::MAX_WORKERS_PER_FILE))
//!   that no config value can exceed.
//! - **X costs the client something.** Every truck in flight is a chunk
//!   buffer sitting in memory until its response arrives. Raising X only
//!   costs you RAM, not server goodwill, so its ceiling
//!   ([`media::MAX_PIPELINE_DEPTH`](crate::media::MAX_PIPELINE_DEPTH)) exists
//!   to bound memory use, not to protect Telegram.
//! - **Y helps most when bandwidth is the bottleneck** - more parallel
//!   sockets means more of your link's total bandwidth gets used at once.
//! - **X helps most when round-trip time is the bottleneck** - on a
//!   high-latency link (mobile data, satellite, a far-away datacenter),
//!   most of a single connection's time is spent waiting for a reply, and
//!   pipelining is what fills that dead time with useful work.
//!
//! In practice most people never need to touch either: Ferogram already
//! picks sensible values per file size (see the size tables in
//! [`crate::media`]) and clamps everything to numbers Ankit has already
//! verified are safe. These fields exist for the minority of cases where
//! you know something about your own network or device that the size
//! heuristic can't see.
//!
//! ## Setting it up
//!
//! Most people should change nothing. If you do want to tune it, use the
//! [`ClientBuilder`](crate::builder::ClientBuilder) shorthands - each one
//! changes exactly one field and leaves the rest at their defaults:
//!
//! ```rust,no_run
//! use ferogram::Client;
//!
//! # #[tokio::main] async fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let (client, _sd) = Client::builder()
//!     .api_id(12345)
//!     .api_hash("abc")
//!     // Slow, high-latency mobile link: fewer highways, deeper pipeline.
//!     .download_tcp_connections(2)
//!     .upload_tcp_connections(2)
//!     .download_pipeline_depth(6)
//!     .upload_pipeline_depth(6)
//!     .connect().await?;
//! # Ok(()) }
//! ```
//!
//! Or set every field together with [`TransferLimits`] and
//! [`ClientBuilder::transfer_limits`](crate::builder::ClientBuilder::transfer_limits)
//! if you want to reason about the whole picture at once instead of one
//! field at a time. See the struct-level docs below for what each field
//! does and what it defaults to.
//!
//! [`TransferLimits::bypass_tcp_allotments`] is a separate, more drastic
//! switch: instead of letting file size decide Y (see the tables in
//! [`crate::media`]), it forces every transfer in that direction to use
//! your configured ceiling directly, no matter how small the file is. Read
//! its own docs and the responsibility section below carefully before
//! turning it on.
//!
//! ## Overriding defaults is your responsibility
//!
//! Ferogram's defaults were not picked at random. Ankit tuned them against
//! how Telegram's servers actually behave, not just what the docs say is
//! technically allowed. Every field in [`TransferLimits`] is still clamped
//! to a hard ceiling that Ankit controls, so you cannot accidentally
//! configure something that breaks the protocol outright. But *within*
//! those ceilings, Ferogram gives you real room to push harder than the
//! defaults, and if you do, you are the one deciding to trade safety
//! margin for speed. That trade is yours to make and yours to own -
//! Ferogram will not silently protect you from a choice you asked it to
//! make.
//!
//! The most common way this shows up is `FLOOD_WAIT`. Telegram enforces
//! per-account, per-DC rate limits that are not published anywhere and can
//! change without notice. Pushing Y and X higher, or turning on
//! [`bypass_tcp_allotments`](TransferLimits::bypass_tcp_allotments) so
//! every transfer opens its full ceiling of connections regardless of
//! size, means more simultaneous requests hitting the same account and DC.
//! Ferogram already handles an individual `FLOOD_WAIT` correctly - it backs
//! off for the time Telegram asks for and retries the affected chunk, so a
//! single transfer will not fail outright because of one flood wait. But
//! if you are seeing `FLOOD_WAIT` regularly, that is Telegram telling you,
//! directly, that the concurrency you configured is higher than it is
//! currently willing to tolerate for this account or DC.
//!
//! **If that happens, the fix is on your side, not Ferogram's: turn the
//! dial back down.** Reduce X first (pipeline depth), since it is the
//! cheaper adjustment and does not change how many sockets you hold open.
//! If flood waits continue, reduce Y (connections per file) next, and turn
//! off `bypass_tcp_allotments` if you had it on, so small files go back to
//! using only as many connections as they actually need. There is no
//! configuration that makes Telegram's rate limits go away - only one that
//! respects them. Ferogram and Ankit give you the maximum control the
//! protocol allows; using that control responsibly, and noticing when it
//! is time to back off, is on you.
//!
//! ## What's deliberately not here
//!
//! Chunk-size tiers, the big-file threshold, the free-tier part-count
//! ceiling, and the *absolute* per-file worker ceiling itself all follow
//! Telegram's wire-format rules and Ferogram's own empirically-tuned
//! defaults - see [`crate::media`]. Those stay constant. Getting them wrong
//! risks invalid requests or the server shedding connections with
//! early-EOF. [`TransferLimits`] only lets you move within the range that's
//! already known to be safe.

/// User-tunable ceilings for concurrent transfer connections.
///
/// Set via [`ClientBuilder::transfer_limits`](crate::builder::ClientBuilder::transfer_limits),
/// or the [`download_tcp_connections`](crate::builder::ClientBuilder::download_tcp_connections) /
/// [`upload_tcp_connections`](crate::builder::ClientBuilder::upload_tcp_connections) /
/// [`max_tcp_connections`](crate::builder::ClientBuilder::max_tcp_connections) /
/// [`download_pipeline_depth`](crate::builder::ClientBuilder::download_pipeline_depth) /
/// [`upload_pipeline_depth`](crate::builder::ClientBuilder::upload_pipeline_depth)
/// shorthands.
///
/// All fields are clamped on [`connect`](crate::Client::connect):
/// `download_tcp_connections` / `upload_tcp_connections`
/// never exceed [`media::MAX_WORKERS_PER_FILE`](crate::media::MAX_WORKERS_PER_FILE),
/// `download_pipeline_depth` / `upload_pipeline_depth` never exceed
/// [`media::MAX_PIPELINE_DEPTH`](crate::media::MAX_PIPELINE_DEPTH), and
/// `max_tcp_connections` is never allowed below 1. See the [module
/// docs](self) for the highway/trucks model this controls, and its
/// "Overriding defaults is your responsibility" section before pushing
/// any of these past their defaults.
///
/// # Example
/// ```rust,no_run
/// use ferogram::{Client, TransferLimits};
///
/// # #[tokio::main] async fn main() -> Result<(), Box<dyn std::error::Error>> {
/// let (client, _sd) = Client::builder()
///     .api_id(12345)
///     .api_hash("abc")
///     .transfer_limits(TransferLimits {
///         download_tcp_connections: 2,  // low-memory device: fewer highways per download
///         upload_tcp_connections: 4,    // uploads keep the default
///         max_tcp_connections: 6,                // fewer total sockets across the client
///         download_pipeline_depth: 2,            // fewer trucks in flight per highway
///         upload_pipeline_depth: 2,
///         bypass_tcp_allotments: false,
///     })
///     .connect().await?;
/// # Ok(()) }
/// ```
#[derive(Clone, Copy, Debug)]
pub struct TransferLimits {
    /// Y: how many parallel MTProto connections a single download may open
    /// for itself. Small files always use 1 regardless of this value;
    /// larger files scale up towards this ceiling.
    ///
    /// Default: [`media::MAX_WORKERS_PER_FILE`](crate::media::MAX_WORKERS_PER_FILE) (4).
    pub download_tcp_connections: usize,

    /// Y for uploads. See [`download_tcp_connections`](Self::download_tcp_connections).
    ///
    /// Default: [`media::MAX_WORKERS_PER_FILE`](crate::media::MAX_WORKERS_PER_FILE) (4).
    pub upload_tcp_connections: usize,

    /// Total highways available to the whole client at once, shared across
    /// every upload and download running concurrently - a large download
    /// and a large upload happening at the same time draw from this same
    /// pool.
    ///
    /// Lower this on memory- or socket-constrained devices (e.g. Termux on
    /// older Android hardware). Raise it if you routinely run several
    /// transfers side by side and have the bandwidth to back it.
    ///
    /// Default: [`media::MAX_GLOBAL_SENDERS`](crate::media::MAX_GLOBAL_SENDERS) (12).
    pub max_tcp_connections: usize,

    /// X: how many chunk requests a single download connection keeps in
    /// flight at once ("trucks on the highway"), instead of waiting for
    /// each response before sending the next. Higher values help most on
    /// high-latency links, since round-trip time - not bandwidth - is
    /// usually the bottleneck for a single connection.
    ///
    /// Default: [`media::DEFAULT_PIPELINE_DEPTH`](crate::media::DEFAULT_PIPELINE_DEPTH) (4).
    pub download_pipeline_depth: usize,

    /// X for uploads. See [`download_pipeline_depth`](Self::download_pipeline_depth).
    ///
    /// Default: [`media::DEFAULT_PIPELINE_DEPTH`](crate::media::DEFAULT_PIPELINE_DEPTH) (4).
    pub upload_pipeline_depth: usize,

    /// Skip the size-based lookup tables for Y entirely and always use
    /// `download_tcp_connections` / `upload_tcp_connections` directly,
    /// regardless of file size.
    ///
    /// Normally Y is looked up from a fixed table (bigger files get more
    /// connections, up to your configured ceiling) - see [`crate::media`]
    /// for the tiers. With this set, every transfer in that direction
    /// opens exactly your configured ceiling worth of connections, even a
    /// small file that would otherwise only need one. Useful if you know
    /// your link's real capacity better than the size-based heuristic
    /// does, or want predictable connection counts for testing.
    ///
    /// Does not affect chunk size or X (pipeline depth) - those are
    /// unaffected by this flag either way.
    ///
    /// **This is an override, not just a tuning knob.** Turning it on
    /// means small files now open as many connections as large ones would,
    /// which adds real load Telegram wasn't expecting for that file size.
    /// If you start seeing `FLOOD_WAIT` after enabling this, that's
    /// Telegram telling you the ceiling you configured is too aggressive
    /// for this account or DC right now - turn this back off, or lower
    /// your `download_tcp_connections` / `upload_tcp_connections`, before
    /// doing anything else. See the [module docs](self) for the full
    /// responsibility note.
    ///
    /// Default: `false`.
    pub bypass_tcp_allotments: bool,
}

impl Default for TransferLimits {
    fn default() -> Self {
        Self {
            download_tcp_connections: crate::media::MAX_WORKERS_PER_FILE,
            upload_tcp_connections: crate::media::MAX_WORKERS_PER_FILE,
            max_tcp_connections: crate::media::MAX_GLOBAL_SENDERS,
            download_pipeline_depth: crate::media::DEFAULT_PIPELINE_DEPTH,
            upload_pipeline_depth: crate::media::DEFAULT_PIPELINE_DEPTH,
            bypass_tcp_allotments: false,
        }
    }
}

impl TransferLimits {
    /// Clamp to the safe range Ferogram guarantees not to misbehave in.
    ///
    /// Called once in [`ClientBuilder::build`](crate::builder::ClientBuilder::build);
    /// not exposed further because user code should never need to bypass it.
    pub(crate) fn normalized(self) -> Self {
        Self {
            download_tcp_connections: self
                .download_tcp_connections
                .clamp(1, crate::media::MAX_WORKERS_PER_FILE),
            upload_tcp_connections: self
                .upload_tcp_connections
                .clamp(1, crate::media::MAX_WORKERS_PER_FILE),
            max_tcp_connections: self.max_tcp_connections.max(1),
            download_pipeline_depth: self
                .download_pipeline_depth
                .clamp(1, crate::media::MAX_PIPELINE_DEPTH),
            upload_pipeline_depth: self
                .upload_pipeline_depth
                .clamp(1, crate::media::MAX_PIPELINE_DEPTH),
            bypass_tcp_allotments: self.bypass_tcp_allotments,
        }
    }
}
