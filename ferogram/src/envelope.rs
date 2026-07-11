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

//! Update entity extraction and chat/peer conversion helpers.
//!
//! This module lives in the top crate because it touches `tl-api` types
//! (`User`, `Chat`, `Peer`). Envelope/frame unwrapping itself now lives in
//! `ferogram-mtsender::mtp_sender::dispatch`. The transport-layer PFS bind
//! decoder (no tl-api dependency) lives in `ferogram-connect/src/pfs.rs`.

use ferogram_tl_types as tl;

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
        // No `Peer::Community` variant exists (layer 228 didn't add one) -
        // communities aren't addressable as a `Peer` at all.
        tl::enums::Chat::Community(_) => None,
        tl::enums::Chat::CommunityForbidden(_) => None,
    }
}
