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

//! High-level update types delivered by [`crate::Client::stream_updates`].
//!
//! Every update the Telegram server pushes is classified into one of the
//! variants of [`Update`].  The raw constructor ID is always available
//! via [`Update::Raw`] for anything not yet wrapped.

use ferogram_tl_types as tl;
use ferogram_tl_types::{Cursor, Deserializable};

use crate::{Client, InvocationError as Error};

/// A new or edited message.
#[derive(Clone)]
pub struct IncomingMessage {
    /// The underlying TL message object.
    pub raw: tl::enums::Message,
    /// An embedded client reference, populated for messages received via
    /// `stream_updates()` and returned from send/search/history APIs.
    /// When present, the clientless action methods (`reply`, `respond`,
    /// `edit`, `delete`, `pin`, `unpin`, `react`, ...) can be called without
    /// passing a `&Client` argument.
    pub client: Option<Client>,
}

impl std::fmt::Debug for IncomingMessage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("IncomingMessage")
            .field("raw", &self.raw)
            .field("has_client", &self.client.is_some())
            .finish()
    }
}

impl IncomingMessage {
    pub fn from_raw(raw: tl::enums::Message) -> Self {
        Self { raw, client: None }
    }

    /// Attach a `Client` so the clientless action methods work.
    ///
    /// Returns `self` for chaining:
    /// ```rust,no_run
    /// # use ferogram::update::IncomingMessage;
    /// # fn ex(raw: ferogram_tl_types::enums::Message, client: ferogram::Client) {
    /// let msg = IncomingMessage::from_raw(raw).with_client(client);
    /// # }
    /// ```
    pub fn with_client(mut self, client: Client) -> Self {
        self.client = Some(client);
        self
    }

    /// Returns an error when no client is embedded.
    fn require_client(&self, method: &str) -> Result<&Client, Error> {
        self.client.as_ref().ok_or_else(|| {
            Error::Deserialize(format!(
                "{method}: this IncomingMessage has no embedded client: \
                 use the `_with` variant and pass a &Client explicitly"
            ))
        })
    }

    /// The message text (or caption for media messages).
    pub fn text(&self) -> Option<&str> {
        match &self.raw {
            tl::enums::Message::Message(m) => {
                if m.message.is_empty() {
                    None
                } else {
                    Some(&m.message)
                }
            }
            _ => None,
        }
    }

    /// Unique message ID within the chat.
    pub fn id(&self) -> i32 {
        match &self.raw {
            tl::enums::Message::Message(m) => m.id,
            tl::enums::Message::Service(m) => m.id,
            tl::enums::Message::Empty(m) => m.id,
        }
    }

    /// The peer (chat) this message belongs to.
    pub fn peer_id(&self) -> Option<&tl::enums::Peer> {
        match &self.raw {
            tl::enums::Message::Message(m) => Some(&m.peer_id),
            tl::enums::Message::Service(m) => Some(&m.peer_id),
            _ => None,
        }
    }

    /// The sender peer, if available (not set for anonymous channel posts).
    pub fn sender_id(&self) -> Option<&tl::enums::Peer> {
        match &self.raw {
            tl::enums::Message::Message(m) => m.from_id.as_ref(),
            tl::enums::Message::Service(m) => m.from_id.as_ref(),
            _ => None,
        }
    }

    /// `true` if the message was sent by the logged-in account.
    pub fn outgoing(&self) -> bool {
        match &self.raw {
            tl::enums::Message::Message(m) => m.out,
            tl::enums::Message::Service(m) => m.out,
            _ => false,
        }
    }

    /// Unix timestamp when the message was sent.
    pub fn date(&self) -> i32 {
        match &self.raw {
            tl::enums::Message::Message(m) => m.date,
            tl::enums::Message::Service(m) => m.date,
            _ => 0,
        }
    }

    /// Unix timestamp of the last edit, if the message has been edited.
    pub fn edit_date(&self) -> Option<i32> {
        match &self.raw {
            tl::enums::Message::Message(m) => m.edit_date,
            _ => None,
        }
    }

    /// `true` if the logged-in user was mentioned in this message.
    pub fn mentioned(&self) -> bool {
        match &self.raw {
            tl::enums::Message::Message(m) => m.mentioned,
            tl::enums::Message::Service(m) => m.mentioned,
            _ => false,
        }
    }

    /// `true` if the message was sent silently (no notification).
    pub fn silent(&self) -> bool {
        match &self.raw {
            tl::enums::Message::Message(m) => m.silent,
            tl::enums::Message::Service(m) => m.silent,
            _ => false,
        }
    }

    /// `true` if this is a channel post (no sender).
    pub fn post(&self) -> bool {
        match &self.raw {
            tl::enums::Message::Message(m) => m.post,
            _ => false,
        }
    }

    /// `true` if this message is currently pinned.
    pub fn pinned(&self) -> bool {
        match &self.raw {
            tl::enums::Message::Message(m) => m.pinned,
            _ => false,
        }
    }

    /// Number of times the message has been forwarded (channels only).
    pub fn forward_count(&self) -> Option<i32> {
        match &self.raw {
            tl::enums::Message::Message(m) => m.forwards,
            _ => None,
        }
    }

    /// View count for channel posts.
    pub fn view_count(&self) -> Option<i32> {
        match &self.raw {
            tl::enums::Message::Message(m) => m.views,
            _ => None,
        }
    }

    /// Reply count (number of replies in a thread).
    pub fn reply_count(&self) -> Option<i32> {
        match &self.raw {
            tl::enums::Message::Message(m) => m.replies.as_ref().map(|r| match r {
                tl::enums::MessageReplies::MessageReplies(x) => x.replies,
            }),
            _ => None,
        }
    }

    /// ID of the message this one is replying to.
    pub fn reply_to_message_id(&self) -> Option<i32> {
        match &self.raw {
            tl::enums::Message::Message(m) => m.reply_to.as_ref().and_then(|r| match r {
                tl::enums::MessageReplyHeader::MessageReplyHeader(h) => h.reply_to_msg_id,
                _ => None,
            }),
            _ => None,
        }
    }

    /// Fetch the message that this one is replying to.
    ///
    /// Returns `None` if this message is not a reply or if the peer is unknown.
    /// Unlike [`reply_to_message_id`] this actually performs an API call to
    /// retrieve the full message object.
    ///
    /// [`reply_to_message_id`]: IncomingMessage::reply_to_message_id
    pub async fn reply_to_message(
        &self,
        client: &Client,
    ) -> Result<Option<IncomingMessage>, Error> {
        let reply_id = match self.reply_to_message_id() {
            Some(id) => id,
            None => return Ok(None),
        };
        let peer = match self.peer_id() {
            Some(p) => p.clone(),
            None => return Ok(None),
        };
        let msgs = client.get_messages_by_id(peer, &[reply_id]).await?;
        Ok(msgs.into_iter().next())
    }

    /// The message's send time as a [`chrono::DateTime<chrono::Utc>`].
    ///
    /// Typed wrapper around the raw `date()` Unix timestamp.
    pub fn date_utc(&self) -> Option<chrono::DateTime<chrono::Utc>> {
        use chrono::TimeZone;
        let ts = self.date();
        if ts == 0 {
            return None;
        }
        chrono::Utc.timestamp_opt(ts as i64, 0).single()
    }

    /// The last edit time as a [`chrono::DateTime<chrono::Utc>`], if edited.
    pub fn edit_date_utc(&self) -> Option<chrono::DateTime<chrono::Utc>> {
        use chrono::TimeZone;
        self.edit_date()
            .and_then(|ts| chrono::Utc.timestamp_opt(ts as i64, 0).single())
    }

    /// The media attached to this message, if any.
    pub fn media(&self) -> Option<&tl::enums::MessageMedia> {
        match &self.raw {
            tl::enums::Message::Message(m) => m.media.as_ref(),
            _ => None,
        }
    }

    /// Formatting entities (bold, italic, code, links, etc).
    pub fn entities(&self) -> Option<&Vec<tl::enums::MessageEntity>> {
        match &self.raw {
            tl::enums::Message::Message(m) => m.entities.as_ref(),
            _ => None,
        }
    }

    /// Group ID for album messages (multiple media in one).
    pub fn grouped_id(&self) -> Option<i64> {
        match &self.raw {
            tl::enums::Message::Message(m) => m.grouped_id,
            _ => None,
        }
    }

    /// `true` if this message was sent from a scheduled one.
    pub fn from_scheduled(&self) -> bool {
        match &self.raw {
            tl::enums::Message::Message(m) => m.from_scheduled,
            _ => false,
        }
    }

