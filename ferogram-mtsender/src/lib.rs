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

#![cfg_attr(docsrs, feature(doc_cfg))]
#![doc(html_root_url = "https://docs.rs/ferogram-mtsender/0.6.2")]
//! MTProto sender pool and retry policy for ferogram.
//!
//! This crate is part of [ferogram](https://crates.io/crates/ferogram), an async Rust
//! MTProto client built by [Ankit Chaubey](https://github.com/ankit-chaubey).
//!
//! - Channel: [t.me/Ferogram](https://t.me/Ferogram)
//! - Chat: [t.me/FerogramChat](https://t.me/FerogramChat)
//!
//! Most users do not need this crate directly. The `ferogram` crate wraps
//! everything. Use `ferogram-mtsender` only if you are building a custom
//! dispatch layer or need direct access to the DC connection pool.
//!
//! # What's in here
//!
//! - **[`MtpSender`]**: Single-task MTProto sender. All TCP I/O (read, write,
//!   ping) runs inside one Tokio task that owns the unsplit `TcpStream`.
//!   Callers enqueue request bodies via [`MtpSender::enqueue`] and receive
//!   results through a oneshot channel. This design eliminates mutex
//!   contention between reader and writer halves and ensures ACKs are flushed
//!   on every outgoing frame.
//! - **[`DcPool`]**: Per-DC connection pool capped at three slots. Requests
//!   are round-robined across live [`ConnSlot`]s. The pool opens new
//!   connections on demand and replaces slots that have faulted.
//! - **[`DcConnection`]**: One encrypted connection to a single DC. Handles
//!   pending `msgs_ack` accumulation and issues `ping_delay_disconnect` to
//!   keep the socket alive inside Telegram's 75-second idle window.
//! - **[`RetryPolicy`] / [`RetryLoop`]**: Trait and executor for retry
//!   behaviour on RPC failures. [`AutoSleep`] sleeps through `FLOOD_WAIT`
//!   errors; [`NoRetries`] returns the error immediately. Implement the
//!   trait to add exponential back-off or a circuit breaker.
//! - **[`CircuitBreaker`]**: Stops issuing requests to a DC that has failed
//!   repeatedly, giving the pool time to reconnect before hammering the
//!   server.
//! - **`spawn_sender_task` / [`SenderHandle`]**: Spawns the background I/O
//!   loop and returns a handle for enqueuing RPCs, subscribing to
//!   [`FrameEvent`]s (updates), and issuing [`ReconnectRequest`]s.
//! - **[`InvocationError`] / [`RpcError`]**: Typed errors for failed RPC
//!   calls, including flood waits, server errors, and transport failures.
//!
//! # Example: send an RPC via the pool
//!
//! ```rust,no_run
//! use ferogram_mtsender::{DcPool, AutoSleep};
//! use ferogram_tl_types::functions::help::GetConfig;
//!
//! # async fn run() -> Result<(), Box<dyn std::error::Error>> {
//! let pool = DcPool::connect(2, Default::default()).await?;
//! let config = pool.invoke(GetConfig {}, AutoSleep::default()).await?;
//! println!("DC count: {}", config.dc_options.len());
//! # Ok(())
//! # }
//! ```

#![deny(unsafe_code)]

mod errors;
pub mod mtp_sender;
mod pool;
mod retry;
mod sender;
pub mod sender_task;

pub use errors::{InvocationError, RpcError};
pub use mtp_sender::MtpSender;
pub use pool::{ConnSlot, DcPool};
pub use retry::{AutoSleep, CircuitBreaker, NoRetries, RetryContext, RetryLoop, RetryPolicy};
pub use sender::DcConnection;
pub use sender_task::{FrameEvent, ReconnectRequest, RpcEnqueue, SenderHandle, spawn_sender_task};
