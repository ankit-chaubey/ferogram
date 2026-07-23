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

#[allow(unused_imports)]
use super::random_i64;
use crate::*;
#[allow(unused_imports)]
use crate::{
    InputMessage, InvocationError, PeerRef,
    dialog::{Dialog, DialogIter, MessageIter},
    inline_iter, media, participants, search, update,
};
#[allow(unused_imports)]
use ferogram_tl_types::{Cursor, Deserializable};

impl Client {
    /// Fluent search builder for in-chat message search.
    pub fn search(&self, peer: impl Into<PeerRef>, query: &str) -> SearchBuilder {
        SearchBuilder::new(peer.into(), query.to_string())
    }

    /// Fluent builder for global cross-chat search.
    pub fn search_global(&self, query: &str) -> GlobalSearchBuilder {
        GlobalSearchBuilder::new(query.to_string())
    }

    /// Send a message to a peer.
    ///
    /// If `msg` carries attached media (set via [`InputMessage::copy_media`]),
    /// this dispatches through `messages.SendMedia` instead of `messages.SendMessage`,
    /// since the latter has no media field and would silently drop it.
    pub async fn send_message(
        &self,
        peer: impl Into<PeerRef>,
        msg: impl Into<InputMessage>,
    ) -> Result<update::IncomingMessage, InvocationError> {
        let peer = peer.into().resolve(self).await?;
        let input_peer = self.inner.peer_cache.read().await.peer_to_input(&peer)?;
        let msg = msg.into();
        let entities = self.resolve_outgoing_entities(msg.entities.clone()).await;
        // msg is borrowed whole in parse_send_response below, so anything
        // non-Copy we need for the request has to be cloned out here rather
        // than moved - same reason entities is cloned above.
        let send_as = self.resolve_send_as(msg.send_as.clone()).await?;
        let quick_reply_shortcut = msg.quick_reply_shortcut();
        let suggested_post = msg.suggested_post.clone();

        let body: Vec<u8> = if let Some(media) = msg.media.clone() {
            if msg.rich_message.is_some() {
                tracing::warn!(
                    "[ferogram::client] rich_text() is ignored when media is attached; \
                     messages.sendMedia has no rich_message field"
                );
            }
            let req = tl::functions::messages::SendMedia {
                silent: msg.silent,
                background: msg.background,
                clear_draft: msg.clear_draft,
                noforwards: msg.noforwards,
                update_stickersets_order: msg.update_stickersets_order,
                invert_media: msg.invert_media,
                allow_paid_floodskip: msg.allow_paid_floodskip,
                peer: input_peer,
                reply_to: msg.reply_header(),
                media,
                message: msg.text.clone(),
                random_id: random_i64(),
                reply_markup: msg.reply_markup.clone(),
                entities,
                schedule_date: msg.schedule_date,
                schedule_repeat_period: msg.schedule_repeat_period,
                send_as,
                quick_reply_shortcut,
                effect: msg.effect,
                allow_paid_stars: msg.allow_paid_stars,
                suggested_post,
            };
            self.rpc_call_raw(&req).await?
        } else {
            let req = tl::functions::messages::SendMessage {
                no_webpage: msg.no_webpage,
                silent: msg.silent,
                background: msg.background,
                clear_draft: msg.clear_draft,
                noforwards: msg.noforwards,
                update_stickersets_order: msg.update_stickersets_order,
                invert_media: msg.invert_media,
                allow_paid_floodskip: msg.allow_paid_floodskip,
                peer: input_peer,
                reply_to: msg.reply_header(),
                message: msg.text.clone(),
                random_id: random_i64(),
                reply_markup: msg.reply_markup.clone(),
                entities,
                schedule_date: msg.schedule_date,
                schedule_repeat_period: msg.schedule_repeat_period,
                send_as,
                quick_reply_shortcut,
                effect: msg.effect,
                allow_paid_stars: msg.allow_paid_stars,
                suggested_post,
                rich_message: msg.rich_message.clone(),
            };
            self.rpc_call_raw(&req).await?
        };

        Ok(self.parse_send_response(&body, &msg, &peer).await)
    }

    /// Convert `MessageEntity::MentionName` entities (what the markdown/html
    /// parsers emit for `tg://user?id=N`) into the `InputMessageEntityMentionName`
    /// constructor that Telegram actually requires on outgoing messages,
    /// resolving each user's `access_hash` from the peer cache.
    ///
    /// `messageEntityMentionName` (bare `user_id:long`, no access_hash) is the
    /// constructor Telegram sends back to you on *received* messages. Echoing
    /// it back on a *send* is a no-op server-side: Telegram can't resolve a
    /// peer from a bare integer, so the entity is silently dropped and the
    /// mention renders as plain text (0 entities, exactly as reported).
    ///
    /// If a mentioned user hasn't been seen yet (no cached access_hash), the
    /// entity is dropped with a warning rather than sending a request the
    /// server would reject anyway. The peer cache is populated automatically
    /// from incoming updates, so mentioning someone who has recently messaged
    /// the chat (e.g. the sender you're replying to) works as expected.
    /// Resolve `InputMessage::send_as` into the `InputPeer` the raw request
    /// needs, through the same peer cache every other peer argument uses.
    pub(crate) async fn resolve_send_as(
        &self,
        send_as: Option<PeerRef>,
    ) -> Result<Option<tl::enums::InputPeer>, InvocationError> {
        match send_as {
            Some(p) => {
                let peer = p.resolve(self).await?;
                let cache = self.inner.peer_cache.read().await;
                Ok(Some(cache.peer_to_input(&peer)?))
            }
            None => Ok(None),
        }
    }

