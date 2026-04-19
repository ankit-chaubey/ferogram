// Copyright (c) Ankit Chaubey <ankitchaubey.dev@gmail.com>
// SPDX-License-Identifier: MIT OR Apache-2.0
//
// ferogram: async Telegram MTProto client in Rust
// https://github.com/ankit-chaubey/ferogram
//
//
// If you use or modify this code, keep this notice at the top of your file
// and include the LICENSE-MIT or LICENSE-APACHE file from this repository:
// https://github.com/ankit-chaubey/ferogram

//! [`PeerRef`]: flexible peer argument accepted by all `Client` methods.
//!
//! Every public API method that previously required a bare `tl::enums::Peer`
//! now accepts `impl Into<PeerRef>`, so you can pass any of:
//!
//! ```rust,no_run
//! # use ferogram::Client;
//! # async fn f(client: Client) -> anyhow::Result<()> {
//! // @username string
//! client.send_message_to_peer("@durov", "hi").await?;
//!
//! // bare username (no @)
//! client.send_message_to_peer("durov", "hi").await?;
//!
//! // "me" / "self"
//! client.send_message_to_peer("me", "hi").await?;
//!
//! // numeric user ID (positive)
//! client.send_message_to_peer(12345678_i64, "hi").await?;
//!
//! // Bot-API channel ID (negative, -100... prefix)
//! client.iter_messages(-1001234567890_i64);
//!
//! // Bot-API basic-group ID (negative, small)
//! client.mark_as_read(-123456_i64).await?;
//!
//! // already-resolved peer: zero overhead
//! use ferogram::tl;
//! let peer = tl::enums::Peer::User(tl::types::PeerUser { user_id: 123 });
//! client.send_message_to_peer(peer, "hi").await?;
//! # Ok(()) }
//! ```

use ferogram_tl_types as tl;

/// Telegram channels / supergroups use this offset in the Bot API negative-ID
/// encoding: `bot_api_id = -1_000_000_000_000 - channel_id`.
const ZERO_CHANNEL_ID: i64 = -1_000_000_000_000;

/// Flexible reference to a Telegram peer (user, basic group, or channel).
///
/// Construct via any of the `From` impls or by wrapping an already-resolved
/// `tl::enums::Peer`.  Use `peer_ref.resolve(&client).await` to obtain the
/// underlying `tl::enums::Peer`, performing a network lookup only when the
/// username is not yet cached.
#[derive(Clone, Debug)]
pub enum PeerRef {
    /// `"@username"`, `"username"`, `"me"`, or `"self"`.
    Username(String),
    /// Numeric ID.
    ///
    /// Positive → user.  
    /// Negative above `−1 000 000 000 000` → basic group (`chat_id = -id`).  
    /// Negative ≤ `−1 000 000 000 000` → channel/supergroup
    /// (`channel_id = -id - 1_000_000_000_000`).
    Id(i64),
    /// Already-resolved TL peer: forwarded at zero cost.
    Peer(tl::enums::Peer),
}

impl PeerRef {
    /// Resolve this reference to a `tl::enums::Peer`.
    ///
    /// * `Peer` variant → returned immediately.
    /// * `Id` variant   → decoded from Bot-API encoding, no network call.
    /// * `Username` variant → may perform a `contacts.resolveUsername` RPC
    ///   if not already cached.
    pub async fn resolve(
        self,
        client: &crate::Client,
    ) -> Result<tl::enums::Peer, crate::InvocationError> {
        match self {
            PeerRef::Peer(p) => Ok(p),

            PeerRef::Id(id) if id > 0 => {
                Ok(tl::enums::Peer::User(tl::types::PeerUser { user_id: id }))
            }
            PeerRef::Id(id) if id <= ZERO_CHANNEL_ID => {
                let channel_id = -id - 1_000_000_000_000;
                Ok(tl::enums::Peer::Channel(tl::types::PeerChannel {
                    channel_id,
                }))
            }
            PeerRef::Id(id) => {
                let chat_id = -id;
                Ok(tl::enums::Peer::Chat(tl::types::PeerChat { chat_id }))
            }

            PeerRef::Username(s) => client.resolve_peer(&s).await,
        }
    }
}

impl From<&str> for PeerRef {
    fn from(s: &str) -> Self {
        PeerRef::Username(s.to_owned())
    }
}

impl From<String> for PeerRef {
    fn from(s: String) -> Self {
        PeerRef::Username(s)
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
