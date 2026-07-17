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

/// Builder for composing outgoing messages.
///
/// ```rust,no_run
/// use ferogram::InputMessage;
///
/// // plain text
/// let msg = InputMessage::text("Hello!");
///
/// // markdown
/// let msg = InputMessage::markdown("**bold** and _italic_");
///
/// // HTML
/// let msg = InputMessage::html("<b>bold</b> and <i>italic</i>");
///
/// // with options
/// let msg = InputMessage::markdown("**Hello**")
///     .silent(true)
///     .reply_to(Some(42));
/// ```
#[derive(Clone, Default)]
pub struct InputMessage {
    pub text: String,
    pub reply_to: Option<i32>,
    pub silent: bool,
    pub background: bool,
    pub clear_draft: bool,
    pub no_webpage: bool,
    /// Show media above the caption instead of below (Telegram ≥ 10.3).\
    pub invert_media: bool,
    /// Schedule to send when the user goes online (`schedule_date = 0x7FFFFFFE`).\
    pub schedule_once_online: bool,
    pub entities: Option<Vec<tl::enums::MessageEntity>>,
    pub reply_markup: Option<tl::enums::ReplyMarkup>,
    pub schedule_date: Option<i32>,
    /// Repeat the scheduled send every N seconds. Only meaningful with
    /// `schedule_date` set.
    pub schedule_repeat_period: Option<i32>,
    /// Associate with a business-account quick reply shortcut, by ID.
    pub quick_reply_shortcut_id: Option<i32>,
    /// Attached media to send alongside the message.
    /// Use [`InputMessage::copy_media`] to attach media copied from an existing message.
    pub media: Option<tl::enums::InputMedia>,
    /// Structured rich-text content (headings, tables, code blocks, etc).
    /// Use [`InputMessage::rich_text`] to attach `PageBlock`s, e.g. from
    /// [`crate::parsers::parse_rich_markdown`].
    pub rich_message: Option<tl::enums::InputRichMessage>,
}

/// Options for forwarding messages.
///
/// Used by [`crate::Client::forward_messages`] and
/// `IncomingMessage::forward_to_ex`. All fields default to `false`/`None`,
/// matching every field Telegram's `messages.forwardMessages` accepts - if
/// Telegram adds a new one later, add it here rather than hardcoding it.
#[derive(Default, Clone)]
pub struct ForwardOptions {
    /// Send silently (no notification for recipient).
    pub silent: bool,
    /// Send as a background message (doesn't bump the chat to the top).
    pub background: bool,
    /// Also forward the sender's high score, for messages containing a game.
    pub with_my_score: bool,
    /// Strip the original author attribution (`Forwarded from …`).
    pub drop_author: bool,
    /// Remove captions from forwarded media.
    pub drop_media_captions: bool,
    /// Prevent recipients from forwarding the message further.
    pub noforwards: bool,
    /// Reply to an existing message in the destination chat.
    pub reply_to: Option<i32>,
    /// Forward into this forum topic thread (the `top_msg_id` of the topic,
    /// see [`crate::Client::get_forum_topics`]). Needed when the
    /// destination is a supergroup that has forum topics enabled - without
    /// it, forwarded messages land outside any topic instead of the one
    /// you meant.
    pub topic_id: Option<i32>,
    /// Schedule forwarding for this Unix timestamp (seconds).
    pub schedule_date: Option<i32>,
    /// Repeat the scheduled forward every N seconds. Only meaningful with
    /// `schedule_date` set.
    pub schedule_repeat_period: Option<i32>,
    /// Forward as a different identity you're allowed to send as (e.g. an
    /// anonymous admin identity, or a linked channel) instead of yourself.
    pub send_as: Option<crate::PeerRef>,
    /// Attach a message effect (Premium sticker/emoji effect) by its ID.
    pub effect: Option<i64>,
    /// Start the forwarded video's preview at this timestamp (seconds).
    pub video_timestamp: Option<i32>,
    /// Stars you're willing to pay if the destination charges for messages.
    pub allow_paid_stars: Option<i64>,
    /// Bypass paid-messages flood limits by paying Stars.
    pub allow_paid_floodskip: bool,
    /// Associate the forward with a business-account quick reply shortcut.
    pub quick_reply_shortcut: Option<tl::enums::InputQuickReplyShortcut>,
    /// Forward as a channel "suggested post" instead of posting directly.
    pub suggested_post: Option<tl::enums::SuggestedPost>,
}