    pub(crate) async fn resolve_outgoing_entities(
        &self,
        entities: Option<Vec<tl::enums::MessageEntity>>,
    ) -> Option<Vec<tl::enums::MessageEntity>> {
        let entities = entities?;
        if !entities
            .iter()
            .any(|e| matches!(e, tl::enums::MessageEntity::MentionName(_)))
        {
            return Some(entities);
        }

        let cache = self.inner.peer_cache.read().await;
        let mut out = Vec::with_capacity(entities.len());
        for e in entities {
            match e {
                tl::enums::MessageEntity::MentionName(m) => {
                    match cache.users.get(&m.user_id).copied() {
                        Some(access_hash) => {
                            out.push(tl::enums::MessageEntity::InputMessageEntityMentionName(
                                tl::types::InputMessageEntityMentionName {
                                    offset: m.offset,
                                    length: m.length,
                                    user_id: tl::enums::InputUser::InputUser(
                                        tl::types::InputUser {
                                            user_id: m.user_id,
                                            access_hash,
                                        },
                                    ),
                                },
                            ));
                        }
                        None => {
                            tracing::warn!(
                                "[ferogram::client] dropping mention entity: user {} not in peer cache (update loop not running?)",
                                m.user_id
                            );
                        }
                    }
                }
                other => out.push(other),
            }
        }
        if out.is_empty() { None } else { Some(out) }
    }

    pub(crate) async fn parse_send_response(
        &self,
        body: &[u8],

        input: &InputMessage,
        peer: &tl::enums::Peer,
    ) -> update::IncomingMessage {
        if body.len() < 4 {
            return self.synthetic_sent_from_short(input, peer, 0, 0);
        }
        let cid = u32::from_le_bytes(body[..4].try_into().unwrap());

        // updates#74ae4240 / updatesCombined#725b04c3: full Updates container
        if cid == 0x74ae4240 || cid == 0x725b04c3 {
            let mut cur = Cursor::from_slice(body);
            if let Ok(tl::enums::Updates::Updates(u)) = tl::enums::Updates::deserialize(&mut cur) {
                // Cache users/chats before the dispatch_updates spawn runs,
                // to prevent PeerNotCached races on the calling side.
                self.cache_users_and_chats(&u.users, &u.chats).await;
                for upd in &u.updates {
                    if let tl::enums::Update::NewMessage(nm) = upd {
                        return update::IncomingMessage::from_raw(nm.message.clone())
                            .with_client(self.clone());
                    }
                    if let tl::enums::Update::NewChannelMessage(nm) = upd {
                        return update::IncomingMessage::from_raw(nm.message.clone())
                            .with_client(self.clone());
                    }
                }
            }
            if let Ok(tl::enums::Updates::Combined(u)) =
                tl::enums::Updates::deserialize(&mut Cursor::from_slice(body))
            {
                self.cache_users_and_chats(&u.users, &u.chats).await;
                for upd in &u.updates {
                    if let tl::enums::Update::NewMessage(nm) = upd {
                        return update::IncomingMessage::from_raw(nm.message.clone())
                            .with_client(self.clone());
                    }
                    if let tl::enums::Update::NewChannelMessage(nm) = upd {
                        return update::IncomingMessage::from_raw(nm.message.clone())
                            .with_client(self.clone());
                    }
                }
            }
        }

        // updateShortSentMessage#9015e101: server returns id/pts/date/media/entities
        // but not the full message body. Reconstruct from what we know.
        //
        // sent.media carries the real media for things sent to private chats
        // (dice, polls, photos, ...) - it must be threaded through, not dropped,
        // or callers get a synthetic message with media:None (e.g. send_dice()
        // would never be able to expose the rolled value).
        if cid == 0x9015e101 {
            let mut cur = Cursor::from_slice(&body[4..]);
            if let Ok(sent) = tl::types::UpdateShortSentMessage::deserialize(&mut cur) {
                let entities = sent.entities.clone().or_else(|| input.entities.clone());
                return self.synthetic_sent_from_short_ex(
                    input,
                    peer,
                    sent.id,
                    sent.date,
                    sent.media.clone(),
                    entities,
                );
            }
        }

        // updateShortMessage#313bc7f8 (DM to another user: we get a short form)
        if cid == 0x313bc7f8 {
            let mut cur = Cursor::from_slice(&body[4..]);
            if let Ok(m) = tl::types::UpdateShortMessage::deserialize(&mut cur) {
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
                    paid_suggested_post_stars: false,
                    paid_suggested_post_ton: false,
                    id: m.id,
                    from_id: Some(tl::enums::Peer::User(tl::types::PeerUser {
                        user_id: m.user_id,
                    })),
                    peer_id: tl::enums::Peer::User(tl::types::PeerUser { user_id: m.user_id }),
                    saved_peer_id: None,
                    fwd_from: m.fwd_from,
                    via_bot_id: m.via_bot_id,
                    via_business_bot_id: None,
                    guestchat_via_from: None,
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
                    schedule_repeat_period: None,
                    summary_from_language: None,
                    rich_message: None,
                };
                return update::IncomingMessage::from_raw(tl::enums::Message::Message(msg))
                    .with_client(self.clone());
            }
        }

