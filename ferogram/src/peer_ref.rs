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

use std::borrow::Cow;

use crate::PeerCache;

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
                    let mut cache: tokio::sync::RwLockWriteGuard<'_, PeerCache> =
                        client.inner.peer_cache.write().await;
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
                    let cache: tokio::sync::RwLockReadGuard<'_, PeerCache> =
                        client.inner.peer_cache.read().await;
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
                    let cache: tokio::sync::RwLockReadGuard<'_, PeerCache> =
                        client.inner.peer_cache.read().await;
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
        let cache: tokio::sync::RwLockReadGuard<'_, PeerCache> =
            client.inner.peer_cache.read().await;
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

/// The token portion of an invite link, e.g. the `HASH` in `t.me/+HASH`.
///
/// `Client::join_link` and `Client::check_invite` take a full link and
/// extract this internally. If you already have the bare hash (for
/// example, stored from a previous call), use [`InviteHash::new`] instead
/// of rebuilding a fake link just to have it parsed back out.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InviteHash(String);

impl InviteHash {
    /// Wrap a hash you already have.
    pub fn new(hash: impl Into<String>) -> Self {
        Self(hash.into())
    }

    /// Extract the hash from a full invite link.
    ///
    /// Accepts `t.me/+HASH`, `t.me/joinchat/HASH`, and `tg://join?invite=HASH`.
    /// Returns `None` if `link` doesn't match any of those patterns.
    pub fn from_link(link: &str) -> Option<Self> {
        PeerRef::parse_invite_hash(link).map(|h| Self(h.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl AsRef<str> for InviteHash {
    fn as_ref(&self) -> &str {
        &self.0
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

impl std::str::FromStr for PeerRef {
    type Err = std::convert::Infallible;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(normalize_str(s))
    }
}

impl<'a> From<Cow<'a, str>> for PeerRef {
    fn from(cow: Cow<'a, str>) -> Self {
        match cow {
            Cow::Borrowed(s) => normalize_str(s),
            Cow::Owned(s) => normalize_owned(s),
        }
    }
}

/// Same rules as `normalize_str`, but reuses the `String` we already own
/// instead of allocating a new one, for the two branches where that's
/// possible (a clean phone number, or a clean plain username).
///
/// The invite hash and t.me-username branches always need a substring of
/// `s`, so there's nothing to save there, they fall back to `normalize_str`.
fn normalize_owned(mut s: String) -> PeerRef {
    let trimmed = s.trim();
    if trimmed.len() != s.len() {
        // Whitespace to strip: no cheap path, drop into the borrowed logic.
        return normalize_str(trimmed);
    }

    if PeerRef::parse_invite_hash(&s).is_some() || parse_tme_username(&s).is_some() {
        return normalize_str(&s);
    }

    if s.starts_with('+') && s.len() > 5 && s[1..].chars().all(|c| c.is_ascii_digit()) {
        return PeerRef::Phone(s);
    }

    if s.parse::<i64>().is_ok() {
        // Parsing doesn't allocate, so there's no benefit to reusing s here.
        return normalize_str(&s);
    }

    // trim_start_matches('@') strips every leading '@', not just one, match
    // that exactly so this path never disagrees with normalize_str.
    let leading_ats = s.bytes().take_while(|&b| b == b'@').count();
    if leading_ats > 0 {
        s.drain(..leading_ats);
    }

    PeerRef::Username(s)
}
