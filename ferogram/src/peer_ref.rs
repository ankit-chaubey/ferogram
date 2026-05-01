// Copyright (c) Ankit Chaubey <ankitchaubey.dev@gmail.com>
// SPDX-License-Identifier: MIT OR Apache-2.0
//
// ferogram: async Telegram MTProto client in Rust
// https://github.com/ankit-chaubey/ferogram

use ferogram_tl_types as tl;

// channel_id = -(bot_api_id + 1_000_000_000_000)
const ZERO_CHANNEL_ID: i64 = -1_000_000_000_000;

/// Flexible peer argument accepted by every `Client` method.
///
/// You rarely construct this directly. Pass any of the following and it is
/// converted automatically:
///
/// - `"@username"` or `"username"`
/// - `"me"` or `"self"`
/// - Numeric string `"123456789"` (Bot-API encoded)
/// - `i64` or `i32`
/// - `t.me/<username>` URL
/// - Invite link (`t.me/+HASH`, `t.me/joinchat/HASH`, `tg://join?invite=HASH`)
/// - E.164 phone number (`"+12025551234"`)
/// - `tl::enums::Peer` or `tl::enums::InputPeer`
#[derive(Clone, Debug)]
pub enum PeerRef {
    Username(String),
    Id(i64),
    Peer(tl::enums::Peer),
    Input(tl::enums::InputPeer),
    InviteHash(String),
    Phone(String),
}

impl PeerRef {
    /// Extract the hash portion from an invite link, or `None`.
    ///
    /// Handles `t.me/+HASH`, `t.me/joinchat/HASH`, and `tg://join?invite=HASH`.
    pub fn parse_invite_hash(link: &str) -> Option<&str> {
        if let Some(rest) = link.strip_prefix("tg://join?invite=") {
            let hash = rest.split('&').next().unwrap_or(rest);
            if !hash.is_empty() {
                return Some(hash);
            }
        }
        if let Some(pos) = link.find("/+") {
            let hash = &link[pos + 2..];
            if !hash.is_empty() {
                return Some(hash.split('?').next().unwrap_or(hash));
            }
        }
        if let Some(pos) = link.find("/joinchat/") {
            let hash = &link[pos + 10..];
            if !hash.is_empty() {
                return Some(hash.split('?').next().unwrap_or(hash));
            }
        }
        None
    }

    /// Resolve to a `tl::enums::Peer`. An RPC is only made on a cache miss.
    pub async fn resolve(
        self,
        client: &crate::Client,
    ) -> Result<tl::enums::Peer, crate::InvocationError> {
        match self {
            PeerRef::Peer(p) => Ok(p),

            PeerRef::Id(id) => resolve_id(id, client).await,

            PeerRef::Input(ip) => {
                {
                    let mut cache = client.inner.peer_cache.write().await;
                    cache.cache_input_peer(&ip);
                }
                input_peer_to_peer(ip)
            }

            PeerRef::Username(s) => {
                let s = s.trim().trim_start_matches('@').to_owned();

                if s == "me" || s == "self" {
                    return Ok(tl::enums::Peer::User(tl::types::PeerUser { user_id: 0 }));
                }

                // Numeric string: inline the Id path to avoid async recursion.
                if let Ok(id) = s.parse::<i64>() {
                    return resolve_id(id, client).await;
                }

                // Cache index first.
                {
                    let cache = client.inner.peer_cache.read().await;
                    if let Some(&(id, ref ty)) = cache.username_to_peer.get(&s.to_lowercase()) {
                        let peer = match ty {
                            crate::PeerType::User => {
                                tl::enums::Peer::User(tl::types::PeerUser { user_id: id })
                            }
                            crate::PeerType::Channel => {
                                tl::enums::Peer::Channel(tl::types::PeerChannel { channel_id: id })
                            }
                            crate::PeerType::Chat => {
                                tl::enums::Peer::Chat(tl::types::PeerChat { chat_id: id })
                            }
                        };
                        if cache.peer_to_input(&peer).is_ok() {
                            return Ok(peer);
                        }
                        // stale index entry: fall through to RPC
                    }
                }

                client.resolve_username_rpc(&s).await
            }

            PeerRef::Phone(phone) => {
                {
                    let cache = client.inner.peer_cache.read().await;
                    if let Some(&uid) = cache.phone_to_user.get(&phone)
                        && cache.user_input_peer(uid).is_ok()
                    {
                        return Ok(tl::enums::Peer::User(tl::types::PeerUser { user_id: uid }));
                    }
                }
                client.resolve_phone_rpc(&phone).await
            }

            PeerRef::InviteHash(hash) => client.resolve_invite_hash_rpc(&hash).await,
        }
    }
}

