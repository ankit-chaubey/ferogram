// Copyright (c) Ankit Chaubey <ankitchaubey.dev@gmail.com>
// SPDX-License-Identifier: MIT OR Apache-2.0
//
// ferogram: async Telegram MTProto client in Rust
// https://github.com/ankit-chaubey/ferogram
//
// Based on layer: https://github.com/ankit-chaubey/layer
// Follows official Telegram client behaviour (tdesktop, TDLib).
//
// If you use or modify this code, keep this notice at the top of your file
// and include the LICENSE-MIT or LICENSE-APACHE file from this repository:
// https://github.com/ankit-chaubey/ferogram

//! Typed wrappers over raw TL user and chat types.
//!
//! The raw TL layer has `tl::enums::User` (two variants: `Empty` / `User`) and
//! `tl::enums::Chat` (five variants: `Empty`, `Chat`, `Forbidden`,
//! `Channel`, `ChannelForbidden`).  Working with them directly requires constant
//! pattern-matching.  This module gives three thin wrappers: [`User`],
//! [`Group`], and [`Channel`]: with uniform accessor APIs.

use ferogram_tl_types as tl;

// User

/// Typed wrapper over `tl::enums::User`.
#[derive(Debug, Clone)]
pub struct User {
    pub raw: tl::enums::User,
}

impl User {
    /// Wrap a raw TL user.
    pub fn from_raw(raw: tl::enums::User) -> Option<Self> {
        match &raw {
            tl::enums::User::Empty(_) => None,
            tl::enums::User::User(_) => Some(Self { raw }),
        }
    }

    fn inner(&self) -> &tl::types::User {
        match &self.raw {
            tl::enums::User::User(u) => u,
            tl::enums::User::Empty(_) => unreachable!("User::Empty filtered in from_raw"),
        }
    }

    /// Telegram user ID.
    pub fn id(&self) -> i64 {
        self.inner().id
    }

    /// Access hash needed for API calls.
    pub fn access_hash(&self) -> Option<i64> {
        self.inner().access_hash
    }

    /// First name.
    pub fn first_name(&self) -> Option<&str> {
        self.inner().first_name.as_deref()
    }

    /// Last name.
    pub fn last_name(&self) -> Option<&str> {
        self.inner().last_name.as_deref()
    }

    /// Username (without `@`).
    pub fn username(&self) -> Option<&str> {
        self.inner().username.as_deref()
    }

    /// Phone number, if visible.
    pub fn phone(&self) -> Option<&str> {
        self.inner().phone.as_deref()
    }

    /// `true` if this is a verified account.
    pub fn verified(&self) -> bool {
        self.inner().verified
    }

    /// `true` if this is a bot account.
    pub fn bot(&self) -> bool {
        self.inner().bot
    }

    /// `true` if the account is deleted.
    pub fn deleted(&self) -> bool {
        self.inner().deleted
    }

    /// `true` if the current user has blocked this user.
    pub fn blocked(&self) -> bool {
        false
    }

    /// `true` if this is a premium account.
    pub fn premium(&self) -> bool {
        self.inner().premium
    }

    /// Full display name (`first_name [last_name]`).
    pub fn full_name(&self) -> String {
        match (self.first_name(), self.last_name()) {
            (Some(f), Some(l)) => format!("{f} {l}"),
            (Some(f), None) => f.to_string(),
            (None, Some(l)) => l.to_string(),
            (None, None) => String::new(),
        }
    }

    /// All active usernames (including the primary username).
    pub fn usernames(&self) -> Vec<&str> {
        let mut names = Vec::new();
        // Primary username
        if let Some(u) = self.inner().username.as_deref() {
            names.push(u);
        }
        // Additional usernames
        if let Some(extras) = &self.inner().usernames {
            for u in extras {
                let tl::enums::Username::Username(un) = u;
                if un.active {
                    names.push(un.username.as_str());
                }
            }
        }
        names
    }

