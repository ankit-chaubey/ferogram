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

use ferogram_tl_types as tl;
use ferogram_tl_types::{Cursor, Deserializable};

use crate::error::ConnectError;

// Envelope constants
const ID_RPC_RESULT: u32 = 0xf35c6d01;
const ID_RPC_ERROR: u32 = 0x2144ca19;
const ID_MSG_CONTAINER: u32 = 0x73f1f8dc;
const ID_GZIP_PACKED: u32 = 0x3072cfa1;
const ID_MSGS_ACK: u32 = 0x62d6b459;
const ID_BAD_SERVER_SALT: u32 = 0xedab447b;
const ID_NEW_SESSION: u32 = 0x9ec20908;
const ID_BAD_MSG_NOTIFY: u32 = 0xa7eff811;
const ID_UPDATES: u32 = 0x74ae4240;
const ID_UPDATE_SHORT: u32 = 0x78d4dec1;
const ID_UPDATES_COMBINED: u32 = 0x725b04c3;
const ID_UPDATE_SHORT_MSG: u32 = 0x313bc7f8;
const ID_UPDATE_SHORT_CHAT_MSG: u32 = 0x4d6deea5;
const ID_UPDATE_SHORT_SENT_MSG: u32 = 0x9015e101;
const ID_UPDATES_TOO_LONG: u32 = 0xe317af7e;

/// Check a decrypted PFS bind response body for boolTrue.
/// Telegram always wraps the Bool in an rpc_result container:
///   rpc_result#f35c6d01 req_msg_id:long result:Bool   (16 bytes total)
/// But may also return a bare boolTrue in some implementations.
/// Decode one bare MTProto message body for the auth.bindTempAuthKey response.
///
/// Returns Ok(()) if this message body contains boolTrue (success).
/// Returns Err(msg) for real errors.
/// Returns Err("skip".to_string()) for informational messages the caller should ignore
/// (new_session_created, future_salts, msgs_ack, pong, etc.).
pub fn decode_bind_single(body: &[u8]) -> Result<(), String> {
    const RPC_RESULT: u32 = 0xf35c6d01;
    const BOOL_TRUE: u32 = 0x9972_75b5;
    const BOOL_FALSE: u32 = 0xbc79_9737;
    const RPC_ERROR: u32 = 0x2144_ca19;
    const BAD_MSG: u32 = 0xa7ef_f811;
    const BAD_SALT: u32 = 0xedab_447b;
    const NEW_SESSION: u32 = 0x9ec2_0908; // new_session_created
    const FUTURE_SALTS: u32 = 0xae50_0895;
    const MSGS_ACK: u32 = 0x62d6_b459; // msgs_ack#62d6b459
    const PONG: u32 = 0x347_73c5;

    if body.len() < 4 {
        return Err("skip".to_string());
    }
    let ctor = u32::from_le_bytes(body[..4].try_into().unwrap());

    match ctor {
        BOOL_TRUE => Ok(()),

        BOOL_FALSE => Err("server returned boolFalse (binding rejected)".to_string()),

        // Informational: not an error, caller skips these.
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
        // Bare message: decode directly.
        return decode_bind_single(body).map_err(|e| {
            if e == "skip" {
                // Informational frame (msgs_ack, new_session_created, etc.).
                // Caller should read the next frame rather than hard-fail.
                "__need_more__".to_string()
            } else {
                e
            }
        });
    }

    // msg_container#73f1f8dc messages:vector<message> = MessageContainer
    // Each message: msg_id:long seqno:int bytes:int body:bytes
    if body.len() < 8 {
        return Err("msg_container too short to read count".to_string());
    }
    let count = u32::from_le_bytes(body[4..8].try_into().unwrap()) as usize;
    let mut pos = 8usize;
    let mut last_real_err: Option<String> = None;

    for i in 0..count {
        // header: msg_id(8) + seqno(4) + bytes(4) = 16 bytes
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
            Ok(()) => return Ok(()),           // found boolTrue; done
            Err(e) if e == "skip" => continue, // new_session_created etc; normal
            Err(e) => {
                // Real error: remember it but keep iterating in case
                // a later message in the container contains boolTrue.
                last_real_err = Some(e);
            }
        }
    }

    // No message in the container returned boolTrue.
    // If last_real_err is None, every message was informational → caller should read
    // the next frame. If there was a real error, propagate it.
    Err(last_real_err.unwrap_or_else(|| "__need_more__".to_string()))
}

pub enum EnvelopeResult {
    Payload(Vec<u8>),
    /// Raw update bytes to be routed through dispatch_updates for proper pts tracking.
    RawUpdates(Vec<Vec<u8>>),
    /// updateShortSentMessage as RPC result; full struct for outgoing message reconstruction.
    SentMessage(tl::types::UpdateShortSentMessage),
    None,
}

