// Copyright (c) Ankit Chaubey <ankitchaubey.dev@gmail.com>
// SPDX-License-Identifier: MIT OR Apache-2.0
//
// ferogram: async Telegram MTProto client in Rust
// https://github.com/ankit-chaubey/ferogram
//
// If you use or modify this code, keep this notice at the top of your file
// and include the LICENSE-MIT or LICENSE-APACHE file from this repository:
// https://github.com/ankit-chaubey/ferogram

// so that the MessageBoxes state-machine has one uniform input type.

use ferogram_tl_types as tl;

use super::defs::{Gap, Key, NO_PTS, NO_SEQ, PtsInfo, UpdatesLike};

// Builders

fn wrap_updates(updates: tl::types::Updates) -> tl::types::UpdatesCombined {
    tl::types::UpdatesCombined {
        updates: updates.updates,
        users: updates.users,
        chats: updates.chats,
        date: updates.date,
        seq_start: updates.seq,
        seq: updates.seq,
    }
}

fn wrap_short(short: tl::types::UpdateShort) -> tl::types::UpdatesCombined {
    tl::types::UpdatesCombined {
        updates: vec![short.update],
        users: Vec::new(),
        chats: Vec::new(),
        date: short.date,
        seq_start: NO_SEQ,
        seq: NO_SEQ,
    }
}

fn short_message_to_combined(short: tl::types::UpdateShortMessage) -> tl::types::UpdatesCombined {
    wrap_short(tl::types::UpdateShort {
        update: tl::types::UpdateNewMessage {
            message: tl::types::Message {
                out: short.out,
                mentioned: short.mentioned,
                media_unread: short.media_unread,
                silent: short.silent,
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
                reactions: None,
                id: short.id,
                from_id: Some(tl::enums::Peer::User(tl::types::PeerUser {
                    user_id: short.user_id,
                })),
                from_rank: None,
                from_boosts_applied: None,
                peer_id: tl::enums::Peer::User(tl::types::PeerUser {
                    user_id: short.user_id,
                }),
                saved_peer_id: None,
                fwd_from: short.fwd_from,
                via_bot_id: short.via_bot_id,
                via_business_bot_id: None,
                reply_to: short.reply_to,
                date: short.date,
                message: short.message,
                media: None,
                reply_markup: None,
                entities: short.entities,
                views: None,
                forwards: None,
                replies: None,
                edit_date: None,
                post_author: None,
                grouped_id: None,
                restriction_reason: None,
                ttl_period: short.ttl_period,
                quick_reply_shortcut_id: None,
                effect: None,
                factcheck: None,
                report_delivery_until_date: None,
                paid_message_stars: None,
                suggested_post: None,
                schedule_repeat_period: None,
                summary_from_language: None,
            }
            .into(),
            pts: short.pts,
            pts_count: short.pts_count,
        }
        .into(),
        date: short.date,
    })
}

fn short_chat_message_to_combined(
    short: tl::types::UpdateShortChatMessage,
) -> tl::types::UpdatesCombined {
    wrap_short(tl::types::UpdateShort {
        update: tl::types::UpdateNewMessage {
            message: tl::types::Message {
                out: short.out,
                mentioned: short.mentioned,
                media_unread: short.media_unread,
                silent: short.silent,
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
                reactions: None,
                id: short.id,
                from_id: Some(tl::enums::Peer::User(tl::types::PeerUser {
                    user_id: short.from_id,
                })),
                from_rank: None,
                from_boosts_applied: None,
                peer_id: tl::enums::Peer::Chat(tl::types::PeerChat {
                    chat_id: short.chat_id,
                }),
                saved_peer_id: None,
                fwd_from: short.fwd_from,
                via_bot_id: short.via_bot_id,
                via_business_bot_id: None,
                reply_to: short.reply_to,
                date: short.date,
                message: short.message,
                media: None,
                reply_markup: None,
                entities: short.entities,
                views: None,
                forwards: None,
                replies: None,
                edit_date: None,
                post_author: None,
                grouped_id: None,
                restriction_reason: None,
                ttl_period: short.ttl_period,
                quick_reply_shortcut_id: None,
                effect: None,
                factcheck: None,
                report_delivery_until_date: None,
                paid_message_stars: None,
                suggested_post: None,
                schedule_repeat_period: None,
                summary_from_language: None,
            }
            .into(),
            pts: short.pts,
            pts_count: short.pts_count,
        }
        .into(),
        date: short.date,
    })
}

