// Copyright (c) Ankit Chaubey <ankitchaubey.dev@gmail.com>
//
// ferogram: async Telegram MTProto client in Rust
// https://github.com/ankit-chaubey/ferogram
//
// Licensed under either the MIT License or the Apache License 2.0.

use ferogram_tl_types as tl;

/// Convenience extension for [`tl::enums::Peer`] — extract the numeric ID
/// without writing a `match` every time.
///
/// # Example
/// ```rust,no_run
/// use ferogram::PeerExt;
/// use ferogram::OptionPeerExt;
/// use ferogram::tl;
/// # use ferogram::update::IncomingMessage;
/// # fn example(peer: tl::enums::Peer, msg: IncomingMessage) {
///
/// // Instead of:
/// // let id = match peer { Peer::User(u) => u.user_id, Peer::Chat(c) => c.chat_id, ... };
/// // Just write:
/// let id = peer.bare_id();
///
/// // Works great with sender_id() / peer_id() on IncomingMessage:
/// if let Some(id) = msg.sender_id().bare_id() {
///     println!("sender: {id}");
/// }
/// // Chat ID:
/// if let Some(id) = msg.peer_id().bare_id() {
///     println!("chat: {id}");
/// }
/// # }
/// ```
pub trait PeerExt {
    /// Returns the raw Telegram ID for any peer variant.
    ///
    /// - `Peer::User`    → `user_id`
    /// - `Peer::Chat`    → `chat_id`   (basic group)
    /// - `Peer::Channel` → `channel_id` (supergroup / broadcast channel)
    ///
    /// Note: these are **native** Telegram IDs, not Bot-API-encoded ones.
    /// A channel with native ID `1234567890` would be `-1001234567890` in the
    /// Bot API.  Use [`crate::peer_ref`] encoding if you need the Bot-API form.
    fn bare_id(&self) -> i64;
}

impl PeerExt for tl::enums::Peer {
    #[inline]
    fn bare_id(&self) -> i64 {
        match self {
            tl::enums::Peer::User(u) => u.user_id,
            tl::enums::Peer::Chat(c) => c.chat_id,
            tl::enums::Peer::Channel(c) => c.channel_id,
        }
    }
}

/// Same convenience for `Option<&tl::enums::Peer>` — lets you write
/// `msg.sender_id().bare_id()` instead of `.map(|p| p.bare_id())`.
pub trait OptionPeerExt {
    fn bare_id(&self) -> Option<i64>;
}

impl OptionPeerExt for Option<&tl::enums::Peer> {
    #[inline]
    fn bare_id(&self) -> Option<i64> {
        self.map(PeerExt::bare_id)
    }
}