    /// `true` if the edit date is hidden from recipients.
    pub fn edit_hide(&self) -> bool {
        match &self.raw {
            tl::enums::Message::Message(m) => m.edit_hide,
            _ => false,
        }
    }

    /// `true` if the media in this message has not been read yet.
    pub fn media_unread(&self) -> bool {
        match &self.raw {
            tl::enums::Message::Message(m) => m.media_unread,
            tl::enums::Message::Service(m) => m.media_unread,
            _ => false,
        }
    }

    /// ID of the bot that sent this message via inline mode, if any.
    pub fn via_bot_id(&self) -> Option<i64> {
        match &self.raw {
            tl::enums::Message::Message(m) => m.via_bot_id,
            _ => None,
        }
    }

    /// Signature of the post author in a channel, if set.
    pub fn post_author(&self) -> Option<&str> {
        match &self.raw {
            tl::enums::Message::Message(m) => m.post_author.as_deref(),
            _ => None,
        }
    }

    /// Number of reactions on this message, if any.
    pub fn reaction_count(&self) -> i32 {
        match &self.raw {
            tl::enums::Message::Message(m) => m
                .reactions
                .as_ref()
                .map(|r| match r {
                    tl::enums::MessageReactions::MessageReactions(x) => x
                        .results
                        .iter()
                        .map(|res| match res {
                            tl::enums::ReactionCount::ReactionCount(c) => c.count,
                        })
                        .sum(),
                })
                .unwrap_or(0),
            _ => 0,
        }
    }

    /// Restriction reasons (why this message is unavailable in some regions).
    pub fn restriction_reason(&self) -> Option<&Vec<tl::enums::RestrictionReason>> {
        match &self.raw {
            tl::enums::Message::Message(m) => m.restriction_reason.as_ref(),
            _ => None,
        }
    }

    /// Reply markup (inline keyboards, etc).
    pub fn reply_markup(&self) -> Option<&tl::enums::ReplyMarkup> {
        match &self.raw {
            tl::enums::Message::Message(m) => m.reply_markup.as_ref(),
            _ => None,
        }
    }

    /// Forward info header, if this message was forwarded.
    pub fn forward_header(&self) -> Option<&tl::enums::MessageFwdHeader> {
        match &self.raw {
            tl::enums::Message::Message(m) => m.fwd_from.as_ref(),
            _ => None,
        }
    }

    /// `true` if forwarding this message is restricted.
    pub fn noforwards(&self) -> bool {
        match &self.raw {
            tl::enums::Message::Message(m) => m.noforwards,
            _ => false,
        }
    }

    /// Reconstruct Markdown from the message text and its formatting entities.
    ///
    /// Returns plain text if there are no entities.
    pub fn markdown_text(&self) -> Option<String> {
        let text = self.text()?;
        let entities = self.entities().map(|e| e.as_slice()).unwrap_or(&[]);
        Some(crate::parsers::generate_markdown(text, entities))
    }

    /// Reconstruct HTML from the message text and its formatting entities.
    ///
    /// Returns plain text if there are no entities.
    pub fn html_text(&self) -> Option<String> {
        let text = self.text()?;
        let entities = self.entities().map(|e| e.as_slice()).unwrap_or(&[]);
        Some(crate::parsers::generate_html(text, entities))
    }

    /// Service message action (e.g. "user joined", "call started").\
    /// Returns `None` for regular text/media messages.
    pub fn action(&self) -> Option<&tl::enums::MessageAction> {
        match &self.raw {
            tl::enums::Message::Service(m) => Some(&m.action),
            _ => None,
        }
    }

    /// Extract a `Photo` from the message media, if present.
    ///
    /// Shorthand for `Photo::from_media(msg.media()?)`.
    pub fn photo(&self) -> Option<crate::media::Photo> {
        crate::media::Photo::from_media(self.media()?)
    }

    /// Extract a `Document` from the message media, if present.
    ///
    /// Shorthand for `Document::from_media(msg.media()?)`.
    pub fn document(&self) -> Option<crate::media::Document> {
        crate::media::Document::from_media(self.media()?)
    }

    /// The bare numeric chat ID (positive for users/groups, negative for channels).
    pub fn chat_id(&self) -> i64 {
        match self.peer_id() {
            Some(tl::enums::Peer::User(u)) => u.user_id,
            Some(tl::enums::Peer::Chat(c)) => c.chat_id,
            Some(tl::enums::Peer::Channel(c)) => c.channel_id,
            None => 0,
        }
    }

    /// `true` when the message is in a private (1-on-1) chat.
    pub fn is_private(&self) -> bool {
        matches!(self.peer_id(), Some(tl::enums::Peer::User(_)))
    }

    /// `true` when the message is in a basic group.
    pub fn is_group(&self) -> bool {
        matches!(self.peer_id(), Some(tl::enums::Peer::Chat(_)))
    }

    /// `true` when the message is in a channel or supergroup.
    pub fn is_channel(&self) -> bool {
        matches!(self.peer_id(), Some(tl::enums::Peer::Channel(_)))
    }

    /// `true` when the message is in *any* multi-user chat (group or channel).
    pub fn is_any_group(&self) -> bool {
        self.is_group() || self.is_channel()
    }

    /// The bare sender user-ID (`from_id` as an i64). Alias of `sender_user_id`.
    pub fn from_id(&self) -> Option<i64> {
        self.sender_user_id()
    }

    /// `true` when the message text begins with `/` (a bot command).
    pub fn is_bot_command(&self) -> bool {
        self.text().is_some_and(|t| t.starts_with('/'))
    }

    /// If the message is a bot command, returns `(command, rest)` where `command`
    /// is the command name without the `/` and optional `@BotName` suffix,
    /// and `rest` is everything after (trimmed).
    ///
    /// ```rust,no_run
    /// # fn ex(msg: ferogram::update::IncomingMessage) {
    /// if let Some((cmd, args)) = msg.command() {
    ///     // cmd = "start", args = "payload"
    /// }
    /// # }
    /// ```
    pub fn command(&self) -> Option<(&str, &str)> {
        let text = self.text()?;
        if !text.starts_with('/') {
            return None;
        }
        let without_slash = &text[1..];
        // Split off @BotName if present
        let cmd_full = without_slash.split_whitespace().next().unwrap_or("");
        let cmd = cmd_full.split('@').next().unwrap_or(cmd_full);
        let rest = text[1 + cmd_full.len()..].trim();
        Some((cmd, rest))
    }

    /// `true` if the message is the named command (case-insensitive, ignoring `@bot` suffix).
    pub fn is_command_named(&self, name: &str) -> bool {
        self.command()
            .is_some_and(|(cmd, _)| cmd.eq_ignore_ascii_case(name))
    }

    /// Return the arguments portion of a bot command (text after `/cmd`), trimmed.
    /// Returns `None` if the message is not a command.
    pub fn command_args(&self) -> Option<&str> {
        self.command().map(|(_, args)| args)
    }

    /// `true` if the message carries any media attachment.
    pub fn has_media(&self) -> bool {
        self.media().is_some()
    }

    /// `true` if the message carries a photo.
    pub fn has_photo(&self) -> bool {
        matches!(self.media(), Some(tl::enums::MessageMedia::Photo(_)))
    }

    /// `true` if the message carries a document (file, video, audio, etc).
    pub fn has_document(&self) -> bool {
        matches!(self.media(), Some(tl::enums::MessageMedia::Document(_)))
    }

    /// `true` if this message was forwarded from another chat or user.
    pub fn is_forwarded(&self) -> bool {
        self.forward_header().is_some()
    }

    /// `true` if this message is a reply to another message.
    pub fn is_reply(&self) -> bool {
        self.reply_to_message_id().is_some()
    }

    /// Alias of `grouped_id` - the album/grouped-media ID, if this message is
    /// part of an album.
    pub fn album_id(&self) -> Option<i64> {
        self.grouped_id()
    }

    // reply

    /// Reply to this message (clientless: requires an embedded client).
    ///
    /// Returns the sent message so you can chain further operations on it.
    pub async fn reply(&self, text: impl Into<String>) -> Result<IncomingMessage, Error> {
        let client = self.require_client("reply")?.clone();
        self.reply_with(&client, text).await
    }