    /// The user's current online status.
    pub fn status(&self) -> Option<&tl::enums::UserStatus> {
        self.inner().status.as_ref()
    }

    /// Profile photo, if set.
    pub fn photo(&self) -> Option<&tl::types::UserProfilePhoto> {
        match self.inner().photo.as_ref()? {
            tl::enums::UserProfilePhoto::UserProfilePhoto(p) => Some(p),
            _ => None,
        }
    }

    /// `true` if this is the currently logged-in user.
    pub fn is_self(&self) -> bool {
        self.inner().is_self
    }

    /// `true` if this user is in the logged-in user's contact list.
    pub fn contact(&self) -> bool {
        self.inner().contact
    }

    /// `true` if the logged-in user is also in this user's contact list.
    pub fn mutual_contact(&self) -> bool {
        self.inner().mutual_contact
    }

    /// `true` if this account has been flagged as a scam.
    pub fn scam(&self) -> bool {
        self.inner().scam
    }

    /// `true` if this account has been restricted (e.g. spam-banned).
    pub fn restricted(&self) -> bool {
        self.inner().restricted
    }

    /// `true` if the bot does not display in inline mode publicly.
    pub fn bot_privacy(&self) -> bool {
        self.inner().bot_nochats
    }

    /// `true` if the bot supports being added to groups.
    pub fn bot_supports_chats(&self) -> bool {
        !self.inner().bot_nochats
    }

    /// `true` if the bot can be used inline even without a location share.
    pub fn bot_inline_geo(&self) -> bool {
        self.inner().bot_inline_geo
    }

    /// `true` if this account belongs to Telegram support staff.
    pub fn support(&self) -> bool {
        self.inner().support
    }

    /// Language code reported by the user's client.
    pub fn lang_code(&self) -> Option<&str> {
        self.inner().lang_code.as_deref()
    }

    /// Restriction reasons (why this account is unavailable in certain regions).
    pub fn restriction_reason(&self) -> Vec<&tl::enums::RestrictionReason> {
        self.inner()
            .restriction_reason
            .as_deref()
            .unwrap_or(&[])
            .iter()
            .collect()
    }

    /// Bot inline placeholder text (shown in the compose bar when the user activates inline mode).
    pub fn bot_inline_placeholder(&self) -> Option<&str> {
        self.inner().bot_inline_placeholder.as_deref()
    }

    /// Convert to a `Peer` for use in API calls.
    pub fn as_peer(&self) -> tl::enums::Peer {
        tl::enums::Peer::User(tl::types::PeerUser { user_id: self.id() })
    }

    /// Convert to an `InputPeer` for API calls (requires access hash).
    pub fn as_input_peer(&self) -> tl::enums::InputPeer {
        match self.inner().access_hash {
            Some(ah) => tl::enums::InputPeer::User(tl::types::InputPeerUser {
                user_id: self.id(),
                access_hash: ah,
            }),
            None => tl::enums::InputPeer::PeerSelf,
        }
    }
}

impl std::fmt::Display for User {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let name = self.full_name();
        if let Some(uname) = self.username() {
            write!(f, "{name} (@{uname})")
        } else {
            write!(f, "{name} [{}]", self.id())
        }
    }
}

// Group

/// Typed wrapper over `tl::types::Chat`.
#[derive(Debug, Clone)]
pub struct Group {
    pub raw: tl::types::Chat,
}

impl Group {
    /// Wrap from a raw `tl::enums::Chat`, returning `None` if it is not a
    /// basic group (i.e. empty, forbidden, or a channel).
    pub fn from_raw(raw: tl::enums::Chat) -> Option<Self> {
        match raw {
            tl::enums::Chat::Chat(c) => Some(Self { raw: c }),
            tl::enums::Chat::Empty(_)
            | tl::enums::Chat::Forbidden(_)
            | tl::enums::Chat::Channel(_)
            | tl::enums::Chat::ChannelForbidden(_) => None,
        }
    }

