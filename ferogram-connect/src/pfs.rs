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

//! PFS (Perfect Forward Secrecy) bind response decoder.
//!
//! Pure byte-slice logic; no `tl-api` dependency.

/// Decode one bare MTProto message body for the auth.bindTempAuthKey response.
///
/// Returns `Ok(())` if this message body contains boolTrue (success).
/// Returns `Err("skip")` for informational messages the caller should ignore
/// (new_session_created, future_salts, msgs_ack, pong, etc.).
/// Returns `Err(msg)` for real errors.
pub fn decode_bind_single(body: &[u8]) -> Result<(), String> {
    const RPC_RESULT: u32 = 0xf35c6d01;
    const BOOL_TRUE: u32 = 0x9972_75b5;
    const BOOL_FALSE: u32 = 0xbc79_9737;
    const RPC_ERROR: u32 = 0x2144_ca19;
    const BAD_MSG: u32 = 0xa7ef_f811;
    const BAD_SALT: u32 = 0xedab_447b;
    const NEW_SESSION: u32 = 0x9ec2_0908;
    const FUTURE_SALTS: u32 = 0xae50_0895;
    const MSGS_ACK: u32 = 0x62d6_b459;
    const PONG: u32 = 0x0347_73c5;

    if body.len() < 4 {
        return Err("skip".to_string());
    }
    let ctor = u32::from_le_bytes(body[..4].try_into().unwrap());

    match ctor {
        BOOL_TRUE => Ok(()),

        BOOL_FALSE => Err("server returned boolFalse (binding rejected)".to_string()),

        NEW_SESSION | FUTURE_SALTS | MSGS_ACK | PONG => Err("skip".to_string()),

        RPC_RESULT if body.len() >= 16 => {
            let inner = u32::from_le_bytes(body[12..16].try_into().unwrap());
            match inner {
                BOOL_TRUE => Ok(()),
                BOOL_FALSE => Err("rpc_result{boolFalse} (server rejected binding)".to_string()),
                RPC_ERROR if body.len() >= 20 => {
                    let code = i32::from_le_bytes(body[16..20].try_into().unwrap());
                    let msg = crate::util::tl_read_string(body.get(20..).unwrap_or(&[]))
                        .unwrap_or_default();
                    Err(format!("rpc_error code={code} message={msg:?}"))
                }
                _ => Err(format!("rpc_result inner ctor={inner:#010x}")),
            }
        }

        BAD_MSG if body.len() >= 16 => {
            let code = u32::from_le_bytes(body[12..16].try_into().unwrap());
            let desc = match code {
                16 => "msg_id too low (clock skew)",
                17 => "msg_id too high (clock skew)",
                18 => "incorrect lower 2 bits of msg_id",
                19 => "duplicate msg_id",
                20 => "message too old (>300s)",
                32 => "msg_seqno too low",
                33 => "msg_seqno too high",
                34 => "even seqno expected, odd received",
                35 => "odd seqno expected, even received",
                48 => "incorrect server salt",
                64 => "invalid container",
                _ => "unknown code",
            };
            Err(format!("bad_msg_notification code={code} ({desc})"))
        }

        BAD_SALT if body.len() >= 24 => {
            let new_salt = i64::from_le_bytes(body[16..24].try_into().unwrap());
            Err(format!(
                "bad_server_salt, server wants salt={new_salt:#018x}"
            ))
        }

        _ => Err(format!("unknown ctor={ctor:#010x}")),
    }
}

/// Decode the server response to auth.bindTempAuthKey.
///
/// Handles bare messages AND msg_container (the server frequently bundles
/// new_session_created + rpc_result together in a container on the very first
/// encrypted message of a fresh temp session).
pub fn decode_bind_response(body: &[u8]) -> Result<(), String> {
    const MSG_CONTAINER: u32 = 0x73f1f8dc;

    if body.len() < 4 {
        return Err(format!("response body too short ({} bytes)", body.len()));
    }
    let ctor = u32::from_le_bytes(body[..4].try_into().unwrap());

    if ctor != MSG_CONTAINER {
        return decode_bind_single(body).map_err(|e| {
            if e == "skip" {
                "__need_more__".to_string()
            } else {
                e
            }
        });
    }

    if body.len() < 8 {
        return Err("msg_container too short to read count".to_string());
    }
    let count = u32::from_le_bytes(body[4..8].try_into().unwrap()) as usize;
    let mut pos = 8usize;
    let mut last_real_err: Option<String> = None;

    for i in 0..count {
        if pos + 16 > body.len() {
            return Err(format!(
                "msg_container truncated at message {i}/{count} (pos={pos} body_len={})",
                body.len()
            ));
        }
        let msg_bytes = u32::from_le_bytes(body[pos + 12..pos + 16].try_into().unwrap()) as usize;
        pos += 16;

        if pos + msg_bytes > body.len() {
            return Err(format!(
                "msg_container message {i} body overflows (need {msg_bytes}, have {})",
                body.len() - pos
            ));
        }
        let msg_body = &body[pos..pos + msg_bytes];
        pos += msg_bytes;

        match decode_bind_single(msg_body) {
            Ok(()) => return Ok(()),
            Err(e) if e == "skip" => continue,
            Err(e) => {
                last_real_err = Some(e);
            }
        }
    }

    Err(last_real_err.unwrap_or_else(|| "__need_more__".to_string()))
}