    /// Reply to this message with a plain string.
    ///
    /// Returns the sent message so you can chain further operations on it.
    pub async fn reply_with(
        &self,
        client: &Client,
        text: impl Into<String>,
    ) -> Result<IncomingMessage, Error> {
        let peer = match self.peer_id() {
            Some(p) => p.clone(),
            None => return Err(Error::Deserialize("cannot reply: unknown peer".into())),
        };
        let msg_id = self.id();
        client
            .send_message_to_peer_ex(
                peer,
                &crate::InputMessage::text(text.into()).reply_to(Some(msg_id)),
            )
            .await
    }

    /// Reply with a full [`InputMessage`](crate::InputMessage) (clientless).
    pub async fn reply_ex(&self, msg: crate::InputMessage) -> Result<IncomingMessage, Error> {
        let client = self.require_client("reply_ex")?.clone();
        self.reply_ex_with(&client, msg).await
    }

    /// Reply with a full [`InputMessage`](crate::InputMessage).
    pub async fn reply_ex_with(
        &self,
        client: &Client,
        msg: crate::InputMessage,
    ) -> Result<IncomingMessage, Error> {
        let peer = self
            .peer_id()
            .cloned()
            .ok_or_else(|| Error::Deserialize("cannot reply_ex: unknown peer".into()))?;
        client
            .send_message_to_peer_ex(peer, &msg.reply_to(Some(self.id())))
            .await
    }

    // respond

    /// Send to the same chat without quoting (clientless).
    pub async fn respond(&self, text: impl Into<String>) -> Result<IncomingMessage, Error> {
        let client = self.require_client("respond")?.clone();
        self.respond_with(&client, text).await
    }

    /// Send to the same chat without quoting.
    pub async fn respond_with(
        &self,
        client: &Client,
        text: impl Into<String>,
    ) -> Result<IncomingMessage, Error> {
        let peer = self
            .peer_id()
            .cloned()
            .ok_or_else(|| Error::Deserialize("cannot respond: unknown peer".into()))?;
        client
            .send_message_to_peer_ex(peer, &crate::InputMessage::text(text.into()))
            .await
    }

    /// Full [`InputMessage`] to the same chat without quoting (clientless).
    pub async fn respond_ex(&self, msg: crate::InputMessage) -> Result<IncomingMessage, Error> {
        let client = self.require_client("respond_ex")?.clone();
        self.respond_ex_with(&client, msg).await
    }

    /// Full [`InputMessage`] to the same chat without quoting.
    pub async fn respond_ex_with(
        &self,
        client: &Client,
        msg: crate::InputMessage,
    ) -> Result<IncomingMessage, Error> {
        let peer = self
            .peer_id()
            .cloned()
            .ok_or_else(|| Error::Deserialize("cannot respond_ex: unknown peer".into()))?;
        client.send_message_to_peer_ex(peer, &msg).await
    }

    // edit

    /// Edit this message (clientless).
    pub async fn edit(&self, new_text: impl Into<String>) -> Result<(), Error> {
        let client = self.require_client("edit")?.clone();
        self.edit_with(&client, new_text).await
    }

    /// Edit this message.
    pub async fn edit_with(
        &self,
        client: &Client,
        new_text: impl Into<String>,
    ) -> Result<(), Error> {
        let peer = self
            .peer_id()
            .cloned()
            .ok_or_else(|| Error::Deserialize("cannot edit: unknown peer".into()))?;
        client
            .edit_message(peer, self.id(), new_text.into().as_str())
            .await
    }

    // delete

    /// Delete this message (clientless).
    pub async fn delete(&self) -> Result<(), Error> {
        let client = self.require_client("delete")?.clone();
        self.delete_with(&client).await
    }

    /// Delete this message.
    pub async fn delete_with(&self, client: &Client) -> Result<(), Error> {
        client.delete_messages(vec![self.id()], true).await
    }

    // mark_as_read

    /// Mark this message (and all before it) as read (clientless).
    pub async fn mark_as_read(&self) -> Result<(), Error> {
        let client = self.require_client("mark_as_read")?.clone();
        self.mark_as_read_with(&client).await
    }

    /// Mark this message (and all before it) as read.
    pub async fn mark_as_read_with(&self, client: &Client) -> Result<(), Error> {
        let peer = self
            .peer_id()
            .cloned()
            .ok_or_else(|| Error::Deserialize("cannot mark_as_read: unknown peer".into()))?;
        client.mark_as_read(peer).await
    }

    // pin

    /// Pin this message silently (clientless).
    pub async fn pin(&self) -> Result<(), Error> {
        let client = self.require_client("pin")?.clone();
        self.pin_with(&client).await
    }

    /// Pin this message silently.
    pub async fn pin_with(&self, client: &Client) -> Result<(), Error> {
        let peer = self
            .peer_id()
            .cloned()
            .ok_or_else(|| Error::Deserialize("cannot pin: unknown peer".into()))?;
        client
            .pin_message(peer, self.id(), true, false, false)
            .await
    }

    // unpin

    /// Unpin this message (clientless).
    pub async fn unpin(&self) -> Result<(), Error> {
        let client = self.require_client("unpin")?.clone();
        self.unpin_with(&client).await
    }

    /// Unpin this message.
    pub async fn unpin_with(&self, client: &Client) -> Result<(), Error> {
        let peer = self
            .peer_id()
            .cloned()
            .ok_or_else(|| Error::Deserialize("cannot unpin: unknown peer".into()))?;
        client.unpin_message(peer, self.id()).await
    }

    // forward_to

    /// Forward this message to another chat (clientless).
    ///
    /// Returns the forwarded message in the destination chat.
    pub async fn forward_to(
        &self,
        destination: impl Into<crate::PeerRef>,
    ) -> Result<IncomingMessage, Error> {
        let client = self.require_client("forward_to")?.clone();
        self.forward_to_with(&client, destination).await
    }

    /// Forward this message to another chat.
    ///
    /// Returns the forwarded message in the destination chat.
    pub async fn forward_to_with(
        &self,
        client: &Client,
        destination: impl Into<crate::PeerRef>,
    ) -> Result<IncomingMessage, Error> {
        let src = self
            .peer_id()
            .cloned()
            .ok_or_else(|| Error::Deserialize("cannot forward: unknown source peer".into()))?;
        client
            .forward_messages_returning(destination, &[self.id()], src)
            .await
            .and_then(|v| {
                v.into_iter()
                    .next()
                    .ok_or_else(|| Error::Deserialize("forward returned no message".into()))
            })
    }

    // refetch

    /// Re-fetch this message from Telegram (clientless).
    ///
    /// Useful to get updated view/forward counts, reactions, edit state, etc.
    /// Updates `self` in place; returns an error if the message was deleted.
    pub async fn refetch(&mut self) -> Result<(), Error> {
        let client = self.require_client("refetch")?.clone();
        self.refetch_with(&client).await
    }

    /// Re-fetch this message from Telegram.
    pub async fn refetch_with(&mut self, client: &Client) -> Result<(), Error> {
        let peer = self
            .peer_id()
            .cloned()
            .ok_or_else(|| Error::Deserialize("cannot refetch: unknown peer".into()))?;
        let mut msgs = client.get_messages_by_id(peer, &[self.id()]).await?;
        match msgs.pop() {
            Some(m) => {
                self.raw = m.raw;
                Ok(())
            }
            None => Err(Error::Deserialize(
                "refetch: message not found (deleted?)".into(),
            )),
        }
    }

    // download_media

    /// Download attached media to `path` (clientless).
    pub async fn download_media(&self, path: impl AsRef<std::path::Path>) -> Result<bool, Error> {
        let client = self.require_client("download_media")?.clone();
        self.download_media_with(&client, path).await
    }

