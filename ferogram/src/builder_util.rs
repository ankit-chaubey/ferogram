/*
 * Copyright (c) 2026 Ankit Chaubey <ankitchaubey.dev@gmail.com>
 * https://github.com/ankit-chaubey
 *
 * Project: ferogram
 * Website: https://ferogram.dev
 *
 * Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
 * https://www.apache.org/licenses/LICENSE-2.0> or the MIT license
 * <LICENSE-MIT or https://opensource.org/licenses/MIT>, at your option.
 * This file may not be copied, modified, or distributed except according
 * to those terms.
 */

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

    let persisted = PersistedSession {
        home_dc_id: session.dc_id as i32,
        dcs: vec![dc_entry],
        ..Default::default()
    };
    Some(persisted)
}