/// Decode a Bot-API numeric ID and resolve it, fetching from Telegram on a
/// cache miss. Basic groups never need a hash and are returned immediately.
async fn resolve_id(
    id: i64,
    client: &crate::Client,
) -> Result<tl::enums::Peer, crate::InvocationError> {
    let decoded = decode_bot_api_id(id);

    if matches!(decoded, tl::enums::Peer::Chat(_)) {
        return Ok(decoded);
    }

    {
        let cache = client.inner.peer_cache.read().await;
        if cache.peer_to_input(&decoded).is_ok() {
            return Ok(decoded);
        }
    }

    client.fetch_by_id_rpc(decoded).await
}

/// Decode a Bot-API-encoded integer to a `Peer`.
///
/// Positive  -> User
/// In (-1_000_000_000_000, 0)  -> basic group Chat
/// <= -1_000_000_000_000  -> Channel
fn decode_bot_api_id(id: i64) -> tl::enums::Peer {
    if id > 0 {
        tl::enums::Peer::User(tl::types::PeerUser { user_id: id })
    } else if id <= ZERO_CHANNEL_ID {
        let channel_id = -(id + 1_000_000_000_000);
        tl::enums::Peer::Channel(tl::types::PeerChannel { channel_id })
    } else {
        tl::enums::Peer::Chat(tl::types::PeerChat { chat_id: -id })
    }
}

/// Strip an `InputPeer` down to its bare `Peer` key.
///
/// The access hash was already written into the peer cache by
/// `cache_input_peer()` before this is called, so `peer_to_input()` can
/// reconstruct the full `InputPeer` (including `UserFromMessage`) from
/// the bare `Peer` on the next call.
fn input_peer_to_peer(ip: tl::enums::InputPeer) -> Result<tl::enums::Peer, crate::InvocationError> {
    match ip {
        tl::enums::InputPeer::PeerSelf => {
            Ok(tl::enums::Peer::User(tl::types::PeerUser { user_id: 0 }))
        }
        tl::enums::InputPeer::User(u) => Ok(tl::enums::Peer::User(tl::types::PeerUser {
            user_id: u.user_id,
        })),
        tl::enums::InputPeer::Chat(c) => Ok(tl::enums::Peer::Chat(tl::types::PeerChat {
            chat_id: c.chat_id,
        })),
        tl::enums::InputPeer::Channel(c) => Ok(tl::enums::Peer::Channel(tl::types::PeerChannel {
            channel_id: c.channel_id,
        })),
        tl::enums::InputPeer::UserFromMessage(u) => {
            Ok(tl::enums::Peer::User(tl::types::PeerUser {
                user_id: u.user_id,
            }))
        }
        tl::enums::InputPeer::ChannelFromMessage(c) => {
            Ok(tl::enums::Peer::Channel(tl::types::PeerChannel {
                channel_id: c.channel_id,
            }))
        }
        tl::enums::InputPeer::Empty => Err(crate::InvocationError::Deserialize(
            "cannot resolve InputPeer::Empty".into(),
        )),
    }
}

// From impls

impl From<&str> for PeerRef {
    fn from(s: &str) -> Self {
        normalize_str(s)
    }
}

impl From<String> for PeerRef {
    fn from(s: String) -> Self {
        normalize_str(&s)
    }
}

impl From<i64> for PeerRef {
    fn from(id: i64) -> Self {
        PeerRef::Id(id)
    }
}

impl From<i32> for PeerRef {
    fn from(id: i32) -> Self {
        PeerRef::Id(id as i64)
    }
}

impl From<tl::enums::Peer> for PeerRef {
    fn from(p: tl::enums::Peer) -> Self {
        PeerRef::Peer(p)
    }
}

impl From<tl::enums::InputPeer> for PeerRef {
    fn from(ip: tl::enums::InputPeer) -> Self {
        PeerRef::Input(ip)
    }
}

fn normalize_str(s: &str) -> PeerRef {
    let s = s.trim();

    if let Some(hash) = PeerRef::parse_invite_hash(s) {
        return PeerRef::InviteHash(hash.to_owned());
    }

    if let Some(uname) = parse_tme_username(s) {
        return PeerRef::Username(uname.to_owned());
    }

    if s.starts_with('+') && s.len() > 5 && s[1..].chars().all(|c| c.is_ascii_digit()) {
        return PeerRef::Phone(s.to_owned());
    }

    if let Ok(id) = s.parse::<i64>() {
        return PeerRef::Id(id);
    }

    PeerRef::Username(s.trim_start_matches('@').to_owned())
}

/// Extract a username from a `t.me/<username>` URL.
/// Returns `None` for invite links, channel message links (`t.me/c/...`),
/// and non-t.me strings.
fn parse_tme_username(s: &str) -> Option<&str> {
    let path = s
        .strip_prefix("https://t.me/")
        .or_else(|| s.strip_prefix("http://t.me/"))
        .or_else(|| s.strip_prefix("https://telegram.me/"))
        .or_else(|| s.strip_prefix("http://telegram.me/"))?;

    if path.starts_with('+') || path.starts_with("joinchat/") {
        return None;
    }

    // t.me/c/<channel_id>/<msg_id> - reject, "c" is not a username
    if path.starts_with("c/") {
        return None;
    }

    path.split('/').next().filter(|u| !u.is_empty())
}