pub fn unwrap_envelope(body: Vec<u8>) -> Result<EnvelopeResult, ConnectError> {
    if body.len() < 4 {
        return Err(ConnectError::other("body < 4 bytes"));
    }
    let cid = u32::from_le_bytes(body[..4].try_into().unwrap());

    match cid {
        ID_RPC_RESULT => {
            if body.len() < 12 {
                return Err(ConnectError::other("rpc_result too short"));
            }
            unwrap_envelope(body[12..].to_vec())
        }
        ID_RPC_ERROR => {
            if body.len() < 8 {
                return Err(ConnectError::other("rpc_error too short"));
            }
            let code    = i32::from_le_bytes(body[4..8].try_into().unwrap());
            let message = crate::util::tl_read_string(&body[8..]).unwrap_or_default();
            Err(ConnectError::Rpc { code, message })
        }
        ID_MSG_CONTAINER => {
            if body.len() < 8 {
                return Err(ConnectError::other("container too short"));
            }
            let count = u32::from_le_bytes(body[4..8].try_into().unwrap()) as usize;
            let mut pos = 8usize;
            let mut payload: Option<Vec<u8>> = None;
            let mut raw_updates: Vec<Vec<u8>> = Vec::new();

            for _ in 0..count {
                if pos + 16 > body.len() { break; }
                let inner_len = u32::from_le_bytes(body[pos + 12..pos + 16].try_into().unwrap()) as usize;
                pos += 16;
                if pos + inner_len > body.len() { break; }
                let inner = body[pos..pos + inner_len].to_vec();
                pos += inner_len;
                match unwrap_envelope(inner)? {
                    EnvelopeResult::Payload(p)          => { payload = Some(p); }
                    EnvelopeResult::RawUpdates(mut raws) => { raw_updates.append(&mut raws); }
                    EnvelopeResult::SentMessage(_)       => {} // handled via spawned task in route_frame
                    EnvelopeResult::None                 => {}
                }
            }
            if let Some(p) = payload {
                Ok(EnvelopeResult::Payload(p))
            } else if !raw_updates.is_empty() {
                Ok(EnvelopeResult::RawUpdates(raw_updates))
            } else {
                Ok(EnvelopeResult::None)
            }
        }
        ID_GZIP_PACKED => {
            let bytes = crate::util::tl_read_bytes(&body[4..]).unwrap_or_default();
            unwrap_envelope(crate::util::gz_inflate(&bytes)?)
        }
        // MTProto service messages: silently acknowledged, no payload extracted.
        // NOTE: ID_PONG is intentionally NOT listed here. Pong arrives as a bare
        // top-level frame (never inside rpc_result), so it is handled in route_frame
        // directly. Silencing it here would drop it before invoke() can resolve it.
        ID_MSGS_ACK | ID_NEW_SESSION | ID_BAD_SERVER_SALT | ID_BAD_MSG_NOTIFY
        // These are correctly silenced ( silences these too)
        | 0xd33b5459  // MsgsStateReq
        | 0x04deb57d  // MsgsStateInfo
        | 0x8cc0d131  // MsgsAllInfo
        | 0x276d3ec6  // MsgDetailedInfo
        | 0x809db6df  // MsgNewDetailedInfo
        | 0x7d861a08  // MsgResendReq / MsgResendAnsReq
        | 0x0949d9dc  // FutureSalt
        | 0xae500895  // FutureSalts
        | 0x9299359f  // HttpWait
        | 0xe22045fc  // DestroySessionOk
        | 0x62d350c9  // DestroySessionNone
        => {
            Ok(EnvelopeResult::None)
        }
        // Route all update containers via RawUpdates so route_frame can call
        // dispatch_updates, which handles pts/seq tracking. Without this, updates
        // from RPC responses (e.g. updateNewMessage + updateReadHistoryOutbox from
        // messages.sendMessage) bypass pts entirely -> false gaps -> getDifference
        // -> duplicate message delivery.
        ID_UPDATES | ID_UPDATE_SHORT | ID_UPDATES_COMBINED
        | ID_UPDATE_SHORT_MSG | ID_UPDATE_SHORT_CHAT_MSG
        | ID_UPDATES_TOO_LONG => {
            Ok(EnvelopeResult::RawUpdates(vec![body]))
        }
        // updateShortSentMessage carries pts for the bot's own sent message;
        // extract and advance the pts counter.
        ID_UPDATE_SHORT_SENT_MSG => {
            let mut cur = Cursor::from_slice(&body[4..]);
            match tl::types::UpdateShortSentMessage::deserialize(&mut cur) {
                Ok(m) => {
                    tracing::debug!(
                        "[ferogram] updateShortSentMessage (RPC): pts={} pts_count={}: advancing pts",
                        m.pts, m.pts_count
                    );
                    Ok(EnvelopeResult::SentMessage(m))
                }
                Err(e) => {
                    tracing::debug!("[ferogram] updateShortSentMessage deserialize error: {e}");
                    Ok(EnvelopeResult::None)
                }
            }
        }
        _ => Ok(EnvelopeResult::Payload(body)),
    }
}

/// Extract (users, chats) slices from any `Updates` variant.
///
/// Covers `Updates`, `UpdatesCombined`, and `UpdateShortChatMessage` /
/// `UpdateShortMessage` (which embed no entities; returns empty vecs).
/// Used to cache entities immediately after any RPC that returns `Updates`.
pub fn updates_entities(
    updates: &tl::enums::Updates,
) -> (Vec<tl::enums::User>, Vec<tl::enums::Chat>) {
    match updates {
        tl::enums::Updates::Updates(u) => (u.users.clone(), u.chats.clone()),
        tl::enums::Updates::Combined(u) => (u.users.clone(), u.chats.clone()),
        _ => (Vec::new(), Vec::new()),
    }
}

/// Convert a `Chat` enum variant to its corresponding `Peer`.
pub fn chat_to_peer(chat: &tl::enums::Chat) -> Option<tl::enums::Peer> {
    match chat {
        tl::enums::Chat::Channel(c) => Some(tl::enums::Peer::Channel(tl::types::PeerChannel {
            channel_id: c.id,
        })),
        tl::enums::Chat::ChannelForbidden(c) => {
            Some(tl::enums::Peer::Channel(tl::types::PeerChannel {
                channel_id: c.id,
            }))
        }
        tl::enums::Chat::Chat(c) => {
            Some(tl::enums::Peer::Chat(tl::types::PeerChat { chat_id: c.id }))
        }
        tl::enums::Chat::Forbidden(c) => {
            Some(tl::enums::Peer::Chat(tl::types::PeerChat { chat_id: c.id }))
        }
        tl::enums::Chat::Empty(_) => None,
    }
}
