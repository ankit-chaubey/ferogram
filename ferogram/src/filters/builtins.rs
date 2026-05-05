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

use super::core::{BoxFilter, make};
use crate::update::IncomingMessage;

/// Passes every message (wildcard / fallback handler).
pub fn all() -> BoxFilter {
    make(|_| true)
}

/// Never passes (disabled handler placeholder).
pub fn none() -> BoxFilter {
    make(|_| false)
}

/// Private (1-on-1) chats only.
pub fn private() -> BoxFilter {
    make(|m| m.is_private())
}

/// Basic group chats only.
pub fn group() -> BoxFilter {
    make(|m| m.is_group())
}

/// Channels and supergroups only.
pub fn channel() -> BoxFilter {
    make(|m| m.is_channel())
}

/// Any non-empty text message.
pub fn text() -> BoxFilter {
    make(|m| m.text().is_some())
}

/// Messages with any media attachment.
pub fn media() -> BoxFilter {
    make(|m| m.has_media())
}

/// Messages with a photo.
pub fn photo() -> BoxFilter {
    make(|m| m.has_photo())
}

/// Messages with a document (file, video, audio, sticker ...).
pub fn document() -> BoxFilter {
    make(|m| m.has_document())
}

/// Forwarded messages.
pub fn forwarded() -> BoxFilter {
    make(|m| m.is_forwarded())
}

/// Reply messages.
pub fn reply() -> BoxFilter {
    make(|m| m.is_reply())
}

/// Album / grouped-media messages.
pub fn album() -> BoxFilter {
    make(|m| m.album_id().is_some())
}

/// Any bot command (`/something`).
pub fn any_command() -> BoxFilter {
    make(|m| m.is_bot_command())
}

/// A specific bot command (case-insensitive, strips `@BotName` suffix).
///
/// # Example
/// ```rust,no_run
/// use ferogram::filters::command;
/// let start = command("start");
/// let help  = command("help");
/// ```
pub fn command(name: impl Into<String>) -> BoxFilter {
    let name = name.into();
    make(move |m| m.is_command_named(&name))
}

/// Text contains a substring (case-sensitive).
pub fn text_contains(needle: impl Into<String>) -> BoxFilter {
    let needle = needle.into();
    make(move |m| m.text().is_some_and(|t| t.contains(needle.as_str())))
}

/// Text starts with a prefix (case-sensitive).
pub fn text_starts_with(prefix: impl Into<String>) -> BoxFilter {
    let prefix = prefix.into();
    make(move |m| m.text().is_some_and(|t| t.starts_with(prefix.as_str())))
}

/// Message is from a specific user ID.
pub fn from_user(id: i64) -> BoxFilter {
    make(move |m| m.sender_user_id() == Some(id))
}

/// Message is in a specific chat.
pub fn in_chat(id: i64) -> BoxFilter {
    make(move |m| m.chat_id() == id)
}

/// Filter from an arbitrary closure.
///
/// # Example
/// ```rust,no_run
/// use ferogram::filters::custom;
/// let long_text = custom(|msg| msg.text().map_or(false, |t| t.len() > 200));
/// ```
pub fn custom<F>(f: F) -> BoxFilter
where
    F: Fn(&IncomingMessage) -> bool + Send + Sync + 'static,
{
    make(f)
}
