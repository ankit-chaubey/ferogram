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

//! Parsing of `t.me` message links, with optional forum topic IDs.

use crate::PeerRef;

/// Offset between a channel ID and its Bot API ID (`-100<id>`).
const ZERO_CHANNEL_ID: i64 = -1_000_000_000_000;

/// Where a message link points.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkPeer {
    /// A public chat, `t.me/<username>/...`.
    Username(String),
    /// A private channel or supergroup, `t.me/c/<channel_id>/...`.
    PrivateChannel(i64),
}

/// A parsed `t.me` message link.
///
/// Supported forms:
///
/// - `t.me/<username>/<message_id>`
/// - `t.me/c/<channel_id>/<message_id>`
/// - `t.me/c/<channel_id>/<topic_id>/<message_id>`
///
/// The scheme may be `https://`, `http://` or absent, and the host `t.me`,
/// `telegram.me` or `telegram.dog`. A query string or fragment is ignored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MessageLink {
    /// The chat the message is in.
    pub peer: LinkPeer,
    /// Forum topic ID (the topic root message ID), if the link has one.
    ///
    /// Only private links carry a topic ID.
    pub topic_id: Option<i32>,
    /// The message ID.
    pub message_id: i32,
}

impl MessageLink {
    /// Parse a message link, or return `None` if `link` is not one.
    pub fn parse(link: &str) -> Option<Self> {
        let link = link.trim();
        let path = [
            "https://t.me/",
            "http://t.me/",
            "https://telegram.me/",
            "http://telegram.me/",
            "https://telegram.dog/",
            "http://telegram.dog/",
            "t.me/",
            "telegram.me/",
            "telegram.dog/",
        ]
        .iter()
        .find_map(|p| link.strip_prefix(p))?;

        let path = path.split(['?', '#']).next()?;
        let parts: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();

        let id = |s: &str| s.parse::<i32>().ok().filter(|n| *n > 0);

        match parts.as_slice() {
            ["c", channel, msg] => Some(Self {
                peer: LinkPeer::PrivateChannel(channel.parse::<i64>().ok().filter(|n| *n > 0)?),
                topic_id: None,
                message_id: id(msg)?,
            }),
            ["c", channel, topic, msg] => Some(Self {
                peer: LinkPeer::PrivateChannel(channel.parse::<i64>().ok().filter(|n| *n > 0)?),
                topic_id: Some(id(topic)?),
                message_id: id(msg)?,
            }),
            [username, msg] if is_username(username) => Some(Self {
                peer: LinkPeer::Username((*username).to_owned()),
                topic_id: None,
                message_id: id(msg)?,
            }),
            _ => None,
        }
    }

    /// The peer to resolve, as a [`PeerRef`].
    ///
    /// A private channel becomes its Bot API ID (`-100<channel_id>`), so the
    /// channel must already be known to the client, for example because the
    /// account is a member.
    pub fn peer_ref(&self) -> PeerRef {
        match &self.peer {
            LinkPeer::Username(u) => PeerRef::Username(u.clone()),
            LinkPeer::PrivateChannel(id) => PeerRef::Id(ZERO_CHANNEL_ID - id),
        }
    }
}

fn is_username(s: &str) -> bool {
    s != "c"
        && !s.starts_with('+')
        && !s.is_empty()
        && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Whether a message belongs to forum topic `topic_id`.
///
/// - The topic root message itself matches (`message_id == topic_id`).
/// - Otherwise the message matches when its reply-to-top ID equals the topic
///   ID, falling back to its direct reply ID when it has no reply-to-top ID.
/// - The General topic (ID 1) also matches messages with no reply header at
///   all, because messages posted in General don't point at a topic root.
pub fn message_in_topic(
    message_id: i32,
    reply_to_top_id: Option<i32>,
    reply_to_msg_id: Option<i32>,
    topic_id: i32,
) -> bool {
    if message_id == topic_id {
        return true;
    }
    match reply_to_top_id.or(reply_to_msg_id) {
        Some(top) => top == topic_id,
        None => topic_id == 1,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn private_link_without_topic() {
        let l = MessageLink::parse("https://t.me/c/123/789").unwrap();
        assert_eq!(l.peer, LinkPeer::PrivateChannel(123));
        assert_eq!(l.topic_id, None);
        assert_eq!(l.message_id, 789);
    }

    #[test]
    fn private_link_with_topic() {
        let l = MessageLink::parse("https://t.me/c/123/456/789").unwrap();
        assert_eq!(l.peer, LinkPeer::PrivateChannel(123));
        assert_eq!(l.topic_id, Some(456));
        assert_eq!(l.message_id, 789);
    }

    #[test]
    fn scheme_host_query_and_fragment_variants() {
        for s in [
            "t.me/c/123/456/789",
            "http://t.me/c/123/456/789",
            "https://telegram.me/c/123/456/789",
            "https://telegram.dog/c/123/456/789",
            "https://t.me/c/123/456/789?single",
            "https://t.me/c/123/456/789#x",
            "  https://t.me/c/123/456/789/ ",
        ] {
            let l = MessageLink::parse(s).unwrap_or_else(|| panic!("{s}"));
            assert_eq!((l.topic_id, l.message_id), (Some(456), 789), "{s}");
        }
    }

    #[test]
    fn public_links_are_unaffected() {
        let l = MessageLink::parse("https://t.me/ferogram/42").unwrap();
        assert_eq!(l.peer, LinkPeer::Username("ferogram".into()));
        assert_eq!((l.topic_id, l.message_id), (None, 42));
    }

    #[test]
    fn rejects_non_message_links() {
        for s in [
            "",
            "https://example.com/c/123/789",
            "https://t.me/ferogram",
            "https://t.me/+AbCdEf/1",
            "https://t.me/c/123",
            "https://t.me/c/abc/789",
            "https://t.me/c/123/0/789",
            "https://t.me/c/123/456/-1",
            "https://t.me/c/123/456/789/1",
            "https://t.me/ferogram/456/789",
        ] {
            assert!(MessageLink::parse(s).is_none(), "{s}");
        }
    }

    #[test]
    fn peer_ref_uses_bot_api_channel_id() {
        let l = MessageLink::parse("https://t.me/c/123/789").unwrap();
        assert!(matches!(l.peer_ref(), PeerRef::Id(-1_000_000_000_123)));
        let l = MessageLink::parse("https://t.me/ferogram/42").unwrap();
        assert!(matches!(l.peer_ref(), PeerRef::Username(u) if u == "ferogram"));
    }

    #[test]
    fn topic_root_is_accepted() {
        assert!(message_in_topic(456, None, None, 456));
    }

    #[test]
    fn message_in_requested_topic_is_accepted() {
        assert!(message_in_topic(789, Some(456), Some(700), 456));
        // No reply-to-top ID: fall back to the direct reply ID.
        assert!(message_in_topic(789, None, Some(456), 456));
    }

    #[test]
    fn message_from_another_topic_is_rejected() {
        assert!(!message_in_topic(789, Some(999), Some(456), 456));
        assert!(!message_in_topic(789, None, Some(999), 456));
        assert!(!message_in_topic(789, None, None, 456));
    }

    #[test]
    fn general_topic_accepts_headerless_messages() {
        assert!(message_in_topic(789, None, None, 1));
        assert!(!message_in_topic(789, Some(456), None, 1));
    }
}
