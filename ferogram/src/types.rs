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

    /// `true` if this bot supports guest-chat mode (`updateBotGuestChatQuery`).
    ///
    /// Bots with this flag can receive guest-chat inline queries and must
    /// answer them with `messages.setBotGuestChatResult`.
    pub fn bot_guestchat(&self) -> bool {
        self.inner().bot_guestchat
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

// UserFull

/// Typed wrapper over `tl::enums::users::UserFull`, the response of
/// [`Client::get_user_full`](crate::Client::get_user_full).
///
/// The bare `userFull` constructor doesn't carry the account's name,
/// username, or online status - those live on the `User` object that
/// Telegram returns alongside it in the same response. This wrapper keeps
/// both together so callers can read status without a follow-up
/// `users.getUsers` round-trip.
#[derive(Debug, Clone)]
pub struct UserFull {
    /// The raw `userFull` constructor, with every field Telegram sent.
    ///
    /// Public so you can move an owned field out directly
    /// (`user.full.about` instead of `user.full().about.clone()`) when you
    /// don't need the rest of the wrapper afterwards.
    pub full: tl::types::UserFull,
    chats: Vec<tl::enums::Chat>,
    users: Vec<tl::enums::User>,
}

impl UserFull {
    /// Wrap the raw response of `get_user_full`.
    pub fn from_raw(raw: tl::enums::users::UserFull) -> Self {
        let tl::enums::users::UserFull::UserFull(inner) = raw;
        let tl::enums::UserFull::UserFull(full) = inner.full_user;
        Self {
            full,
            chats: inner.chats,
            users: inner.users,
        }
    }

    /// Same as the `full` field, as a reference. Use this if you're
    /// borrowing `self` and don't need to move a field out; use `self.full`
    /// directly if you do (e.g. `user.full.about` moves the `String` out
    /// instead of cloning it).
    pub fn full(&self) -> &tl::types::UserFull {
        &self.full
    }

    /// Bio / "about" text.
    pub fn about(&self) -> Option<&str> {
        self.full.about.as_deref()
    }

    /// `true` if the current user has blocked this contact.
    pub fn blocked(&self) -> bool {
        self.full.blocked
    }

    /// Number of chats this user has in common with the current account.
    pub fn common_chats_count(&self) -> i32 {
        self.full.common_chats_count
    }

    /// `true` if voice/video calls are available with this user.
    pub fn phone_calls_available(&self) -> bool {
        self.full.phone_calls_available
    }

    /// `true` if this user's phone calls are set to private.
    pub fn phone_calls_private(&self) -> bool {
        self.full.phone_calls_private
    }

    /// The requested user's `id`.
    pub fn id(&self) -> i64 {
        self.full.id
    }

    /// The `User` object for the requested user - name, username, status,
    /// etc. - taken from this same response, no extra RPC required.
    pub fn user(&self) -> Option<User> {
        let uid = self.full.id;
        self.users.iter().find_map(|u| match u {
            tl::enums::User::User(inner) if inner.id == uid => User::from_raw(u.clone()),
            _ => None,
        })
    }

    /// Shortcut for the requested user's current online/offline status.
    pub fn status(&self) -> Option<tl::enums::UserStatus> {
        self.user().and_then(|u| u.status().cloned())
    }

    /// Chats referenced by this response (e.g. via `personal_channel_id`).
    pub fn chats(&self) -> &[tl::enums::Chat] {
        &self.chats
    }

    /// All users referenced by this response (usually just the requested one).
    pub fn users(&self) -> &[tl::enums::User] {
        &self.users
    }
}

// MessagePage

/// A page of messages returned by [`Client::get_message_history`](crate::Client::get_message_history)
/// or [`Client::get_replies`](crate::Client::get_replies), together with the
/// pagination metadata Telegram sends alongside it.
///
/// The raw `messages.Messages` response comes in four shapes
/// (`Messages`, `Slice`, `ChannelMessages`, `NotModified`), each carrying
/// `count`/`offset_id_offset` differently (or not at all). This flattens
/// that into one shape so you can hand `count`/`offset_id_offset` to a
/// caller (e.g. a UI or bot client) that wants to request the next page,
/// without re-implementing the raw RPC call by hand.
#[derive(Debug, Clone)]
pub struct MessagePage {
    /// The messages in this page.
    pub messages: Vec<crate::update::IncomingMessage>,
    /// Total number of messages in the full result set, when known
    /// (`messages.Slice` / `messages.channelMessages`). `None` for
    /// `messages.Messages`, which means the returned list *is* the whole
    /// result - there's no next page to fetch.
    pub count: Option<i32>,
    /// How many messages lie between the requested `offset_id` and the
    /// start of this page. Add this to the `add_offset` you already used
    /// (plus `limit`) to jump straight to the next page.
    pub offset_id_offset: Option<i32>,
}

impl MessagePage {
    /// `true` if `count` indicates there are more messages beyond this page.
    ///
    /// Only meaningful when `count` is `Some` (a `Slice`/`ChannelMessages`
    /// response); always `false` for a full `Messages` response.
    pub fn has_more(&self) -> bool {
        matches!(self.count, Some(total) if (self.messages.len() as i32) < total)
    }

    /// The `add_offset` to pass to the next call, or `None` if there's no
    /// next page.
    ///
    /// Equivalent to `offset_id_offset.unwrap_or(0) + messages.len()`, kept
    /// as a method so callers don't have to hand-write that math (and get
    /// the `unwrap_or` wrong) at every call site.
    pub fn next_offset(&self) -> Option<i32> {
        if !self.has_more() {
            return None;
        }
        Some(self.offset_id_offset.unwrap_or(0) + self.messages.len() as i32)
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
    /// basic group (i.e. empty, forbidden, a channel, or a community).
    pub fn from_raw(raw: tl::enums::Chat) -> Option<Self> {
        match raw {
            tl::enums::Chat::Chat(c) => Some(Self { raw: c }),
            tl::enums::Chat::Empty(_)
            | tl::enums::Chat::Forbidden(_)
            | tl::enums::Chat::Channel(_)
            | tl::enums::Chat::ChannelForbidden(_)
            | tl::enums::Chat::Community(_)
            | tl::enums::Chat::CommunityForbidden(_) => None,
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