    /// Download attached media to `path`. Returns `true` if media was found.
    pub async fn download_media_with(
        &self,
        client: &Client,
        path: impl AsRef<std::path::Path>,
    ) -> Result<bool, Error> {
        if let Some((loc, dc_id)) = crate::media::download_location_from_media(self.media()) {
            client
                .download_media_to_file_on_dc(loc, dc_id, path)
                .await?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    // react

    /// Send a reaction (clientless).
    ///
    /// # Example
    /// ```rust,no_run
    /// # async fn f(msg: ferogram::update::IncomingMessage)
    /// #   -> Result<(), ferogram::InvocationError> {
    /// use ferogram::reactions::InputReactions;
    /// msg.react(InputReactions::emoticon("👍")).await?;
    /// # Ok(()) }
    /// ```
    pub async fn react(
        &self,
        reactions: impl Into<crate::reactions::InputReactions>,
    ) -> Result<(), Error> {
        let client = self.require_client("react")?.clone();
        self.react_with(&client, reactions).await
    }

    /// Send a reaction.
    pub async fn react_with(
        &self,
        client: &Client,
        reactions: impl Into<crate::reactions::InputReactions>,
    ) -> Result<(), Error> {
        let peer = self
            .peer_id()
            .cloned()
            .ok_or_else(|| Error::Deserialize("cannot react: unknown peer".into()))?;
        client.send_reaction(peer, self.id(), reactions).await
    }

    // get_reply

    /// Fetch the message this is a reply to (clientless).
    pub async fn get_reply(&self) -> Result<Option<IncomingMessage>, Error> {
        let client = self.require_client("get_reply")?.clone();
        self.get_reply_with(&client).await
    }

    /// Fetch the message this is a reply to.
    pub async fn get_reply_with(&self, client: &Client) -> Result<Option<IncomingMessage>, Error> {
        client.get_reply_to_message(self).await
    }

    // sender helpers

    /// The sender's bare user-ID, if this is a user message.
    ///
    /// Returns `None` for anonymous channel posts.
    pub fn sender_user_id(&self) -> Option<i64> {
        match self.sender_id()? {
            tl::enums::Peer::User(u) => Some(u.user_id),
            _ => None,
        }
    }

    /// The chat/channel-ID the sender belongs to (non-user senders).
    pub fn sender_chat_id(&self) -> Option<i64> {
        match self.sender_id()? {
            tl::enums::Peer::Chat(c) => Some(c.chat_id),
            tl::enums::Peer::Channel(c) => Some(c.channel_id),
            _ => None,
        }
    }

    /// Fetch the sender as a typed [`User`](crate::types::User) (clientless, async).
    ///
    /// Returns `None` if the sender is not a user, or if the user is not in
    /// the local peer cache.  Performs a network call if needed.
    pub async fn sender_user(&self) -> Result<Option<crate::types::User>, Error> {
        let uid = match self.sender_user_id() {
            Some(id) => id,
            None => return Ok(None),
        };
        let client = self.require_client("sender_user")?.clone();
        let users = client.get_users_by_id(&[uid]).await?;
        Ok(users.into_iter().next().flatten())
    }
}

// MessageDeletion

/// One or more messages were deleted.
#[derive(Debug, Clone)]
pub struct MessageDeletion {
    /// IDs of the deleted messages.
    pub message_ids: Vec<i32>,
    /// Channel ID, if the deletion happened in a channel / supergroup.
    pub channel_id: Option<i64>,
}

impl MessageDeletion {
    /// Consume self and return the deleted message IDs without cloning.
    pub fn into_messages(self) -> Vec<i32> {
        self.message_ids
    }
}

// CallbackQuery

/// A user pressed an inline keyboard button on a bot message.
#[derive(Debug, Clone)]
pub struct CallbackQuery {
    pub query_id: i64,
    pub user_id: i64,
    pub message_id: Option<i32>,
    pub chat_instance: i64,
    /// Raw `data` bytes from the button.
    pub data_raw: Option<Vec<u8>>,
    /// Game short name (if a game button was pressed).
    pub game_short_name: Option<String>,
    /// The peer (chat/channel/user) where the button was pressed.
    /// `None` for inline-message callback queries.
    pub chat_peer: Option<tl::enums::Peer>,
    /// For inline-message callbacks: the message ID token.
    pub inline_msg_id: Option<tl::enums::InputBotInlineMessageId>,
}

impl CallbackQuery {
    /// Button data as a UTF-8 string, if valid.
    pub fn data(&self) -> Option<&str> {
        self.data_raw
            .as_ref()
            .and_then(|d| std::str::from_utf8(d).ok())
    }

    /// Begin building an answer for this callback query.
    ///
    /// Finish with `.send(&client).await`:
    ///
    /// ```rust,no_run
    /// query.answer().text("Done!").send(&client).await?;
    /// query.answer().alert("No permission!").send(&client).await?;
    /// query.answer().url("https://example.com/game").send(&client).await?;
    /// query.answer()
    /// .text("Cached")
    /// .cache_time(std::time::Duration::from_secs(60))
    /// .send(&client).await?;
    /// ```
    pub fn answer(&self) -> Answer<'_> {
        Answer {
            query_id: self.query_id,
            message: None,
            alert: false,
            url: None,
            cache_time: 0,
            _marker: std::marker::PhantomData,
        }
    }

    /// Answer the callback query (flat helper: prefer `answer()` builder).
    pub async fn answer_flat(&self, client: &Client, text: Option<&str>) -> Result<(), Error> {
        client
            .answer_callback_query(self.query_id, text, false)
            .await
            .map(|_| ())
    }

    /// Answer with a popup alert (flat helper: prefer `answer().alert(...)`).
    pub async fn answer_alert(&self, client: &Client, text: &str) -> Result<(), Error> {
        client
            .answer_callback_query(self.query_id, Some(text), true)
            .await
            .map(|_| ())
    }
}

/// Fluent builder returned by [`CallbackQuery::answer`]. Finalize with `.send(&client).await`.
pub struct Answer<'a> {
    query_id: i64,
    message: Option<String>,
    alert: bool,
    url: Option<String>,
    cache_time: i32,
    _marker: std::marker::PhantomData<&'a ()>,
}

impl<'a> Answer<'a> {
    /// Show `text` as a toast notification (fades automatically).
    pub fn text<S: Into<String>>(mut self, text: S) -> Self {
        self.message = Some(text.into());
        self.alert = false;
        self
    }

    /// Show `text` as a modal alert the user must dismiss.
    pub fn alert<S: Into<String>>(mut self, text: S) -> Self {
        self.message = Some(text.into());
        self.alert = true;
        self
    }

    /// Open `url` on the client (e.g. to launch a game).
    pub fn url<S: Into<String>>(mut self, url: S) -> Self {
        self.url = Some(url.into());
        self
    }

    /// Cache this answer for `duration` so repeated presses don't reach the bot.
    pub fn cache_time(mut self, duration: std::time::Duration) -> Self {
        self.cache_time = duration.as_secs().min(i32::MAX as u64) as i32;
        self
    }

    /// Send the answer to Telegram.
    pub async fn send(self, client: &Client) -> Result<(), Error> {
        let req = tl::functions::messages::SetBotCallbackAnswer {
            alert: self.alert,
            query_id: self.query_id,
            message: self.message,
            url: self.url,
            cache_time: self.cache_time,
        };
        client.rpc_call_raw_pub(&req).await.map(|_| ())
    }
}

// InlineQuery

/// A user is typing an inline query (`@bot something`).
#[derive(Debug, Clone)]
pub struct InlineQuery {
    pub query_id: i64,
    pub user_id: i64,
    pub query: String,
    pub offset: String,
    /// Peer of the chat the user sent the inline query from, if available.
    pub peer: Option<tl::enums::Peer>,
}

impl InlineQuery {
    /// The text the user typed after the bot username.
    pub fn query(&self) -> &str {
        &self.query
    }
}

// InlineSend

/// A user chose an inline result and sent it.
#[derive(Debug, Clone)]
pub struct InlineSend {
    pub user_id: i64,
    pub query: String,
    pub id: String,
    /// Message ID of the sent message, if available.
    pub msg_id: Option<tl::enums::InputBotInlineMessageId>,
}

impl InlineSend {
    /// Edit the inline message that was sent as a result of this inline query.
    ///
    /// Requires that [`msg_id`] is present (i.e. the result had `peer_type` set).
    /// Returns `Err` with a descriptive message if `msg_id` is `None`.
    ///
    /// [`msg_id`]: InlineSend::msg_id
    ///
    /// # Example
    /// ```rust,no_run
    /// # async fn f(client: ferogram::Client, send: ferogram::update::InlineSend)
    /// # -> Result<(), Box<dyn std::error::Error>> {
    /// send.edit_message(&client, "updated text", None).await?;
    /// # Ok(()) }
    /// ```
    pub async fn edit_message(
        &self,
        client: &Client,
        new_text: &str,
        reply_markup: Option<tl::enums::ReplyMarkup>,
    ) -> Result<bool, Error> {
        let msg_id =
            match self.msg_id.clone() {
                Some(id) => id,
                None => return Err(Error::Deserialize(
                    "InlineSend::edit_message: msg_id is None (bot_inline_send had no peer_type)"
                        .into(),
                )),
            };
        let req = tl::functions::messages::EditInlineBotMessage {
            no_webpage: false,
            invert_media: false,
            id: msg_id,
            message: Some(new_text.to_string()),
            media: None,
            reply_markup,
            entities: None,
        };
        let body = client.rpc_call_raw(&req).await?;
        // Returns Bool
        Ok(!body.is_empty())
    }
}

