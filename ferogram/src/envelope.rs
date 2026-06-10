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

//! MTProto envelope unwrapping and update entity extraction.
//!
//! This module lives in the top crate because it touches `tl-api` types
//! (`Updates`, `User`, `Chat`, `Peer`, `UpdateShortSentMessage`).
//! The transport-layer PFS bind decoder (no tl-api dependency) lives in
//! `ferogram-connect/src/pfs.rs`.

use ferogram_tl_types as tl;
use ferogram_tl_types::{Cursor, Deserializable};

use ferogram_connect::error::ConnectError;

// Envelope constructor IDs
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

pub enum EnvelopeResult {
    Payload(Vec<u8>),
    /// Raw update bytes to be routed through dispatch_updates for proper pts tracking.
    RawUpdates(Vec<Vec<u8>>),
    /// updateShortSentMessage as RPC result; full struct for outgoing message reconstruction.
    SentMessage(Box<tl::types::UpdateShortSentMessage>),
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
            let message = ferogram_connect::util::tl_read_string(&body[8..]).unwrap_or_default();
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
                    EnvelopeResult::Payload(p)           => { payload = Some(p); }
                    EnvelopeResult::RawUpdates(mut raws) => { raw_updates.append(&mut raws); }
                    EnvelopeResult::SentMessage(_)        => {}
                    EnvelopeResult::None                  => {}
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
            let bytes = ferogram_connect::util::tl_read_bytes(&body[4..]).unwrap_or_default();
            unwrap_envelope(ferogram_connect::gz_inflate(&bytes)?)
        }
        ID_MSGS_ACK | ID_NEW_SESSION | ID_BAD_SERVER_SALT | ID_BAD_MSG_NOTIFY
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
        ID_UPDATES | ID_UPDATE_SHORT | ID_UPDATES_COMBINED
        | ID_UPDATE_SHORT_MSG | ID_UPDATE_SHORT_CHAT_MSG
        | ID_UPDATES_TOO_LONG => {
            Ok(EnvelopeResult::RawUpdates(vec![body]))
        }
        ID_UPDATE_SHORT_SENT_MSG => {
            let mut cur = Cursor::from_slice(&body[4..]);
            match tl::types::UpdateShortSentMessage::deserialize(&mut cur) {
                Ok(m) => {
                    tracing::debug!(
                        "[ferogram] updateShortSentMessage (RPC): pts={} pts_count={}: advancing pts",
                        m.pts, m.pts_count
                    );
                    Ok(EnvelopeResult::SentMessage(Box::new(m)))
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
