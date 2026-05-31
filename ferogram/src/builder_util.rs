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

//! Internal helpers for `ClientBuilder`.

use ferogram_session::{DcEntry, DcFlags, PersistedSession, string_session::StringSession};

/// Try to decode `s` as a compact V1/V2 `StringSession`.
/// Returns `None` for ferogram native format, empty string, or decode failure.
pub(crate) fn detect_compact_session(s: &str) -> Option<PersistedSession> {
    let ss = StringSession::decode(s).ok()?;
    let session = ss.session();

    let ip = session.ip;
    let flags = if ip.is_ipv6() {
        DcFlags::IPV6
    } else {
        DcFlags::NONE
    };

    let dc_entry = DcEntry {
        dc_id: session.dc_id as i32,
        addr: if ip.is_ipv6() {
            format!("[{}]:{}", ip, session.port)
        } else {
            format!("{}:{}", ip, session.port)
        },
        auth_key: Some(session.auth_key),
        first_salt: ss.full_session().map(|f| f.server_salt).unwrap_or(0),
        time_offset: 0,
        flags,
    };

    let mut persisted = PersistedSession {
        home_dc_id: session.dc_id as i32,
        ..Default::default()
    };
    persisted.dcs.push(dc_entry);
    Some(persisted)
}
