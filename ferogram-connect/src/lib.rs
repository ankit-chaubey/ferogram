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
#![doc(html_root_url = "https://docs.rs/ferogram-connect/0.6.4")]
//! Raw TCP connection, MTProto framing, and transport for ferogram.
//!
//! This crate is part of [ferogram](https://crates.io/crates/ferogram), an async Rust
//! MTProto client built by [Ankit Chaubey](https://github.com/ankit-chaubey).
//!
//! - Channel: [t.me/Ferogram](https://t.me/Ferogram)
//! - Chat: [t.me/FerogramChat](https://t.me/FerogramChat)
//!
//! Most users do not need this crate directly. The `ferogram` crate wraps
//! everything. Use `ferogram-connect` only if you are building a custom
//! transport layer, an MTProxy relay, or need low-level control over how
//! frames are sent and received.
//!
//! # What's in here
//!
//! - **`connect_to_dc`**: Dials a Telegram DC, performs the MTProto
//!   handshake (auth key generation or reuse), and returns a [`Connection`]
//!   ready for encrypted RPC traffic.
//! - **[`TransportKind`]**: Selects the wire framing: Abridged,
//!   Intermediate, Full (default), Obfuscated2, PaddedIntermediate, or
//!   FakeTLS. Obfuscated variants are required for MTProxy and resist DPI.
//! - **[`FrameKind`]**: Runtime framing state attached to a live connection.
//!   Full transport tracks per-direction sequence numbers and CRC32;
//!   Obfuscated variants share an `Arc<Mutex<ObfuscatedCipher>>` so TX and
//!   RX run concurrently without a separate lock per direction.
//! - **`send_frame` / `recv_frame_plain`**: Frame serialisation and
//!   deserialisation helpers for the various transport shapes.
//! - **SOCKS5 / MTProxy**: [`Socks5Config`] and [`MtProxyConfig`] let you
//!   route connections through a proxy before the MTProto handshake.
//! - **PFS helpers**: [`decode_bind_response`] / [`decode_bind_single`]
//!   decode the `auth.bindTempAuthKey` response without pulling in the full
//!   TL schema crate.
//! - **Utilities**: [`gz_inflate`], [`maybe_gz_decompress`],
//!   [`build_container_body`], [`maybe_gz_pack`], [`crc32_ieee`], and
//!   friends used by the sender layer.
//!
//! # Example: establish a plain connection
//!
//! ```rust,no_run
//! use ferogram_connect::{TransportKind, connect_to_dc};
//!
//! # async fn run() -> Result<(), Box<dyn std::error::Error>> {
//! // DC 2 production address; in practice load this from your session/config.
//! let (stream, frame_kind, session) =
//!     connect_to_dc("149.154.167.51:443", 2, &TransportKind::Full, None, None).await?;
//! println!("connected, salt={}", session.salt);
//! # let _ = (stream, frame_kind);
//! # Ok(())
//! # }
//! ```

#![deny(unsafe_code)]

pub mod connection;
pub mod error;
pub mod frame;
pub mod pfs;
pub mod proxy;
pub mod socks5;
pub mod tls_record;
pub mod transport;
pub mod transport_intermediate;
pub mod transport_kind;
pub mod transport_obfuscated;
pub mod util;

pub use connection::{Connection, FrameKind, FutureSalt, connect_to_dc};
pub use error::ConnectError;
pub use frame::{faketls_read_exact, send_frame};
pub use pfs::{decode_bind_response, decode_bind_single};
pub use proxy::MtProxyConfig;
pub use socks5::Socks5Config;
pub use transport_intermediate::{
    FullTransport, IntermediateTransport, PaddedIntermediateTransport,
};
pub use transport_kind::TransportKind;
pub use transport_obfuscated::{ObfuscatedFraming, ObfuscatedStream};
pub use util::{crc32_ieee, gz_inflate, maybe_gz_decompress, random_i64, tl_read_bytes};

// Additional exports needed by ferogram crate
pub use connection::{NO_PING_DISCONNECT, PING_DELAY_SECS, SALT_USE_DELAY};
pub use frame::recv_frame_plain;

pub use util::{
    COMPRESSION_THRESHOLD, build_container_body, build_msgs_ack_body, gz_pack_body, jitter_delay,
    maybe_gz_pack, tl_read_string, tl_write_bytes,
};