// RawUpdate

/// A TL update that has no dedicated high-level variant yet.
#[derive(Debug, Clone)]
pub struct RawUpdate {
    /// Constructor ID of the inner update.
    pub constructor_id: u32,
}

/// A user's online / offline status changed.
///
/// Delivered as [`Update::UserStatus`].
///
/// # Example
/// ```rust,no_run
/// # use ferogram::{Update, update::UserStatusUpdate};
/// # async fn example(mut stream: ferogram::UpdateStream) {
/// while let Some(upd) = stream.next().await {
/// if let Update::UserStatus(s) = upd {
///     println!("user {} status: {:?}", s.user_id, s.status);
/// }
/// }
/// # }
/// ```
#[derive(Debug, Clone)]
pub struct UserStatusUpdate {
    /// The bare user ID whose status changed.
    pub user_id: i64,
    /// New online/offline/recently/etc. status.
    pub status: tl::enums::UserStatus,
}

/// A user is performing a chat action (typing, uploading, recording...).
///
/// Delivered as [`Update::UserTyping`].  Covers DMs, groups, and channels.
///
/// # Example
/// ```rust,no_run
/// # use ferogram::{Update, update::ChatActionUpdate};
/// # async fn example(mut stream: ferogram::UpdateStream) {
/// while let Some(upd) = stream.next().await {
/// if let Update::UserTyping(a) = upd {
///     println!("user {} is typing in {:?}", a.user_id, a.peer);
/// }
/// }
/// # }
/// ```
#[derive(Debug, Clone)]
pub struct ChatActionUpdate {
    /// The peer (chat / channel) the action is happening in.
    /// For DM typing updates (`updateUserTyping`) this is the user's own peer.
    pub peer: tl::enums::Peer,
    /// The bare user ID performing the action.
    pub user_id: i64,
    /// What the user is currently doing (typing, uploading video, etc.).
    pub action: tl::enums::SendMessageAction,
}

/// A chat member's status changed (joined, left, promoted, banned, etc.).
///
/// Delivered as [`Update::ParticipantUpdate`].
/// Covers both basic groups (`updateChatParticipant`) and
/// channels/supergroups (`updateChannelParticipant`).
///
/// # Example
/// ```rust,no_run
/// # use ferogram::{Update, update::ParticipantUpdate};
/// # async fn example(mut stream: ferogram::UpdateStream) {
/// while let Some(upd) = stream.next().await {
///     if let Update::ParticipantUpdate(p) = upd {
///         println!(
///             "chat={:?} user={} actor={}: {:?} → {:?}",
///             p.chat_id, p.user_id, p.actor_id,
///             p.prev_participant, p.new_participant,
///         );
///     }
/// }
/// # }
/// ```
#[derive(Debug, Clone)]
pub struct ParticipantUpdate {
    /// The chat (basic-group ID) or channel ID the event happened in.
    pub chat_id: i64,
    /// The user whose membership changed.
    pub user_id: i64,
    /// The admin or bot that triggered the change.
    pub actor_id: i64,
    /// Unix timestamp of the event.
    pub date: i32,
    /// Previous participant record for basic groups
    /// (`None` = user wasn't in the chat before, or this is a channel update).
    pub prev_participant: Option<tl::enums::ChatParticipant>,
    /// New participant record for basic groups
    /// (`None` = user left / was kicked, or this is a channel update).
    pub new_participant: Option<tl::enums::ChatParticipant>,
    /// Previous participant record for channels/supergroups
    /// (`None` for basic-group updates).
    pub prev_channel_participant: Option<tl::enums::ChannelParticipant>,
    /// New participant record for channels/supergroups
    /// (`None` for basic-group updates).
    pub new_channel_participant: Option<tl::enums::ChannelParticipant>,
    /// The invite link used, if any.
    pub invite: Option<tl::enums::ExportedChatInvite>,
    /// QTS counter (used for acknowledgement).
    pub qts: i32,
    /// `true` when the update comes from a channel/supergroup,
    /// `false` for a basic group.
    pub is_channel: bool,
}

/// A user has requested to join a chat via an invite link.
///
/// Delivered as [`Update::JoinRequest`].
/// Only sent to bots that manage the chat (requires `manage_chat` admin right).
///
/// # Example
/// ```rust,no_run
/// # use ferogram::{Update, update::JoinRequestUpdate};
/// # async fn example(mut stream: ferogram::UpdateStream) {
/// while let Some(upd) = stream.next().await {
///     if let Update::JoinRequest(r) = upd {
///         println!("user {} wants to join {:?}: {:?}", r.user_id, r.peer, r.about);
///     }
/// }
/// # }
/// ```
#[derive(Debug, Clone)]
pub struct JoinRequestUpdate {
    /// The chat/channel/group the request is for.
    pub peer: tl::enums::Peer,
    /// The user requesting to join.
    pub user_id: i64,
    /// The user's bio / message attached to the request.
    pub about: String,
    /// The invite link they used.
    pub invite: tl::enums::ExportedChatInvite,
    /// Unix timestamp.
    pub date: i32,
    /// QTS counter.
    pub qts: i32,
}

/// A bot received a reaction on one of its messages.
///
/// Delivered as [`Update::MessageReaction`]. Only for bots.
#[derive(Debug, Clone)]
pub struct MessageReactionUpdate {
    /// The peer (chat/channel) where the reaction occurred.
    pub peer: tl::enums::Peer,
    /// The message ID that was reacted to.
    pub msg_id: i32,
    /// Unix timestamp.
    pub date: i32,
    /// The peer that reacted.
    pub actor: tl::enums::Peer,
    /// Reactions that were removed.
    pub old_reactions: Vec<tl::enums::Reaction>,
    /// Reactions that were added.
    pub new_reactions: Vec<tl::enums::Reaction>,
    /// QTS counter.
    pub qts: i32,
}

/// A user voted in a poll.
///
/// Delivered as [`Update::PollVote`]. Only for bots that sent the poll.
#[derive(Debug, Clone)]
pub struct PollVoteUpdate {
    /// The poll ID.
    pub poll_id: i64,
    /// The peer that voted.
    pub peer: tl::enums::Peer,
    /// The option bytes they selected.
    pub options: Vec<Vec<u8>>,
    /// Their positions in the option list.
    pub positions: Vec<i32>,
    /// QTS counter.
    pub qts: i32,
}

/// A user stopped (or restarted) the bot.
///
/// Delivered as [`Update::BotStopped`].
#[derive(Debug, Clone)]
pub struct BotStoppedUpdate {
    /// The user who stopped/restarted the bot.
    pub user_id: i64,
    /// Unix timestamp.
    pub date: i32,
    /// `true` if the bot was stopped, `false` if restarted.
    pub stopped: bool,
    /// QTS counter.
    pub qts: i32,
}

/// A user submitted a shipping address for a physical-goods invoice.
///
/// Delivered as [`Update::ShippingQuery`]. Only for bots.
///
/// Respond with [`Client::answer_shipping_query`].
#[derive(Debug, Clone)]
pub struct ShippingQueryUpdate {
    /// The query ID, pass to `answer_shipping_query`.
    pub query_id: i64,
    /// The user who submitted the address.
    pub user_id: i64,
    /// The invoice payload you set in `send_invoice`.
    pub payload: Vec<u8>,
    /// The address the user entered.
    pub shipping_address: tl::types::PostAddress,
}

/// A user confirmed payment on the final checkout screen.
///
/// Delivered as [`Update::PreCheckoutQuery`]. Only for bots.
///
/// Respond within 10 seconds via [`Client::answer_precheckout_query`].
#[derive(Debug, Clone)]
pub struct PreCheckoutQueryUpdate {
    /// The query ID, pass to `answer_precheckout_query`.
    pub query_id: i64,
    /// The user who pressed "Pay".
    pub user_id: i64,
    /// The invoice payload you set in `send_invoice`.
    pub payload: Vec<u8>,
    /// Payment info (name, email, phone, etc.) if requested.
    pub info: Option<tl::types::PaymentRequestedInfo>,
    /// The chosen shipping option ID, if applicable.
    pub shipping_option_id: Option<String>,
    /// ISO 4217 currency code (e.g. `"USD"`).
    pub currency: String,
    /// Total amount in the smallest currency unit (e.g. cents).
    pub total_amount: i64,
}