    /// Group ID.
    pub fn id(&self) -> i64 {
        self.raw.id
    }

    /// Group title.
    pub fn title(&self) -> &str {
        &self.raw.title
    }

    /// Member count.
    pub fn participants_count(&self) -> i32 {
        self.raw.participants_count
    }

    /// `true` if the logged-in user is the creator.
    pub fn creator(&self) -> bool {
        self.raw.creator
    }

    /// `true` if the group has been migrated to a supergroup.
    pub fn migrated_to(&self) -> Option<&tl::enums::InputChannel> {
        self.raw.migrated_to.as_ref()
    }

    /// Convert to a `Peer`.
    pub fn as_peer(&self) -> tl::enums::Peer {
        tl::enums::Peer::Chat(tl::types::PeerChat { chat_id: self.id() })
    }

    /// Convert to an `InputPeer`.
    pub fn as_input_peer(&self) -> tl::enums::InputPeer {
        tl::enums::InputPeer::Chat(tl::types::InputPeerChat { chat_id: self.id() })
    }
}

impl std::fmt::Display for Group {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} [group {}]", self.title(), self.id())
    }
}

// Channel

/// The kind of a channel or supergroup.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChannelKind {
    /// A broadcast channel (posts only, no member replies by default).
    Broadcast,
    /// A supergroup (all members can post).
    Megagroup,
    /// A gigagroup / broadcast group (large public broadcast supergroup).
    Gigagroup,
}

/// Typed wrapper over `tl::types::Channel`.
#[derive(Debug, Clone)]
pub struct Channel {
    pub raw: tl::types::Channel,
}

impl Channel {
    /// Wrap from a raw `tl::enums::Chat`, returning `None` if it is not a channel.
    pub fn from_raw(raw: tl::enums::Chat) -> Option<Self> {
        match raw {
            tl::enums::Chat::Channel(c) => Some(Self { raw: c }),
            _ => None,
        }
    }

    /// Channel ID.
    pub fn id(&self) -> i64 {
        self.raw.id
    }

    /// Access hash.
    pub fn access_hash(&self) -> Option<i64> {
        self.raw.access_hash
    }

    /// Channel / supergroup title.
    pub fn title(&self) -> &str {
        &self.raw.title
    }

    /// Username (without `@`), if public.
    pub fn username(&self) -> Option<&str> {
        self.raw.username.as_deref()
    }

    /// `true` if this is a supergroup (not a broadcast channel).
    pub fn megagroup(&self) -> bool {
        self.raw.megagroup
    }

    /// `true` if this is a broadcast channel.
    pub fn broadcast(&self) -> bool {
        self.raw.broadcast
    }

    /// `true` if this is a verified channel.
    pub fn verified(&self) -> bool {
        self.raw.verified
    }

    /// `true` if the channel is restricted.
    pub fn restricted(&self) -> bool {
        self.raw.restricted
    }

    /// `true` if the channel has signatures on posts.
    pub fn signatures(&self) -> bool {
        self.raw.signatures
    }

    /// Approximate member count (may be `None` for private channels).
    pub fn participants_count(&self) -> Option<i32> {
        self.raw.participants_count
    }

    /// The kind of this channel.
    ///
    /// Returns `ChannelKind::Megagroup` for supergroups, `ChannelKind::Broadcast` for
    /// broadcast channels, and `ChannelKind::Gigagroup` for large broadcast groups.
    pub fn kind(&self) -> ChannelKind {
        if self.raw.megagroup {
            ChannelKind::Megagroup
        } else if self.raw.gigagroup {
            ChannelKind::Gigagroup
        } else {
            ChannelKind::Broadcast
        }
    }

    /// All active usernames (including the primary username).
    pub fn usernames(&self) -> Vec<&str> {
        let mut names = Vec::new();
        if let Some(u) = self.raw.username.as_deref() {
            names.push(u);
        }
        if let Some(extras) = &self.raw.usernames {
            for u in extras {
                let tl::enums::Username::Username(un) = u;
                if un.active {
                    names.push(un.username.as_str());
                }
            }
        }
        names
    }