fn short_sent_message_to_combined(
    short: tl::types::UpdateShortSentMessage,
) -> tl::types::UpdatesCombined {
    wrap_short(tl::types::UpdateShort {
        update: tl::types::UpdateNewMessage {
            message: tl::types::MessageEmpty {
                id: short.id,
                peer_id: None,
            }
            .into(),
            pts: short.pts,
            pts_count: short.pts_count,
        }
        .into(),
        date: short.date,
    })
}

// AffectedMessages helper

fn affected_messages_to_combined(
    affected: tl::types::messages::AffectedMessages,
) -> tl::types::UpdatesCombined {
    wrap_short(tl::types::UpdateShort {
        update: tl::types::UpdateDeleteMessages {
            messages: Vec::new(),
            pts: affected.pts,
            pts_count: affected.pts_count,
        }
        .into(),
        date: 0,
    })
}

fn affected_channel_messages_to_combined(
    affected: tl::types::messages::AffectedMessages,
    channel_id: i64,
) -> tl::types::UpdatesCombined {
    wrap_short(tl::types::UpdateShort {
        update: tl::types::UpdateDeleteChannelMessages {
            channel_id,
            messages: Vec::new(),
            pts: affected.pts,
            pts_count: affected.pts_count,
        }
        .into(),
        date: 0,
    })
}

// Public entry points

pub(super) fn adapt(updates: UpdatesLike) -> Result<tl::types::UpdatesCombined, Gap> {
    match updates {
        UpdatesLike::Updates(u) => adapt_updates(*u),
        UpdatesLike::ConnectionClosed | UpdatesLike::MalformedUpdates => Err(Gap),
        UpdatesLike::AffectedMessages(affected) => Ok(affected_messages_to_combined(affected)),
        UpdatesLike::AffectedChannelMessages {
            affected,
            channel_id,
        } => Ok(affected_channel_messages_to_combined(affected, channel_id)),
    }
}

fn adapt_updates(updates: tl::enums::Updates) -> Result<tl::types::UpdatesCombined, Gap> {
    Ok(match updates {
        // updatesTooLong → gap; must getDifference.
        tl::enums::Updates::TooLong => {
            tracing::info!("[ferogram/msgbox] updatesTooLong → gap");
            return Err(Gap);
        }
        tl::enums::Updates::UpdateShortMessage(s) => short_message_to_combined(s),
        tl::enums::Updates::UpdateShortChatMessage(s) => short_chat_message_to_combined(s),
        tl::enums::Updates::UpdateShort(s) => wrap_short(s),
        tl::enums::Updates::Combined(c) => c,
        tl::enums::Updates::Updates(u) => wrap_updates(u),
        tl::enums::Updates::UpdateShortSentMessage(s) => short_sent_message_to_combined(s),
    })
}

// Channel difference flattening

pub(super) fn adapt_channel_difference(
    diff: tl::enums::updates::ChannelDifference,
) -> tl::types::updates::ChannelDifference {
    match diff {
        tl::enums::updates::ChannelDifference::Empty(e) => tl::types::updates::ChannelDifference {
            r#final: e.r#final,
            pts: e.pts,
            timeout: e.timeout,
            new_messages: Vec::new(),
            other_updates: Vec::new(),
            chats: Vec::new(),
            users: Vec::new(),
        },
        tl::enums::updates::ChannelDifference::TooLong(t) => {
            let pts = match t.dialog {
                tl::enums::Dialog::Dialog(d) => {
                    d.pts.expect("channelDifferenceTooLong: dialog had no pts")
                }
                tl::enums::Dialog::Folder(_) => {
                    panic!("channelDifferenceTooLong: unexpected folder dialog");
                }
            };
            tl::types::updates::ChannelDifference {
                r#final: t.r#final,
                pts,
                timeout: t.timeout,
                new_messages: Vec::new(),
                other_updates: Vec::new(),
                chats: t.chats,
                users: t.users,
            }
        }
        tl::enums::updates::ChannelDifference::ChannelDifference(d) => d,
    }
}

// PtsInfo::from_update – extract pts/count/key from a single tl::enums::Update