/// A channel was boosted via the bot.
///
/// Delivered as [`Update::ChatBoost`]. Only for bots that manage the channel.
#[derive(Debug, Clone)]
pub struct ChatBoostUpdate {
    /// The channel/chat that was boosted.
    pub peer: tl::enums::Peer,
    /// The boost record.
    pub boost: tl::enums::Boost,
    /// QTS counter.
    pub qts: i32,
}

/// A high-level event received from Telegram.
#[non_exhaustive]
#[derive(Debug, Clone)]
pub enum Update {
    /// A new message (personal chat, group, channel, or bot command).
    NewMessage(IncomingMessage),
    /// An existing message was edited.
    MessageEdited(IncomingMessage),
    /// One or more messages were deleted.
    MessageDeleted(MessageDeletion),
    /// An inline keyboard button was pressed on a bot message.
    CallbackQuery(CallbackQuery),
    /// A user typed an inline query for the bot.
    InlineQuery(InlineQuery),
    /// A user chose an inline result and sent it (bots only).
    InlineSend(InlineSend),
    /// A user's online status changed.
    UserStatus(UserStatusUpdate),
    /// A user is typing / uploading / recording in a chat.
    UserTyping(ChatActionUpdate),
    /// A chat member's status changed (joined, left, promoted, banned).
    /// Covers both basic groups and channels/supergroups.
    ParticipantUpdate(ParticipantUpdate),
    /// A user requested to join a chat via an invite link (bots only).
    JoinRequest(JoinRequestUpdate),
    /// A bot received a reaction on one of its messages (bots only).
    MessageReaction(MessageReactionUpdate),
    /// A user voted in a poll (bots only).
    PollVote(PollVoteUpdate),
    /// A user stopped or restarted the bot.
    BotStopped(BotStoppedUpdate),
    /// A user submitted a shipping address for a physical-goods invoice (bots only).
    ShippingQuery(ShippingQueryUpdate),
    /// A user confirmed payment on the final pre-checkout screen (bots only).
    PreCheckoutQuery(PreCheckoutQueryUpdate),
    /// A channel was boosted via the bot (bots only).
    ChatBoost(ChatBoostUpdate),
    /// A raw TL update not mapped to any of the above variants.
    Raw(RawUpdate),
}

// MTProto update container IDs

#[allow(dead_code)]
const ID_UPDATES_TOO_LONG: u32 = 0xe317af7e;
#[allow(dead_code)]
const ID_UPDATE_SHORT_MESSAGE: u32 = 0x313bc7f8;
#[allow(dead_code)]
const ID_UPDATE_SHORT_CHAT_MSG: u32 = 0x4d6deea5;
#[allow(dead_code)]
const ID_UPDATE_SHORT: u32 = 0x78d4dec1;
#[allow(dead_code)]
const ID_UPDATES: u32 = 0x74ae4240;
#[allow(dead_code)]
const ID_UPDATES_COMBINED: u32 = 0x725b04c3;
#[allow(dead_code)]
const ID_UPDATE_SHORT_SENT_MSG: u32 = 0x9015e101;

// Parser

/// Parse raw update container bytes into high-level [`Update`] values.
#[allow(dead_code)]
pub(crate) fn parse_updates(bytes: &[u8]) -> Vec<Update> {
    if bytes.len() < 4 {
        return vec![];
    }
    let cid = u32::from_le_bytes(bytes[..4].try_into().unwrap());

    match cid {
        ID_UPDATES_TOO_LONG => {
            tracing::warn!(
                "[ferogram] updatesTooLong: call client.get_difference() to recover missed updates"
            );
            vec![]
        }

        ID_UPDATE_SHORT_MESSAGE => {
            let mut cur = Cursor::from_slice(&bytes[4..]); // skip constructor prefix
            match tl::types::UpdateShortMessage::deserialize(&mut cur) {
                Ok(m) => vec![Update::NewMessage(make_short_dm(m))],
                Err(e) => {
                    tracing::debug!(
                        "[ferogram] updateShortMessage parse error (unknown constructor or newer layer): {e}"
                    );
                    vec![]
                }
            }
        }

        ID_UPDATE_SHORT_CHAT_MSG => {
            let mut cur = Cursor::from_slice(&bytes[4..]); // skip constructor prefix
            match tl::types::UpdateShortChatMessage::deserialize(&mut cur) {
                Ok(m) => vec![Update::NewMessage(make_short_chat(m))],
                Err(e) => {
                    tracing::debug!(
                        "[ferogram] updateShortChatMessage parse error (unknown constructor or newer layer): {e}"
                    );
                    vec![]
                }
            }
        }

        ID_UPDATE_SHORT => {
            let mut cur = Cursor::from_slice(&bytes[4..]); // skip constructor prefix
            match tl::types::UpdateShort::deserialize(&mut cur) {
                Ok(m) => from_single_update(m.update),
                Err(e) => {
                    tracing::debug!(
                        "[ferogram] updateShort parse error (unknown constructor or newer layer): {e}"
                    );
                    vec![]
                }
            }
        }

        ID_UPDATES => {
            let mut cur = Cursor::from_slice(bytes);
            match tl::enums::Updates::deserialize(&mut cur) {
                Ok(tl::enums::Updates::Updates(u)) => {
                    u.updates.into_iter().flat_map(from_single_update).collect()
                }
                Err(e) => {
                    tracing::debug!(
                        "[ferogram] Updates parse error (unknown constructor or newer layer): {e}"
                    );
                    vec![]
                }
                _ => vec![],
            }
        }

        ID_UPDATES_COMBINED => {
            let mut cur = Cursor::from_slice(bytes);
            match tl::enums::Updates::deserialize(&mut cur) {
                Ok(tl::enums::Updates::Combined(u)) => {
                    u.updates.into_iter().flat_map(from_single_update).collect()
                }
                Err(e) => {
                    tracing::debug!(
                        "[ferogram] UpdatesCombined parse error (unknown constructor or newer layer): {e}"
                    );
                    vec![]
                }
                _ => vec![],
            }
        }

        // updateShortSentMessage: pts is now handled by dispatch_updates/route_frame
        // directly (via EnvelopeResult::Pts or the push branch). parse_updates is only
        // called for the old code path; we absorb here as a safe fallback.
        ID_UPDATE_SHORT_SENT_MSG => vec![],

        _ => vec![],
    }
}

/// Convert a single `tl::enums::Update` into a `Vec<Update>`.
pub fn from_single_update_pub(upd: tl::enums::Update) -> Vec<Update> {
    from_single_update(upd)
}