        // Fallback: synthetic stub with no message ID known
        self.synthetic_sent_from_short(input, peer, 0, 0)
    }

    #[allow(dead_code)]
    pub(crate) async fn extract_sent_message(
        &self,
        sent: tl::types::UpdateShortSentMessage,

        input: &InputMessage,
        peer: &tl::enums::Peer,
    ) -> update::IncomingMessage {
        let msg = tl::types::Message {
            out: sent.out,
            mentioned: false,
            media_unread: false,
            silent: input.silent,
            post: false,
            from_scheduled: false,
            legacy: false,
            edit_hide: false,
            pinned: false,
            noforwards: false,
            invert_media: input.invert_media,
            offline: false,
            video_processing_pending: false,
            paid_suggested_post_stars: false,
            paid_suggested_post_ton: false,
            id: sent.id,
            from_id: None,
            from_boosts_applied: None,
            from_rank: None,
            peer_id: peer.clone(),
            saved_peer_id: None,
            fwd_from: None,
            via_bot_id: None,
            via_business_bot_id: None,
            guestchat_via_from: None,
            reply_to: input.reply_to.map(|id| {
                tl::enums::MessageReplyHeader::MessageReplyHeader(tl::types::MessageReplyHeader {
                    reply_to_scheduled: false,
                    forum_topic: false,
                    quote: false,
                    reply_to_msg_id: Some(id),
                    reply_to_peer_id: None,
                    reply_from: None,
                    reply_media: None,
                    reply_to_top_id: None,
                    quote_text: None,
                    quote_entities: None,
                    quote_offset: None,
                    todo_item_id: None,
                    poll_option: None,
                    reply_to_ephemeral: false,
                })
            }),
            date: sent.date,
            message: input.text.clone(),
            media: sent.media,
            reply_markup: input.reply_markup.clone(),
            entities: sent.entities,
            views: None,
            forwards: None,
            replies: None,
            edit_date: None,
            post_author: None,
            grouped_id: None,
            reactions: None,
            restriction_reason: None,
            ttl_period: sent.ttl_period,
            quick_reply_shortcut_id: None,
            effect: None,
            factcheck: None,
            report_delivery_until_date: None,
            paid_message_stars: None,
            suggested_post: None,
            schedule_repeat_period: None,
            summary_from_language: None,
            rich_message: None,
        };
        update::IncomingMessage::from_raw(tl::enums::Message::Message(msg))
            .with_client(self.clone())
    }

    fn synthetic_sent_from_short(
        &self,
        input: &InputMessage,

        peer: &tl::enums::Peer,
        id: i32,

        date: i32,
    ) -> update::IncomingMessage {
        self.synthetic_sent_from_short_ex(input, peer, id, date, None, input.entities.clone())
    }

    /// Like [`synthetic_sent_from_short`] but lets the caller supply the real
    /// `media` and `entities` returned by the server (e.g. from
    /// `updateShortSentMessage`) instead of always reconstructing from `input`.
    fn synthetic_sent_from_short_ex(
        &self,
        input: &InputMessage,
        peer: &tl::enums::Peer,
        id: i32,
        date: i32,
        media: Option<tl::enums::MessageMedia>,
        entities: Option<Vec<tl::enums::MessageEntity>>,
    ) -> update::IncomingMessage {
        let msg = tl::types::Message {
            out: true,
            mentioned: false,
            media_unread: false,
            silent: input.silent,
            post: false,
            from_scheduled: false,
            legacy: false,
            edit_hide: false,
            pinned: false,
            noforwards: false,
            invert_media: input.invert_media,
            offline: false,
            video_processing_pending: false,
            paid_suggested_post_stars: false,
            paid_suggested_post_ton: false,
            id,
            from_id: None,
            from_boosts_applied: None,
            from_rank: None,
            peer_id: peer.clone(),
            saved_peer_id: None,
            fwd_from: None,
            via_bot_id: None,
            via_business_bot_id: None,
            guestchat_via_from: None,
            reply_to: input.reply_to.map(|rid| {
                tl::enums::MessageReplyHeader::MessageReplyHeader(tl::types::MessageReplyHeader {
                    reply_to_scheduled: false,
                    forum_topic: false,
                    quote: false,
                    reply_to_msg_id: Some(rid),
                    reply_to_peer_id: None,
                    reply_from: None,
                    reply_media: None,
                    reply_to_top_id: None,
                    quote_text: None,
                    quote_entities: None,
                    quote_offset: None,
                    todo_item_id: None,
                    poll_option: None,
                    reply_to_ephemeral: false,
                })
            }),
            date,
            message: input.text.clone(),
            media,
            reply_markup: input.reply_markup.clone(),
            entities,
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
            schedule_repeat_period: None,
            summary_from_language: None,
            rich_message: None,
        };
        update::IncomingMessage::from_raw(tl::enums::Message::Message(msg))
            .with_client(self.clone())
    }

    /// Send a message to your own Saved Messages. A quick way to test that
    /// a connection works, or to leave yourself a note.
    pub async fn send_to_self(
        &self,
        msg: impl Into<InputMessage>,
    ) -> Result<update::IncomingMessage, InvocationError> {
        let msg = msg.into();
        let entities = self.resolve_outgoing_entities(msg.entities.clone()).await;
        let send_as = self.resolve_send_as(msg.send_as.clone()).await?;
        let req = tl::functions::messages::SendMessage {
            no_webpage: msg.no_webpage,
            silent: msg.silent,
            background: msg.background,
            clear_draft: msg.clear_draft,
            noforwards: msg.noforwards,
            update_stickersets_order: msg.update_stickersets_order,
            invert_media: msg.invert_media,
            allow_paid_floodskip: msg.allow_paid_floodskip,
            peer: tl::enums::InputPeer::PeerSelf,
            reply_to: msg.reply_header(),
            message: msg.text.clone(),
            random_id: random_i64(),
            reply_markup: msg.reply_markup.clone(),
            entities,
            schedule_date: msg.schedule_date,
            schedule_repeat_period: msg.schedule_repeat_period,
            send_as,
            quick_reply_shortcut: msg.quick_reply_shortcut(),
            effect: msg.effect,
            allow_paid_stars: msg.allow_paid_stars,
            suggested_post: msg.suggested_post.clone(),
            rich_message: msg.rich_message.clone(),
        };
        let body: Vec<u8> = self.rpc_call_raw(&req).await?;
        let self_peer = tl::enums::Peer::User(tl::types::PeerUser { user_id: 0 });
        Ok(self.parse_send_response(&body, &msg, &self_peer).await)
    }

    /// Edit the text of an existing message.
    pub async fn edit_message(
        &self,
        peer: impl Into<PeerRef>,
        message_id: i32,
        new_text: impl Into<InputMessage>,
    ) -> Result<(), InvocationError> {
        let peer = peer.into().resolve(self).await?;
        let input_peer = self.inner.peer_cache.read().await.peer_to_input(&peer)?;
        let msg = new_text.into();
        let req = tl::functions::messages::EditMessage {
            no_webpage: msg.no_webpage,
            invert_media: msg.invert_media,
            peer: input_peer,
            id: message_id,
            message: Some(msg.text),
            media: msg.media,
            reply_markup: msg.reply_markup,
            entities: msg.entities,
            schedule_date: msg.schedule_date,
            quick_reply_shortcut_id: msg.quick_reply_shortcut_id,
            schedule_repeat_period: msg.schedule_repeat_period,
            rich_message: msg.rich_message,
        };
        self.rpc_write(&req).await
    }

    /// Forward one or more messages from `source` to `destination`. Forwards
    /// keep the "Forwarded from" attribution by default; set `opts` to
    /// strip it, reply to an existing message in the destination, or land
    /// them in a specific forum topic via `opts.topic_id`.
    pub async fn forward_messages(
        &self,
        destination: impl Into<PeerRef>,

        message_ids: &[i32],
        source: impl Into<PeerRef>,

        opts: ForwardOptions,
    ) -> Result<Vec<update::IncomingMessage>, InvocationError> {
        let dest = destination.into().resolve(self).await?;
        let src = source.into().resolve(self).await?;
        let send_as_peer = match opts.send_as {
            Some(p) => Some(p.resolve(self).await?),
            None => None,
        };
        let cache: tokio::sync::RwLockReadGuard<'_, PeerCache> = self.inner.peer_cache.read().await;
        let to_peer = cache.peer_to_input(&dest)?;
        let from_peer = cache.peer_to_input(&src)?;
        let send_as = match send_as_peer {
            Some(p) => Some(cache.peer_to_input(&p)?),
            None => None,
        };
        drop(cache);

        let reply_to = opts.reply_to.map(|id| {
            tl::enums::InputReplyTo::Message(tl::types::InputReplyToMessage {
                reply_to_msg_id: id,
                top_msg_id: opts.topic_id,
                reply_to_peer_id: None,
                quote_text: None,
                quote_entities: None,
                quote_offset: None,
                monoforum_peer_id: None,
                poll_option: None,
                todo_item_id: None,
            })
        });

        let req = tl::functions::messages::ForwardMessages {
            silent: opts.silent,
            background: opts.background,
            with_my_score: opts.with_my_score,
            drop_author: opts.drop_author,
            drop_media_captions: opts.drop_media_captions,
            noforwards: opts.noforwards,
            from_peer,
            id: message_ids.to_vec(),
            random_id: (0..message_ids.len()).map(|_| random_i64()).collect(),
            to_peer,
            top_msg_id: opts.topic_id,
            reply_to,
            schedule_date: opts.schedule_date,
            schedule_repeat_period: opts.schedule_repeat_period,
            send_as,
            quick_reply_shortcut: opts.quick_reply_shortcut,
            effect: opts.effect,
            video_timestamp: opts.video_timestamp,
            allow_paid_stars: opts.allow_paid_stars,
            allow_paid_floodskip: opts.allow_paid_floodskip,
            suggested_post: opts.suggested_post,
        };
        let body: Vec<u8> = self.rpc_call_raw(&req).await?;
        // Parse the Updates container, cache peer info, and collect messages.
        let mut out = Vec::new();
        if body.len() >= 4 {
            let cid = u32::from_le_bytes(body[..4].try_into().unwrap());
            if cid == 0x74ae4240 || cid == 0x725b04c3 {
                let updates_opt = match tl::enums::Updates::from_bytes_exact(&body) {
                    Ok(updates) => Some(updates),
                    Err(e) => {
                        tracing::warn!(
                            "[ferogram::client] failed to deserialize update frame from server: {e}"
                        );
                        None
                    }
                };
                let (raw_updates, users, chats) = match updates_opt {
                    Some(tl::enums::Updates::Updates(u)) => (u.updates, u.users, u.chats),
                    Some(tl::enums::Updates::Combined(u)) => (u.updates, u.users, u.chats),
                    _ => (vec![], vec![], vec![]),
                };
                // Cache peers so returned IncomingMessage objects are immediately usable.
                self.cache_users_and_chats(&users, &chats).await;
                for upd in raw_updates {
                    match upd {
                        tl::enums::Update::NewMessage(u) => {
                            out.push(
                                update::IncomingMessage::from_raw(u.message)
                                    .with_client(self.clone()),
                            );
                        }
                        tl::enums::Update::NewChannelMessage(u) => {
                            out.push(
                                update::IncomingMessage::from_raw(u.message)
                                    .with_client(self.clone()),
                            );
                        }
                        _ => {}
                    }
                }
            }
        }
        Ok(out)
    }

    /// Copy one or more messages from `source` to `destination`, without the
    /// "Forwarded from" attribution. Handles text or media, single or
    /// multiple messages in one call - pass a one-element slice to copy just
    /// one.
    ///
    /// This is `forward_messages` with `drop_author` forced to `true`, which
    /// is exactly what Telegram's own "copy" feature does under the hood.
    pub async fn copy_messages(
        &self,
        destination: impl Into<PeerRef>,

        message_ids: &[i32],
        source: impl Into<PeerRef>,

        opts: CopyOptions,
    ) -> Result<Vec<update::IncomingMessage>, InvocationError> {
        self.forward_messages(destination, message_ids, source, opts.into())
            .await
    }

    #[allow(dead_code)]
    pub(crate) async fn delete_messages_raw(
        &self,
        message_ids: &[i32],
        revoke: bool,
    ) -> Result<(), InvocationError> {
        let req = tl::functions::messages::DeleteMessages {
            revoke,
            id: message_ids.to_vec(),
        };
        self.rpc_write(&req).await
    }

    /// Delete messages by ID from a regular chat or DM. Channel posts need
    /// channel-aware deletion instead, which isn't wired up on this path.
    /// With `revoke: true`, deletes for the other side too, not just you.
    pub async fn delete_messages(
        &self,
        message_ids: &[i32],
        revoke: bool,
    ) -> Result<(), InvocationError> {
        let req = tl::functions::messages::DeleteMessages {
            revoke,
            id: message_ids.to_vec(),
        };
        self.rpc_write(&req).await
    }

    /// Fetch a single message by ID.
    pub async fn get_messages(
        &self,
        peer: impl Into<PeerRef>,
        ids: &[i32],
    ) -> Result<Vec<update::IncomingMessage>, InvocationError> {
        let peer = peer.into().resolve(self).await?;
        let input_peer = self.inner.peer_cache.read().await.peer_to_input(&peer)?;
        let id_list: Vec<tl::enums::InputMessage> = ids
            .iter()
            .map(|&id| tl::enums::InputMessage::Id(tl::types::InputMessageId { id }))
            .collect();
        let body: Vec<u8> = match &input_peer {
            tl::enums::InputPeer::Channel(c) => {
                let req = tl::functions::channels::GetMessages {
                    channel: tl::enums::InputChannel::InputChannel(tl::types::InputChannel {
                        channel_id: c.channel_id,
                        access_hash: c.access_hash,
                    }),
                    id: id_list,
                };
                self.rpc_call_raw(&req).await?
            }
            _ => {
                let req = tl::functions::messages::GetMessages { id: id_list };
                self.rpc_call_raw(&req).await?
            }
        };
        let mut cur = Cursor::from_slice(&body);
        let msgs = match tl::enums::messages::Messages::deserialize(&mut cur)? {
            tl::enums::messages::Messages::Messages(m) => m.messages,
            tl::enums::messages::Messages::Slice(m) => m.messages,
            tl::enums::messages::Messages::ChannelMessages(m) => m.messages,
            tl::enums::messages::Messages::NotModified(_) => vec![],
        };
        Ok(msgs
            .into_iter()
            .map(|m| update::IncomingMessage::from_raw(m).with_client(self.clone()))
            .collect())
    }

    /// Get the currently pinned message in a chat, or `None` if nothing's
    /// pinned. If a chat has multiple pins, this returns only the latest one.
    pub async fn get_pinned_message(
        &self,
        peer: impl Into<PeerRef>,
    ) -> Result<Option<update::IncomingMessage>, InvocationError> {
        let peer = peer.into().resolve(self).await?;
        let input_peer = self.inner.peer_cache.read().await.peer_to_input(&peer)?;
        let req = tl::functions::messages::Search {
            peer: input_peer,
            q: String::new(),
            from_id: None,
            saved_peer_id: None,
            saved_reaction: None,
            top_msg_id: None,
            filter: tl::enums::MessagesFilter::InputMessagesFilterPinned,
            min_date: 0,
            max_date: 0,
            offset_id: 0,
            add_offset: 0,
            limit: 1,
            max_id: 0,
            min_id: 0,
            hash: 0,
        };
        let body: Vec<u8> = self.rpc_call_raw(&req).await?;
        let mut cur = Cursor::from_slice(&body);
        let msgs = match tl::enums::messages::Messages::deserialize(&mut cur)? {
            tl::enums::messages::Messages::Messages(m) => m.messages,
            tl::enums::messages::Messages::Slice(m) => m.messages,
            tl::enums::messages::Messages::ChannelMessages(m) => m.messages,
            tl::enums::messages::Messages::NotModified(_) => vec![],
        };
        Ok(msgs
            .into_iter()
            .next()
            .map(|m| update::IncomingMessage::from_raw(m).with_client(self.clone())))
    }

    /// Pin or unpin a message. `pin: true` pins, `pin: false` unpins.
    pub async fn pin_message(
        &self,
        peer: impl Into<PeerRef>,
        id: i32,
        pin: bool,
    ) -> Result<(), InvocationError> {
        self.update_pinned_message(peer, id, true, !pin, false)
            .await
    }

    pub(crate) async fn update_pinned_message(
        &self,
        peer: impl Into<PeerRef>,
        message_id: i32,
        silent: bool,
        unpin: bool,
        pm_oneside: bool,
    ) -> Result<(), InvocationError> {
        let peer = peer.into().resolve(self).await?;
        let input_peer = self.inner.peer_cache.read().await.peer_to_input(&peer)?;
        let req = tl::functions::messages::UpdatePinnedMessage {
            silent,
            unpin,
            pm_oneside,
            peer: input_peer,
            id: message_id,
        };
        self.rpc_write(&req).await
    }

    pub(crate) async fn pin_message_raw(
        &self,
        peer: impl Into<PeerRef>,
        message_id: i32,
    ) -> Result<(), InvocationError> {
        self.update_pinned_message(peer, message_id, true, false, false)
            .await
    }

    /// Fetch the message that `message` is replying to.
    ///
    /// Returns `None` if the message is not a reply, or if the original
    /// message could not be found (deleted / inaccessible).
    ///
    /// # Example
    /// ```rust,ignore
    /// # async fn f(client: ferogram::Client, msg: ferogram::update::IncomingMessage)
    /// #   -> Result<(), ferogram::InvocationError> {
    /// if let Some(replied) = client.get_reply_to_message(&msg).await? {
    /// println!("Replied to: {:?}", replied.text());
    /// }
    /// # Ok(()) }
    /// ```
    pub(crate) async fn get_reply_to_message(
        &self,
        message: &update::IncomingMessage,
    ) -> Result<Option<update::IncomingMessage>, InvocationError> {
        let reply_id = match message.reply_to_message_id() {
            Some(id) => id,
            None => return Ok(None),
        };
        let peer = match message.peer_id() {
            Some(p) => p.clone(),
            None => return Ok(None),
        };
        let input_peer = self.inner.peer_cache.read().await.peer_to_input(&peer)?;
        let id = vec![tl::enums::InputMessage::Id(tl::types::InputMessageId {
            id: reply_id,
        })];

        let result = match &input_peer {
            tl::enums::InputPeer::Channel(c) => {
                let req = tl::functions::channels::GetMessages {
                    channel: tl::enums::InputChannel::InputChannel(tl::types::InputChannel {
                        channel_id: c.channel_id,
                        access_hash: c.access_hash,
                    }),
                    id,
                };
                self.rpc_call_raw(&req).await?
            }
            _ => {
                let req = tl::functions::messages::GetMessages { id };
                self.rpc_call_raw(&req).await?
            }
        };

        let mut cur = Cursor::from_slice(&result);
        let msgs = match tl::enums::messages::Messages::deserialize(&mut cur)? {
            tl::enums::messages::Messages::Messages(m) => m.messages,
            tl::enums::messages::Messages::Slice(m) => m.messages,
            tl::enums::messages::Messages::ChannelMessages(m) => m.messages,
            tl::enums::messages::Messages::NotModified(_) => vec![],
        };
        Ok(msgs
            .into_iter()
            .next()
            .map(|m| update::IncomingMessage::from_raw(m).with_client(self.clone())))
    }

    /// Unpin every pinned message in a chat at once.
    pub async fn unpin_all_messages(
        &self,
        peer: impl Into<PeerRef>,
    ) -> Result<(), InvocationError> {
        let peer = peer.into().resolve(self).await?;
        let input_peer = self.inner.peer_cache.read().await.peer_to_input(&peer)?;
        let req = tl::functions::messages::UnpinAllMessages {
            peer: input_peer,
            top_msg_id: None,
            saved_peer_id: None,
        };
        self.rpc_write(&req).await
    }

    /// List a chat's scheduled messages - the ones queued to send later,
    /// not sent yet.
    pub async fn get_scheduled_messages(
        &self,
        peer: impl Into<PeerRef>,
    ) -> Result<Vec<update::IncomingMessage>, InvocationError> {
        let peer = peer.into().resolve(self).await?;
        let input_peer = self.inner.peer_cache.read().await.peer_to_input(&peer)?;
        let req = tl::functions::messages::GetScheduledHistory {
            peer: input_peer,
            hash: 0,
        };
        let body = self.rpc_call_raw(&req).await?;
        let mut cur = Cursor::from_slice(&body);
        let msgs = match tl::enums::messages::Messages::deserialize(&mut cur)? {
            tl::enums::messages::Messages::Messages(m) => m.messages,
            tl::enums::messages::Messages::Slice(m) => m.messages,
            tl::enums::messages::Messages::ChannelMessages(m) => m.messages,
            tl::enums::messages::Messages::NotModified(_) => vec![],
        };
        Ok(msgs
            .into_iter()
            .map(|m| update::IncomingMessage::from_raw(m).with_client(self.clone()))
            .collect())
    }

    /// Cancel scheduled messages before they send.
    pub async fn delete_scheduled_messages(
        &self,
        peer: impl Into<PeerRef>,
        ids: &[i32],
    ) -> Result<(), InvocationError> {
        let peer = peer.into().resolve(self).await?;
        let input_peer = self.inner.peer_cache.read().await.peer_to_input(&peer)?;
        let req = tl::functions::messages::DeleteScheduledMessages {
            peer: input_peer,
            id: ids.to_vec(),
        };
        self.rpc_write(&req).await
    }

    /// Fetch message history for a peer, newest first.
    ///
    /// recent message). `add_offset` additionally skips this many messages
    /// past that anchor, which is what makes simple limit-based pagination
    /// possible without having to track the exact last-seen message ID
    /// yourself: e.g. `get_message_history(peer, 20, 0, 0)` for the first
    /// page, then `get_message_history(peer, 20, 0, 20)` for the second,
    /// `add_offset: 40` for the third, and so on. The server still has to
    /// walk past the skipped messages internally, so this does not scale
    /// well to very deep pagination - prefer [`Client::iter_messages`] for
    /// walking large histories.
    ///
    /// Returns a [`crate::types::MessagePage`], which also carries `count`
    /// and `offset_id_offset` from the raw response - forward these to a
    /// caller of your own (bot, UI, etc.) so *they* can ask for the next
    /// page without you having to re-implement this RPC call by hand.
    pub async fn get_message_history(
        &self,
        peer: impl Into<PeerRef>,
        limit: i32,
        offset_id: i32,
        add_offset: i32,
    ) -> Result<crate::types::MessagePage, InvocationError> {
        let peer = peer.into().resolve(self).await?;
        let input_peer = self.inner.peer_cache.read().await.peer_to_input(&peer)?;
        let req = tl::functions::messages::GetHistory {
            peer: input_peer,
            offset_id,
            offset_date: 0,
            add_offset,
            limit,
            max_id: 0,
            min_id: 0,
            hash: 0,
        };
        let body = self.rpc_call_raw(&req).await?;
        let mut cur = Cursor::from_slice(&body);
        let (msgs, count, offset_id_offset, users, chats) =
            match tl::enums::messages::Messages::deserialize(&mut cur)? {
                tl::enums::messages::Messages::Messages(m) => {
                    (m.messages, None, None, m.users, m.chats)
                }
                tl::enums::messages::Messages::Slice(m) => (
                    m.messages,
                    Some(m.count),
                    m.offset_id_offset,
                    m.users,
                    m.chats,
                ),
                tl::enums::messages::Messages::ChannelMessages(m) => (
                    m.messages,
                    Some(m.count),
                    m.offset_id_offset,
                    m.users,
                    m.chats,
                ),
                tl::enums::messages::Messages::NotModified(_) => {
                    (vec![], None, None, vec![], vec![])
                }
            };
        self.cache_users_slice(&users).await;
        self.cache_chats_slice(&chats).await;
        Ok(crate::types::MessagePage {
            messages: msgs
                .into_iter()
                .map(|m| update::IncomingMessage::from_raw(m).with_client(self.clone()))
                .collect(),
            count,
            offset_id_offset,
        })
    }

    /// Show a chat action like "typing..." in a chat. Telegram clears it
    /// automatically after a few seconds, so call this again if the action
    /// is still ongoing.
    pub async fn send_chat_action(
        &self,
        peer: impl Into<PeerRef>,

        action: tl::enums::SendMessageAction,
    ) -> Result<(), InvocationError> {
        let peer = peer.into().resolve(self).await?;
        self.send_chat_action_ex(peer, action, None).await
    }

    /// Send a dice/dart/basketball/etc animated emoji and return the sent message.
    ///
    /// The rolled value is in the returned message's media:
    ///
    /// ```rust,no_run
    /// # use ferogram::{Client, tl};
    /// # async fn example(client: Client) -> anyhow::Result<()> {
    /// let msg = client.send_dice(123456789, "🎲").await?;
    /// if let Some(tl::enums::MessageMedia::Dice(d)) = msg.media() {
    ///     println!("rolled {}", d.value);
    /// }
    /// # Ok(()) }
    /// ```
    pub async fn send_dice(
        &self,
        peer: impl Into<PeerRef>,
        emoticon: impl Into<String>,
    ) -> Result<update::IncomingMessage, InvocationError> {
        use ferogram_tl_types as tl;
        let peer = peer.into().resolve(self).await?;
        let input_peer = self.inner.peer_cache.read().await.peer_to_input(&peer)?;
        let media = tl::enums::InputMedia::Dice(tl::types::InputMediaDice {
            emoticon: emoticon.into(),
        });
        let req = tl::functions::messages::SendMedia {
            silent: false,
            background: false,
            clear_draft: false,
            noforwards: false,
            update_stickersets_order: false,
            invert_media: false,
            allow_paid_floodskip: false,
            peer: input_peer,
            reply_to: None,
            media,
            message: String::new(),
            random_id: random_i64(),
            reply_markup: None,
            entities: None,
            schedule_date: None,
            schedule_repeat_period: None,
            send_as: None,
            quick_reply_shortcut: None,
            effect: None,
            allow_paid_stars: None,
            suggested_post: None,
        };
        let body: Vec<u8> = self.rpc_call_raw(&req).await?;
        Ok(self
            .parse_send_response(&body, &InputMessage::text(""), &peer)
            .await)
    }

    pub(crate) async fn get_messages_with_count(
        &self,
        peer: impl Into<PeerRef>,
        limit: i32,
        offset_id: i32,
    ) -> Result<(Vec<crate::update::IncomingMessage>, Option<i32>), InvocationError> {
        let peer = peer.into().resolve(self).await?;
        let input_peer = self.inner.peer_cache.read().await.peer_to_input(&peer)?;
        let req = tl::functions::messages::GetHistory {
            peer: input_peer,
            offset_id,
            offset_date: 0,
            add_offset: 0,
            limit,
            max_id: 0,
            min_id: 0,
            hash: 0,
        };
        let body: Vec<u8> = self.rpc_call_raw(&req).await?;
        let mut cur = Cursor::from_slice(&body);
        let raw = tl::enums::messages::Messages::deserialize(&mut cur)?;
        let (msgs, users, chats, count) = match raw {
            tl::enums::messages::Messages::Messages(m) => (m.messages, m.users, m.chats, None),
            tl::enums::messages::Messages::Slice(m) => {
                (m.messages, m.users, m.chats, Some(m.count))
            }
            tl::enums::messages::Messages::ChannelMessages(m) => {
                (m.messages, m.users, m.chats, None)
            }
            tl::enums::messages::Messages::NotModified(_) => return Ok((vec![], None)),
        };
        self.cache_users_and_chats(&users, &chats).await;
        let out = msgs
            .into_iter()
            .map(|m| crate::update::IncomingMessage::from_raw(m).with_client(self.clone()))
            .collect();
        Ok((out, count))
    }

    /// Save text as the draft for a chat, the way an unsent typed message
    /// shows up when you reopen it.
    pub async fn save_draft(
        &self,
        peer: impl Into<PeerRef>,
        text: impl Into<String>,
    ) -> Result<(), InvocationError> {
        let peer = peer.into().resolve(self).await?;
        let input_peer = self.inner.peer_cache.read().await.peer_to_input(&peer)?;
        let req = tl::functions::messages::SaveDraft {
            no_webpage: false,
            invert_media: false,
            reply_to: None,
            peer: input_peer,
            message: text.into(),
            entities: None,
            media: None,
            effect: None,
            suggested_post: None,
            rich_message: None,
        };
        self.rpc_write(&req).await
    }

    /// Send scheduled messages immediately instead of waiting for their
    /// scheduled time.
    pub async fn send_scheduled_now(
        &self,
        peer: impl Into<PeerRef>,
        ids: &[i32],
    ) -> Result<(), InvocationError> {
        let peer = peer.into().resolve(self).await?;
        let input_peer = self.inner.peer_cache.read().await.peer_to_input(&peer)?;
        let req = tl::functions::messages::SendScheduledMessages {
            peer: input_peer,
            id: ids.to_vec(),
        };
        self.rpc_write(&req).await
    }

    /// List who has read a message and when, in a small group. Telegram
    /// doesn't track this for everything - large chats or old enough
    /// messages just come back empty.
    pub async fn get_message_read_participants(
        &self,
        peer: impl Into<PeerRef>,
        msg_id: i32,
    ) -> Result<Vec<tl::types::ReadParticipantDate>, InvocationError> {
        let peer = peer.into().resolve(self).await?;
        let input_peer = self.inner.peer_cache.read().await.peer_to_input(&peer)?;
        let req = tl::functions::messages::GetMessageReadParticipants {
            peer: input_peer,
            msg_id,
        };
        let body = self.rpc_call_raw(&req).await?;
        let mut cur = Cursor::from_slice(&body);
        Ok(
            Vec::<tl::enums::ReadParticipantDate>::deserialize(&mut cur)?
                .into_iter()
                .map(|r| match r {
                    tl::enums::ReadParticipantDate::ReadParticipantDate(d) => d,
                })
                .collect(),
        )
    }

    /// Fetch replies in a discussion thread. Same `add_offset` pagination
    /// trick as [`Client::get_message_history`]: skip this many messages
    /// past `offset_id` (which can stay 0) to jump straight to a given page.
    ///
    /// Returns a [`crate::types::MessagePage`] carrying `count` and
    /// `offset_id_offset` alongside the messages, same as
    /// [`Client::get_message_history`].
    pub async fn get_replies(
        &self,
        peer: impl Into<PeerRef>,
        msg_id: i32,
        limit: i32,
        offset_id: i32,
        add_offset: i32,
    ) -> Result<crate::types::MessagePage, InvocationError> {
        let peer = peer.into().resolve(self).await?;
        let input_peer = self.inner.peer_cache.read().await.peer_to_input(&peer)?;
        let req = tl::functions::messages::GetReplies {
            peer: input_peer,
            msg_id,
            offset_id,
            offset_date: 0,
            add_offset,
            limit,
            max_id: 0,
            min_id: 0,
            hash: 0,
        };
        let body = self.rpc_call_raw(&req).await?;
        let mut cur = Cursor::from_slice(&body);
        let (msgs, count, offset_id_offset, users, chats) =
            match tl::enums::messages::Messages::deserialize(&mut cur)? {
                tl::enums::messages::Messages::Messages(m) => {
                    (m.messages, None, None, m.users, m.chats)
                }
                tl::enums::messages::Messages::Slice(m) => (
                    m.messages,
                    Some(m.count),
                    m.offset_id_offset,
                    m.users,
                    m.chats,
                ),
                tl::enums::messages::Messages::ChannelMessages(m) => (
                    m.messages,
                    Some(m.count),
                    m.offset_id_offset,
                    m.users,
                    m.chats,
                ),
                tl::enums::messages::Messages::NotModified(_) => {
                    (vec![], None, None, vec![], vec![])
                }
            };
        self.cache_users_slice(&users).await;
        self.cache_chats_slice(&chats).await;
        Ok(crate::types::MessagePage {
            messages: msgs
                .into_iter()
                .map(|m| update::IncomingMessage::from_raw(m).with_client(self.clone()))
                .collect(),
            count,
            offset_id_offset,
        })
    }

    /// Get the discussion-group message linked to a channel post, for
    /// channels that have comments enabled.
    pub async fn get_discussion_message(
        &self,
        peer: impl Into<PeerRef>,
        msg_id: i32,
    ) -> Result<tl::types::messages::DiscussionMessage, InvocationError> {
        let peer = peer.into().resolve(self).await?;
        let input_peer = self.inner.peer_cache.read().await.peer_to_input(&peer)?;
        let req = tl::functions::messages::GetDiscussionMessage {
            peer: input_peer,
            msg_id,
        };
        let body = self.rpc_call_raw(&req).await?;
        let mut cur = Cursor::from_slice(&body);
        let tl::enums::messages::DiscussionMessage::DiscussionMessage(result) =
            tl::enums::messages::DiscussionMessage::deserialize(&mut cur)?;
        self.cache_users_slice(&result.users).await;
        self.cache_chats_slice(&result.chats).await;
        Ok(result)
    }

    /// Mark a discussion thread as read up to `read_max_id`.
    pub async fn read_discussion(
        &self,
        peer: impl Into<PeerRef>,
        msg_id: i32,
        read_max_id: i32,
    ) -> Result<(), InvocationError> {
        let peer = peer.into().resolve(self).await?;
        let input_peer = self.inner.peer_cache.read().await.peer_to_input(&peer)?;
        let req = tl::functions::messages::ReadDiscussion {
            peer: input_peer,
            msg_id,
            read_max_id,
        };
        self.rpc_write(&req).await
    }

    /// Generate a link preview for `text` without sending it anywhere -
    /// handy for showing a preview before the user actually sends.
    pub async fn get_web_page_preview(
        &self,
        text: impl Into<String>,
    ) -> Result<tl::enums::MessageMedia, InvocationError> {
        let req = tl::functions::messages::GetWebPagePreview {
            message: text.into(),
            entities: None,
        };
        let body = self.rpc_call_raw(&req).await?;
        let mut cur = Cursor::from_slice(&body);
        let tl::enums::messages::WebPagePreview::WebPagePreview(result) =
            tl::enums::messages::WebPagePreview::deserialize(&mut cur)?;
        Ok(result.media)
    }

    /// Translate one or more existing messages to `to_lang` (an ISO language
    /// code like `"en"`).
    pub async fn translate_messages(
        &self,
        peer: impl Into<PeerRef>,
        msg_ids: Vec<i32>,
        to_lang: impl Into<String>,
    ) -> Result<Vec<tl::types::TextWithEntities>, InvocationError> {
        let peer = peer.into().resolve(self).await?;
        let input_peer = self.inner.peer_cache.read().await.peer_to_input(&peer)?;
        let req = tl::functions::messages::TranslateText {
            peer: Some(input_peer),
            id: Some(msg_ids),
            text: None,
            to_lang: to_lang.into(),
            tone: None,
        };
        let body = self.rpc_call_raw(&req).await?;
        let mut cur = Cursor::from_slice(&body);
        let tl::enums::messages::TranslatedText::TranslateResult(result) =
            tl::enums::messages::TranslatedText::deserialize(&mut cur)?;
        Ok(result
            .result
            .into_iter()
            .map(|x| {
                let tl::enums::TextWithEntities::TextWithEntities(t) = x;
                t
            })
            .collect())
    }

    /// Transcribe a voice message to text.
    pub async fn transcribe_audio(
        &self,
        peer: impl Into<PeerRef>,
        msg_id: i32,
    ) -> Result<tl::types::messages::TranscribedAudio, InvocationError> {
        let peer = peer.into().resolve(self).await?;
        let input_peer = self.inner.peer_cache.read().await.peer_to_input(&peer)?;
        let req = tl::functions::messages::TranscribeAudio {
            peer: input_peer,
            msg_id,
        };
        let body = self.rpc_call_raw(&req).await?;
        let mut cur = Cursor::from_slice(&body);
        let tl::enums::messages::TranscribedAudio::TranscribedAudio(result) =
            tl::enums::messages::TranscribedAudio::deserialize(&mut cur)?;
        Ok(result)
    }

    /// Turn the "Translate" button on or off for a chat.
    pub async fn toggle_peer_translations(
        &self,
        peer: impl Into<PeerRef>,
        disabled: bool,
    ) -> Result<(), InvocationError> {
        let peer = peer.into().resolve(self).await?;
        let input_peer = self.inner.peer_cache.read().await.peer_to_input(&peer)?;
        let req = tl::functions::messages::TogglePeerTranslations {
            disabled,
            peer: input_peer,
        };
        self.rpc_write(&req).await
    }

    /// Get a `t.me` link to a specific message. `kind` picks whether it
    /// points at the single message, its whole media group, or its comment
    /// thread.
    pub async fn export_message_link(
        &self,
        peer: impl Into<PeerRef>,
        msg_id: i32,
        kind: LinkKind,
    ) -> Result<String, InvocationError> {
        let (grouped, thread) = match kind {
            LinkKind::Normal => (false, false),
            LinkKind::Grouped => (true, false),
            LinkKind::Thread => (false, true),
        };
        self.export_message_link_raw(peer, msg_id, grouped, thread)
            .await
    }

    pub(crate) async fn export_message_link_raw(
        &self,
        peer: impl Into<PeerRef>,
        msg_id: i32,
        grouped: bool,
        thread: bool,
    ) -> Result<String, InvocationError> {
        let peer = peer.into().resolve(self).await?;
        let input_peer = self.inner.peer_cache.read().await.peer_to_input(&peer)?;
        let channel = match input_peer {
            tl::enums::InputPeer::Channel(c) => {
                tl::enums::InputChannel::InputChannel(tl::types::InputChannel {
                    channel_id: c.channel_id,
                    access_hash: c.access_hash,
                })
            }
            _ => {
                return Err(InvocationError::Deserialize(
                    "export_message_link requires a channel".into(),
                ));
            }
        };
        let req = tl::functions::channels::ExportMessageLink {
            grouped,
            thread,
            channel,
            id: msg_id,
        };
        let body: Vec<u8> = self.rpc_call_raw(&req).await?;
        let mut cur = Cursor::from_slice(&body);
        let link = tl::enums::ExportedMessageLink::deserialize(&mut cur)?;
        match link {
            tl::enums::ExportedMessageLink::ExportedMessageLink(l) => Ok(l.link),
        }
    }
}