    /// Profile photo, if set.
    pub fn photo(&self) -> Option<&tl::types::ChatPhoto> {
        match &self.raw.photo {
            tl::enums::ChatPhoto::ChatPhoto(p) => Some(p),
            _ => None,
        }
    }

    /// Admin rights granted to the logged-in user in this channel, if any.
    pub fn admin_rights(&self) -> Option<&tl::types::ChatAdminRights> {
        match self.raw.admin_rights.as_ref()? {
            tl::enums::ChatAdminRights::ChatAdminRights(r) => Some(r),
        }
    }

    /// Restriction reasons (why this channel is unavailable in certain regions).
    pub fn restriction_reason(&self) -> Vec<&tl::enums::RestrictionReason> {
        self.raw
            .restriction_reason
            .as_deref()
            .unwrap_or(&[])
            .iter()
            .collect()
    }

    /// Convert to a `Peer`.
    pub fn as_peer(&self) -> tl::enums::Peer {
        tl::enums::Peer::Channel(tl::types::PeerChannel {
            channel_id: self.id(),
        })
    }

    /// Convert to an `InputPeer` (requires access hash).
    pub fn as_input_peer(&self) -> tl::enums::InputPeer {
        match self.raw.access_hash {
            Some(ah) => tl::enums::InputPeer::Channel(tl::types::InputPeerChannel {
                channel_id: self.id(),
                access_hash: ah,
            }),
            None => tl::enums::InputPeer::Empty,
        }
    }

    /// Convert to an `InputChannel` for channel-specific RPCs.
    pub fn as_input_channel(&self) -> tl::enums::InputChannel {
        match self.raw.access_hash {
            Some(ah) => tl::enums::InputChannel::InputChannel(tl::types::InputChannel {
                channel_id: self.id(),
                access_hash: ah,
            }),
            None => tl::enums::InputChannel::Empty,
        }
    }
}

impl std::fmt::Display for Channel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let Some(uname) = self.username() {
            write!(f, "{} (@{uname})", self.title())
        } else {
            write!(f, "{} [channel {}]", self.title(), self.id())
        }
    }
}

// Chat enum (unified)

/// A unified chat type: either a basic [`Group`] or a [`Channel`]/supergroup.
#[derive(Debug, Clone)]
pub enum Chat {
    Group(Group),
    Channel(Box<Channel>),
}

impl Chat {
    /// Attempt to construct from a raw `tl::enums::Chat`.
    pub fn from_raw(raw: tl::enums::Chat) -> Option<Self> {
        match &raw {
            tl::enums::Chat::Chat(_) => Group::from_raw(raw).map(Chat::Group),
            tl::enums::Chat::Channel(_) => {
                Channel::from_raw(raw).map(|c| Chat::Channel(Box::new(c)))
            }
            _ => None,
        }
    }

    /// Common ID regardless of variant.
    pub fn id(&self) -> i64 {
        match self {
            Chat::Group(g) => g.id(),
            Chat::Channel(c) => c.id(),
        }
    }

    /// Common title regardless of variant.
    pub fn title(&self) -> &str {
        match self {
            Chat::Group(g) => g.title(),
            Chat::Channel(c) => c.title(),
        }
    }

    /// Convert to a `Peer`.
    pub fn as_peer(&self) -> tl::enums::Peer {
        match self {
            Chat::Group(g) => g.as_peer(),
            Chat::Channel(c) => c.as_peer(),
        }
    }

    /// Convert to an `InputPeer`.
    pub fn as_input_peer(&self) -> tl::enums::InputPeer {
        match self {
            Chat::Group(g) => g.as_input_peer(),
            Chat::Channel(c) => c.as_input_peer(),
        }
    }
}