/// Convert a single `tl::enums::Update` into a `Vec<Update>`.
fn from_single_update(upd: tl::enums::Update) -> Vec<Update> {
    use tl::enums::Update::*;
    match upd {
        NewMessage(u) => vec![Update::NewMessage(IncomingMessage::from_raw(u.message))],
        NewChannelMessage(u) => vec![Update::NewMessage(IncomingMessage::from_raw(u.message))],
        EditMessage(u) => vec![Update::MessageEdited(IncomingMessage::from_raw(u.message))],
        EditChannelMessage(u) => vec![Update::MessageEdited(IncomingMessage::from_raw(u.message))],
        DeleteMessages(u) => vec![Update::MessageDeleted(MessageDeletion {
            message_ids: u.messages,
            channel_id: None,
        })],
        DeleteChannelMessages(u) => vec![Update::MessageDeleted(MessageDeletion {
            message_ids: u.messages,
            channel_id: Some(u.channel_id),
        })],
        BotCallbackQuery(u) => vec![Update::CallbackQuery(CallbackQuery {
            query_id: u.query_id,
            user_id: u.user_id,
            message_id: Some(u.msg_id),
            chat_instance: u.chat_instance,
            data_raw: u.data,
            game_short_name: u.game_short_name,
            chat_peer: Some(u.peer),
            inline_msg_id: None,
        })],
        InlineBotCallbackQuery(u) => vec![Update::CallbackQuery(CallbackQuery {
            query_id: u.query_id,
            user_id: u.user_id,
            message_id: None,
            chat_instance: u.chat_instance,
            data_raw: u.data,
            game_short_name: u.game_short_name,
            chat_peer: None,
            inline_msg_id: Some(u.msg_id),
        })],
        BotInlineQuery(u) => vec![Update::InlineQuery(InlineQuery {
            query_id: u.query_id,
            user_id: u.user_id,
            query: u.query,
            offset: u.offset,
            peer: None,
        })],
        BotInlineSend(u) => vec![Update::InlineSend(InlineSend {
            user_id: u.user_id,
            query: u.query,
            id: u.id,
            msg_id: u.msg_id,
        })],
        // typed UserStatus variant
        UserStatus(u) => vec![Update::UserStatus(UserStatusUpdate {
            user_id: u.user_id,
            status: u.status,
        })],
        // typed ChatAction variant: DM typing
        UserTyping(u) => vec![Update::UserTyping(ChatActionUpdate {
            peer: tl::enums::Peer::User(tl::types::PeerUser { user_id: u.user_id }),
            user_id: u.user_id,
            action: u.action,
        })],
        // group typing
        ChatUserTyping(u) => vec![Update::UserTyping(ChatActionUpdate {
            peer: tl::enums::Peer::Chat(tl::types::PeerChat { chat_id: u.chat_id }),
            user_id: match u.from_id {
                tl::enums::Peer::User(ref p) => p.user_id,
                tl::enums::Peer::Chat(ref p) => p.chat_id,
                tl::enums::Peer::Channel(ref p) => p.channel_id,
            },
            action: u.action,
        })],
        // channel / supergroup typing
        ChannelUserTyping(u) => vec![Update::UserTyping(ChatActionUpdate {
            peer: tl::enums::Peer::Channel(tl::types::PeerChannel {
                channel_id: u.channel_id,
            }),
            user_id: match u.from_id {
                tl::enums::Peer::User(ref p) => p.user_id,
                tl::enums::Peer::Chat(ref p) => p.chat_id,
                tl::enums::Peer::Channel(ref p) => p.channel_id,
            },
            action: u.action,
        })],
        // basic-group participant change
        ChatParticipant(u) => vec![Update::ParticipantUpdate(ParticipantUpdate {
            chat_id: u.chat_id,
            user_id: u.user_id,
            actor_id: u.actor_id,
            date: u.date,
            prev_participant: u.prev_participant,
            new_participant: u.new_participant,
            prev_channel_participant: None,
            new_channel_participant: None,
            invite: u.invite,
            qts: u.qts,
            is_channel: false,
        })],
        // channel/supergroup participant change
        ChannelParticipant(u) => vec![Update::ParticipantUpdate(ParticipantUpdate {
            chat_id: u.channel_id,
            user_id: u.user_id,
            actor_id: u.actor_id,
            date: u.date,
            prev_participant: None,
            new_participant: None,
            prev_channel_participant: u.prev_participant,
            new_channel_participant: u.new_participant,
            invite: u.invite,
            qts: u.qts,
            is_channel: true,
        })],
        // join request (bots only)
        BotChatInviteRequester(u) => vec![Update::JoinRequest(JoinRequestUpdate {
            peer: u.peer,
            user_id: u.user_id,
            about: u.about,
            invite: u.invite,
            date: u.date,
            qts: u.qts,
        })],
        // message reaction (bots only)
        BotMessageReaction(u) => vec![Update::MessageReaction(MessageReactionUpdate {
            peer: u.peer,
            msg_id: u.msg_id,
            date: u.date,
            actor: u.actor,
            old_reactions: u.old_reactions,
            new_reactions: u.new_reactions,
            qts: u.qts,
        })],
        // poll vote (bots only)
        MessagePollVote(u) => vec![Update::PollVote(PollVoteUpdate {
            poll_id: u.poll_id,
            peer: u.peer,
            options: u.options,
            positions: u.positions,
            qts: u.qts,
        })],
        // bot stopped / restarted
        BotStopped(u) => vec![Update::BotStopped(BotStoppedUpdate {
            user_id: u.user_id,
            date: u.date,
            stopped: u.stopped,
            qts: u.qts,
        })],
        // shipping query (bots only)
        BotShippingQuery(u) => vec![Update::ShippingQuery(ShippingQueryUpdate {
            query_id: u.query_id,
            user_id: u.user_id,
            payload: u.payload,
            shipping_address: match u.shipping_address {
                tl::enums::PostAddress::PostAddress(a) => a,
            },
        })],
        // pre-checkout query (bots only)
        BotPrecheckoutQuery(u) => vec![Update::PreCheckoutQuery(PreCheckoutQueryUpdate {
            query_id: u.query_id,
            user_id: u.user_id,
            payload: u.payload,
            info: u.info.map(|i| match i {
                tl::enums::PaymentRequestedInfo::PaymentRequestedInfo(x) => x,
            }),
            shipping_option_id: u.shipping_option_id,
            currency: u.currency,
            total_amount: u.total_amount,
        })],
        // channel boost (bots only)
        BotChatBoost(u) => vec![Update::ChatBoost(ChatBoostUpdate {
            peer: u.peer,
            boost: u.boost,
            qts: u.qts,
        })],
        other => {
            let cid = tl_constructor_id(&other);
            vec![Update::Raw(RawUpdate {
                constructor_id: cid,
            })]
        }
    }
}