fn message_channel_id(message: &tl::enums::Message) -> Option<i64> {
    match message {
        tl::enums::Message::Message(m) => match &m.peer_id {
            tl::enums::Peer::Channel(c) => Some(c.channel_id),
            _ => None,
        },
        tl::enums::Message::Service(m) => match &m.peer_id {
            tl::enums::Peer::Channel(c) => Some(c.channel_id),
            _ => None,
        },
        tl::enums::Message::Empty(_) => None,
    }
}

impl PtsInfo {
    pub(super) fn from_update(update: &tl::enums::Update) -> Option<Self> {
        use tl::enums::Update::*;
        let info = match update {
            NewMessage(u) => Self {
                key: Key::Common,
                pts: u.pts,
                count: u.pts_count,
            },
            DeleteMessages(u) => Self {
                key: Key::Common,
                pts: u.pts,
                count: u.pts_count,
            },
            ReadHistoryInbox(u) => Self {
                key: Key::Common,
                pts: u.pts,
                count: u.pts_count,
            },
            ReadHistoryOutbox(u) => Self {
                key: Key::Common,
                pts: u.pts,
                count: u.pts_count,
            },
            WebPage(u) => Self {
                key: Key::Common,
                pts: u.pts,
                count: u.pts_count,
            },
            ReadMessagesContents(u) => Self {
                key: Key::Common,
                pts: u.pts,
                count: u.pts_count,
            },
            EditMessage(u) => Self {
                key: Key::Common,
                pts: u.pts,
                count: u.pts_count,
            },
            FolderPeers(u) => Self {
                key: Key::Common,
                pts: u.pts,
                count: u.pts_count,
            },
            PinnedMessages(u) => Self {
                key: Key::Common,
                pts: u.pts,
                count: u.pts_count,
            },
            // Channel-scoped updates
            NewChannelMessage(u) => {
                let channel_id = message_channel_id(&u.message)?;
                Self {
                    key: Key::Channel(channel_id),
                    pts: u.pts,
                    count: u.pts_count,
                }
            }
            EditChannelMessage(u) => {
                let channel_id = message_channel_id(&u.message)?;
                Self {
                    key: Key::Channel(channel_id),
                    pts: u.pts,
                    count: u.pts_count,
                }
            }
            DeleteChannelMessages(u) => Self {
                key: Key::Channel(u.channel_id),
                pts: u.pts,
                count: u.pts_count,
            },
            ReadChannelInbox(u) => Self {
                key: Key::Channel(u.channel_id),
                pts: u.pts,
                count: 0,
            },
            ChannelWebPage(u) => Self {
                key: Key::Channel(u.channel_id),
                pts: u.pts,
                count: u.pts_count,
            },
            PinnedChannelMessages(u) => Self {
                key: Key::Channel(u.channel_id),
                pts: u.pts,
                count: u.pts_count,
            },
            // ChannelTooLong is handled specially in process_updates (begin_get_diff).
            ChannelTooLong(u) => u.pts.map(|pts| Self {
                key: Key::Channel(u.channel_id),
                pts,
                count: 0,
            })?,
            // Secondary (qts-based) updates
            NewEncryptedMessage(u) => Self {
                key: Key::Secondary,
                pts: u.qts,
                count: 1,
            },
            ChatParticipant(u) => Self {
                key: Key::Secondary,
                pts: u.qts,
                count: 1,
            },
            ChannelParticipant(u) => Self {
                key: Key::Secondary,
                pts: u.qts,
                count: 1,
            },
            BotStopped(u) => Self {
                key: Key::Secondary,
                pts: u.qts,
                count: 1,
            },
            BotChatInviteRequester(u) => Self {
                key: Key::Secondary,
                pts: u.qts,
                count: 1,
            },
            BotChatBoost(u) => Self {
                key: Key::Secondary,
                pts: u.qts,
                count: 1,
            },
            BotMessageReaction(u) => Self {
                key: Key::Secondary,
                pts: u.qts,
                count: 1,
            },
            MessagePollVote(u) => Self {
                key: Key::Secondary,
                pts: u.qts,
                count: 1,
            },
            // All other updates have no pts.
            _ => return None,
        };
        // Filter out the NO_PTS sentinel.
        if info.pts == NO_PTS { None } else { Some(info) }
    }
}
