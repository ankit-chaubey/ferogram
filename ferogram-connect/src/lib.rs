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

#![deny(unsafe_code)]

pub mod connection;
pub mod envelope;
pub mod error;
pub mod frame;
pub mod proxy;
pub mod socks5;
pub mod transport;
pub mod transport_kind;
pub mod util;

pub use connection::{Connection, ConnectionWriter, FutureSalt};
pub use envelope::{EnvelopeResult, unwrap_envelope};
pub use error::ConnectError;
pub use frame::{FrameOutcome, recv_frame_with_keepalive};
pub use proxy::MtProxyConfig;
pub use socks5::Socks5Config;
pub use transport_kind::TransportKind;
pub use util::{crc32_ieee, gz_inflate, maybe_gz_decompress, random_i64, tl_read_bytes};

// Additional exports needed by ferogram crate
pub use connection::{NO_PING_DISCONNECT, PING_DELAY_SECS, SALT_USE_DELAY};
pub use envelope::{chat_to_peer, decode_bind_response, decode_bind_single, updates_entities};
pub use frame::{recv_frame_plain, recv_frame_read, send_frame, send_frame_write};
pub use transport::{recv_abridged, recv_raw_frame, send_abridged};
pub use util::{
    COMPRESSION_THRESHOLD, build_container_body, build_msgs_ack_body, gz_pack_body, jitter_delay,
    maybe_gz_pack, tl_read_string, tl_write_bytes,
};