/// Extract constructor ID from a `tl::enums::Update` variant.
fn tl_constructor_id(upd: &tl::enums::Update) -> u32 {
    use tl::enums::Update::*;
    match upd {
        AttachMenuBots => 0x17b7a20b,
        AutoSaveSettings => 0xec05b097,
        BotBusinessConnect(_) => 0x8ae5c97a,
        BotCallbackQuery(_) => 0xb9cfc48d,
        BotChatBoost(_) => 0x904dd49c,
        BotChatInviteRequester(_) => 0x11dfa986,
        BotCommands(_) => 0x4d712f2e,
        BotDeleteBusinessMessage(_) => 0xa02a982e,
        BotEditBusinessMessage(_) => 0x7df587c,
        BotInlineQuery(_) => 0x496f379c,
        BotInlineSend(_) => 0x12f12a07,
        BotMenuButton(_) => 0x14b85813,
        BotMessageReaction(_) => 0xac21d3ce,
        BotMessageReactions(_) => 0x9cb7759,
        BotNewBusinessMessage(_) => 0x9ddb347c,
        BotPrecheckoutQuery(_) => 0x8caa9a96,
        BotPurchasedPaidMedia(_) => 0x283bd312,
        BotShippingQuery(_) => 0xb5aefd7d,
        BotStopped(_) => 0xc4870a49,
        BotWebhookJson(_) => 0x8317c0c3,
        BotWebhookJsonquery(_) => 0x9b9240a6,
        BusinessBotCallbackQuery(_) => 0x1ea2fda7,
        Channel(_) => 0x635b4c09,
        ChannelAvailableMessages(_) => 0xb23fc698,
        ChannelMessageForwards(_) => 0xd29a27f4,
        ChannelMessageViews(_) => 0xf226ac08,
        ChannelParticipant(_) => 0x985d3abb,
        ChannelReadMessagesContents(_) => 0x25f324f7,
        ChannelTooLong(_) => 0x108d941f,
        ChannelUserTyping(_) => 0x8c88c923,
        ChannelViewForumAsMessages(_) => 0x7b68920,
        ChannelWebPage(_) => 0x2f2ba99f,
        Chat(_) => 0xf89a6a4e,
        ChatDefaultBannedRights(_) => 0x54c01850,
        ChatParticipant(_) => 0xd087663a,
        ChatParticipantAdd(_) => 0x3dda5451,
        ChatParticipantAdmin(_) => 0xd7ca61a2,
        ChatParticipantDelete(_) => 0xe32f3d77,
        ChatParticipants(_) => 0x7761198,
        ChatUserTyping(_) => 0x83487af0,
        Config => 0xa229dd06,
        ContactsReset => 0x7084a7be,
        DcOptions(_) => 0x8e5e9873,
        DeleteChannelMessages(_) => 0xc32d5b12,
        DeleteGroupCallMessages(_) => 0x3e85e92c,
        DeleteMessages(_) => 0xa20db0e5,
        DeleteQuickReply(_) => 0x53e6f1ec,
        DeleteQuickReplyMessages(_) => 0x566fe7cd,
        DeleteScheduledMessages(_) => 0xf2a71983,
        DialogFilter(_) => 0x26ffde7d,
        DialogFilterOrder(_) => 0xa5d72105,
        DialogFilters => 0x3504914f,
        DialogPinned(_) => 0x6e6fe51c,
        DialogUnreadMark(_) => 0xb658f23e,
        DraftMessage(_) => 0xedfc111e,
        EditChannelMessage(_) => 0x1b3f4df7,
        EditMessage(_) => 0xe40370a3,
        EmojiGameInfo(_) => 0xfb9c547a,
        EncryptedChatTyping(_) => 0x1710f156,
        EncryptedMessagesRead(_) => 0x38fe25b7,
        Encryption(_) => 0xb4a2e88d,
        FavedStickers => 0xe511996d,
        FolderPeers(_) => 0x19360dc0,
        GeoLiveViewed(_) => 0x871fb939,
        GroupCall(_) => 0x9d2216e0,
        GroupCallChainBlocks(_) => 0xa477288f,
        GroupCallConnection(_) => 0xb783982,
        GroupCallEncryptedMessage(_) => 0xc957a766,
        GroupCallMessage(_) => 0xd8326f0d,
        GroupCallParticipants(_) => 0xf2ebdb4e,
        InlineBotCallbackQuery(_) => 0x691e9052,
        LangPack(_) => 0x56022f4d,
        LangPackTooLong(_) => 0x46560264,
        LoginToken => 0x564fe691,
        MessageExtendedMedia(_) => 0xd5a41724,
        MessageId(_) => 0x4e90bfd6,
        MessagePoll(_) => 0xaca1657b,
        MessagePollVote(_) => 0x24f40e77,
        MessageReactions(_) => 0x1e297bfa,
        MonoForumNoPaidException(_) => 0x9f812b08,
        MoveStickerSetToTop(_) => 0x86fccf85,
        NewAuthorization(_) => 0x8951abef,
        NewChannelMessage(_) => 0x62ba04d9,
        NewEncryptedMessage(_) => 0x12bcbd9a,
        NewMessage(_) => 0x1f2b0afd,
        NewQuickReply(_) => 0xf53da717,
        NewScheduledMessage(_) => 0x39a51dfb,
        NewStickerSet(_) => 0x688a30aa,
        NewStoryReaction(_) => 0x1824e40b,
        NotifySettings(_) => 0xbec268ef,
        PaidReactionPrivacy(_) => 0x8b725fce,
        PeerBlocked(_) => 0xebe07752,
        PeerHistoryTtl(_) => 0xbb9bb9a5,
        PeerLocated(_) => 0xb4afcfb0,
        PeerSettings(_) => 0x6a7e7366,
        PeerWallpaper(_) => 0xae3f101d,
        PendingJoinRequests(_) => 0x7063c3db,
        PhoneCall(_) => 0xab0f6b1e,
        PhoneCallSignalingData(_) => 0x2661bf09,
        PinnedChannelMessages(_) => 0x5bb98608,
        PinnedDialogs(_) => 0xfa0f3ca2,
        PinnedForumTopic(_) => 0x683b2c52,
        PinnedForumTopics(_) => 0xdef143d0,
        PinnedMessages(_) => 0xed85eab5,
        PinnedSavedDialogs(_) => 0x686c85a6,
        Privacy(_) => 0xee3b272a,
        PtsChanged => 0x3354678f,
        QuickReplies(_) => 0xf9470ab2,
        QuickReplyMessage(_) => 0x3e050d0f,
        ReadChannelDiscussionInbox(_) => 0xd6b19546,
        ReadChannelDiscussionOutbox(_) => 0x695c9e7c,
        ReadChannelInbox(_) => 0x922e6e10,
        ReadChannelOutbox(_) => 0xb75f99a9,
        ReadFeaturedEmojiStickers => 0xfb4c496c,
        ReadFeaturedStickers => 0x571d2742,
        ReadHistoryInbox(_) => 0x9e84bc99,
        ReadHistoryOutbox(_) => 0x2f2f21bf,
        ReadMessagesContents(_) => 0xf8227181,
        ReadMonoForumInbox(_) => 0x77b0e372,
        ReadMonoForumOutbox(_) => 0xa4a79376,
        ReadStories(_) => 0xf74e932b,
        RecentEmojiStatuses => 0x30f443db,
        RecentReactions => 0x6f7863f4,
        RecentStickers => 0x9a422c20,
        SavedDialogPinned(_) => 0xaeaf9e74,
        SavedGifs => 0x9375341e,
        SavedReactionTags => 0x39c67432,
        SavedRingtones => 0x74d8be99,
        SentPhoneCode(_) => 0x504aa18f,
        SentStoryReaction(_) => 0x7d627683,
        ServiceNotification(_) => 0xebe46819,
        SmsJob(_) => 0xf16269d4,
        StarGiftAuctionState(_) => 0x48e246c2,
        StarGiftAuctionUserState(_) => 0xdc58f31e,
        StarGiftCraftFail => 0xac072444,
        StarsBalance(_) => 0x4e80a379,
        StarsRevenueStatus(_) => 0xa584b019,
        StickerSets(_) => 0x31c24808,
        StickerSetsOrder(_) => 0xbb2d201,
        StoriesStealthMode(_) => 0x2c084dc1,
        Story(_) => 0x75b3b798,
        StoryId(_) => 0x1bf335b9,
        Theme(_) => 0x8216fba3,
        TranscribedAudio(_) => 0x84cd5a,
        User(_) => 0x20529438,
        UserEmojiStatus(_) => 0x28373599,
        UserName(_) => 0xa7848924,
        UserPhone(_) => 0x5492a13,
        UserStatus(_) => 0xe5bdf8de,
        UserTyping(_) => 0x2a17bf5c,
        WebPage(_) => 0x7f891213,
        WebViewResultSent(_) => 0x1592b79d,
        ChatParticipantRank(_) => 0xbd8367b9,
        ManagedBot(_) => 0x4880ed9a,
    }
}

// Short message helpers

pub(crate) fn make_short_dm(m: tl::types::UpdateShortMessage) -> IncomingMessage {
    let msg = tl::types::Message {
        out: m.out,
        mentioned: m.mentioned,
        media_unread: m.media_unread,
        silent: m.silent,
        post: false,
        from_scheduled: false,
        legacy: false,
        edit_hide: false,
        pinned: false,
        noforwards: false,
        invert_media: false,
        offline: false,
        video_processing_pending: false,
        id: m.id,
        from_id: Some(tl::enums::Peer::User(tl::types::PeerUser {
            user_id: m.user_id,
        })),
        peer_id: tl::enums::Peer::User(tl::types::PeerUser { user_id: m.user_id }),
        saved_peer_id: None,
        fwd_from: m.fwd_from,
        via_bot_id: m.via_bot_id,
        via_business_bot_id: None,
        reply_to: m.reply_to,
        date: m.date,
        message: m.message,
        media: None,
        reply_markup: None,
        entities: m.entities,
        views: None,
        forwards: None,
        replies: None,
        edit_date: None,
        post_author: None,
        grouped_id: None,
        reactions: None,
        restriction_reason: None,
        ttl_period: None,
        quick_reply_shortcut_id: None,
        effect: None,
        factcheck: None,
        report_delivery_until_date: None,
        paid_message_stars: None,
        suggested_post: None,
        from_rank: None,
        from_boosts_applied: None,
        paid_suggested_post_stars: false,
        paid_suggested_post_ton: false,
        schedule_repeat_period: None,
        summary_from_language: None,
    };
    IncomingMessage {
        raw: tl::enums::Message::Message(msg),
        client: None,
    }
}

pub(crate) fn make_short_chat(m: tl::types::UpdateShortChatMessage) -> IncomingMessage {
    let msg = tl::types::Message {
        out: m.out,
        mentioned: m.mentioned,
        media_unread: m.media_unread,
        silent: m.silent,
        post: false,
        from_scheduled: false,
        legacy: false,
        edit_hide: false,
        pinned: false,
        noforwards: false,
        invert_media: false,
        offline: false,
        video_processing_pending: false,
        id: m.id,
        from_id: Some(tl::enums::Peer::User(tl::types::PeerUser {
            user_id: m.from_id,
        })),
        peer_id: tl::enums::Peer::Chat(tl::types::PeerChat { chat_id: m.chat_id }),
        saved_peer_id: None,
        fwd_from: m.fwd_from,
        via_bot_id: m.via_bot_id,
        via_business_bot_id: None,
        reply_to: m.reply_to,
        date: m.date,
        message: m.message,
        media: None,
        reply_markup: None,
        entities: m.entities,
        views: None,
        forwards: None,
        replies: None,
        edit_date: None,
        post_author: None,
        grouped_id: None,
        reactions: None,
        restriction_reason: None,
        ttl_period: None,
        quick_reply_shortcut_id: None,
        effect: None,
        factcheck: None,
        report_delivery_until_date: None,
        paid_message_stars: None,
        suggested_post: None,
        from_rank: None,
        from_boosts_applied: None,
        paid_suggested_post_stars: false,
        paid_suggested_post_ton: false,
        schedule_repeat_period: None,
        summary_from_language: None,
    };
    IncomingMessage {
        raw: tl::enums::Message::Message(msg),
        client: None,
    }
}