/// Selects which flavour of message link [`crate::Client::export_message_link`] should produce.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LinkKind {
    /// A plain `t.me/channel/msgid` permalink (default).
    #[default]
    Normal,
    /// A link that reveals the whole album / media group the message belongs to.
    Grouped,
    /// A link that opens the thread (comments) attached to a channel post.
    Thread,
}

impl InputMessage {
    /// Create a message with the given text.
    pub fn text(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            ..Default::default()
        }
    }

    /// Create a message by parsing Telegram-flavoured markdown.
    ///
    /// The markdown is stripped and the resulting plain text + entities are
    /// set on the message. Supports `**bold**`, `_italic_`, `` `code` ``,
    /// `[text](url)`, `||spoiler||`, `~~strike~~`, `![text](tg://emoji?id=...)`,
    /// and backslash escapes.
    ///
    /// ```rust,no_run
    /// use ferogram::InputMessage;
    ///
    /// let msg = InputMessage::markdown("**Hello** _world_!");
    /// ```
    #[cfg(feature = "parsers")]
    pub fn markdown(text: impl AsRef<str>) -> Self {
        let (plain, ents) = crate::parsers::parse_markdown(text.as_ref());
        Self {
            text: plain,
            entities: if ents.is_empty() { None } else { Some(ents) },
            ..Default::default()
        }
    }

    /// Create a message by parsing Telegram-compatible HTML.
    ///
    /// Supports `<b>`, `<i>`, `<u>`, `<s>`, `<code>`, `<pre>`,
    /// `<tg-spoiler>`, `<a href="...">`, `<tg-emoji emoji-id="...">`.
    ///
    /// ```rust,no_run
    /// use ferogram::InputMessage;
    ///
    /// let msg = InputMessage::html("<b>Hello</b> <i>world</i>!");
    /// ```
    #[cfg(feature = "parsers")]
    pub fn html(text: impl AsRef<str>) -> Self {
        let (plain, ents) = crate::parsers::parse_html(text.as_ref());
        Self {
            text: plain,
            entities: if ents.is_empty() { None } else { Some(ents) },
            ..Default::default()
        }
    }

    /// Set the message text.
    pub fn set_text(mut self, text: impl Into<String>) -> Self {
        self.text = text.into();
        self
    }

    /// Reply to a specific message ID.
    pub fn reply_to(mut self, id: Option<i32>) -> Self {
        self.reply_to = id;
        self
    }

    /// Send silently (no notification sound).
    pub fn silent(mut self, v: bool) -> Self {
        self.silent = v;
        self
    }

    /// Send in background.
    pub fn background(mut self, v: bool) -> Self {
        self.background = v;
        self
    }

    /// Clear the draft after sending.
    pub fn clear_draft(mut self, v: bool) -> Self {
        self.clear_draft = v;
        self
    }

    /// Disable link preview.
    pub fn no_webpage(mut self, v: bool) -> Self {
        self.no_webpage = v;
        self
    }

    /// Show media above the caption rather than below (requires Telegram ≥ 10.3).
    pub fn invert_media(mut self, v: bool) -> Self {
        self.invert_media = v;
        self
    }

    /// Schedule the message to be sent when the recipient comes online.
    ///
    /// Mutually exclusive with `schedule_date`: calling this last wins.
    /// Uses the Telegram magic value `0x7FFFFFFE`.
    pub fn schedule_once_online(mut self) -> Self {
        self.schedule_once_online = true;
        self.schedule_date = None;
        self
    }

    /// Attach formatting entities (bold, italic, code, links, etc).
    pub fn entities(mut self, e: Vec<tl::enums::MessageEntity>) -> Self {
        self.entities = Some(e);
        self
    }

    /// Attach a reply markup (inline or reply keyboard).
    pub fn reply_markup(mut self, rm: impl Into<tl::enums::ReplyMarkup>) -> Self {
        self.reply_markup = Some(rm.into());
        self
    }

    /// Schedule the message for a future Unix timestamp.
    pub fn schedule_date(mut self, ts: Option<i32>) -> Self {
        self.schedule_date = ts;
        self
    }

    /// Repeat the scheduled send every N seconds. Only meaningful with
    /// `schedule_date` set.
    pub fn schedule_repeat_period(mut self, seconds: Option<i32>) -> Self {
        self.schedule_repeat_period = seconds;
        self
    }

    /// Associate this message with a business-account quick reply shortcut.
    pub fn quick_reply_shortcut_id(mut self, id: Option<i32>) -> Self {
        self.quick_reply_shortcut_id = id;
        self
    }

    /// Attach media copied from an existing message.
    ///
    /// Pass the `InputMedia` obtained from [`crate::media::Photo`],
    /// [`crate::media::Document`], or directly from a raw `MessageMedia`.
    ///
    /// When a `media` is set, the message is sent via `messages.SendMedia`
    /// instead of `messages.SendMessage`.
    ///
    /// ```rust,no_run
    /// # use ferogram::{InputMessage, tl};
    /// # fn example(media: tl::enums::InputMedia) {
    /// let msg = InputMessage::text("Here is the file again")
    /// .copy_media(media);
    /// # }
    /// ```
    pub fn copy_media(mut self, media: tl::enums::InputMedia) -> Self {
        self.media = Some(media);
        self
    }

    /// Remove any previously attached media.
    pub fn clear_media(mut self) -> Self {
        self.media = None;
        self
    }

    /// Attach structured rich-text content (headings, tables, code blocks,
    /// collapsible sections, etc), rendered as a full document inside
    /// Telegram instead of flat text.
    ///
    /// Pass the blocks returned by [`crate::parsers::parse_rich_markdown`] or
    /// [`crate::parsers::parse_rich_html`].
    ///
    /// ```rust,no_run
    /// # use ferogram::{InputMessage, parsers::parse_rich_markdown};
    /// let blocks = parse_rich_markdown("# Hello\n\nWorld");
    /// let msg = InputMessage::text("").rich_text(blocks);
    /// ```
    pub fn rich_text(mut self, blocks: Vec<tl::enums::PageBlock>) -> Self {
        self.rich_message = Some(tl::enums::InputRichMessage::InputRichMessage(
            tl::types::InputRichMessage {
                rtl: false,
                noautolink: false,
                blocks,
                photos: None,
                documents: None,
                users: None,
            },
        ));
        self
    }

    pub(crate) fn reply_header(&self) -> Option<tl::enums::InputReplyTo> {
        self.reply_to.map(|id| {
            tl::enums::InputReplyTo::Message(tl::types::InputReplyToMessage {
                reply_to_msg_id: id,
                top_msg_id: None,
                reply_to_peer_id: None,
                quote_text: None,
                quote_entities: None,
                quote_offset: None,
                monoforum_peer_id: None,
                todo_item_id: None,
                poll_option: None,
            })
        })
    }
}

impl From<&str> for InputMessage {
    fn from(s: &str) -> Self {
        Self::text(s)
    }
}

impl From<String> for InputMessage {
    fn from(s: String) -> Self {
        Self::text(s)
    }
}

/// Groups all invoice parameters for [`crate::Client::send_invoice`].
#[derive(Debug, Default, Clone)]
pub struct InvoiceOptions {
    /// Three-letter ISO 4217 currency code (e.g. `"USD"`).
    pub currency: String,
    /// Line items: `(label, amount_in_smallest_units)`.
    pub prices: Vec<(String, i64)>,
    /// Optional URL of a photo to attach to the invoice.
    pub photo_url: Option<String>,
    /// Request the payer's full name.
    pub need_name: bool,
    /// Request the payer's phone number.
    pub need_phone: bool,
    /// Request the payer's email address.
    pub need_email: bool,
    /// Request the payer's shipping address.
    pub need_shipping_address: bool,
    /// Whether the final price depends on the shipping method.
    pub is_flexible: bool,
}
