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

//! [`InputReactions`]: typed parameter for reacting to messages.
//!
//!  API.
//!
//! # Examples
//!
//! ```rust,no_run
//! use ferogram::reactions::InputReactions;
//!
//! // Simple emoji
//! InputReactions::emoticon("👍");
//!
//! // Custom emoji (premium)
//! InputReactions::custom_emoji(1234567890);
//!
//! // Remove all reactions
//! InputReactions::remove();
//!
//! // Multi-reaction
//! use ferogram_tl_types::enums::Reaction;
//! InputReactions::from(vec![
//! Reaction::Emoji(ferogram_tl_types::types::ReactionEmoji { emoticon: "👍".into() }),
//! Reaction::Emoji(ferogram_tl_types::types::ReactionEmoji { emoticon: "❤️".into() }),
//! ]);
//!
//! // Chained modifiers
//! InputReactions::emoticon("🔥").big().add_to_recent();
//! ```

use ferogram_tl_types::{self as tl, enums::Reaction};

/// A set of reactions to apply to a message.
///
/// Construct with [`InputReactions::emoticon`], [`InputReactions::custom_emoji`],
/// [`InputReactions::remove`], or `From<Vec<Reaction>>`.
#[derive(Clone, Debug, Default)]
pub struct InputReactions {
    pub(crate) reactions: Vec<Reaction>,
    pub(crate) add_to_recent: bool,
    pub(crate) big: bool,
}

impl InputReactions {
    // Constructors

    /// React with a standard Unicode emoji (e.g. `"👍"`).
    pub fn emoticon<S: Into<String>>(emoticon: S) -> Self {
        Self {
            reactions: vec![Reaction::Emoji(tl::types::ReactionEmoji {
                emoticon: emoticon.into(),
            })],
            ..Self::default()
        }
    }

    /// React with a custom (premium) emoji identified by its `document_id`.
    pub fn custom_emoji(document_id: i64) -> Self {
        Self {
            reactions: vec![Reaction::CustomEmoji(tl::types::ReactionCustomEmoji {
                document_id,
            })],
            ..Self::default()
        }
    }

    /// Remove all reactions from the message.
    pub fn remove() -> Self {
        Self::default()
    }

    // Modifiers

    /// Play the reaction with a large animated effect.
    pub fn big(mut self) -> Self {
        self.big = true;
        self
    }

    /// Add this reaction to the user's recent reactions list.
    pub fn add_to_recent(mut self) -> Self {
        self.add_to_recent = true;
        self
    }
}

// From impls

impl From<&str> for InputReactions {
    fn from(s: &str) -> Self {
        InputReactions::emoticon(s)
    }
}

impl From<String> for InputReactions {
    fn from(s: String) -> Self {
        InputReactions::emoticon(s)
    }
}

impl From<Vec<Reaction>> for InputReactions {
    fn from(reactions: Vec<Reaction>) -> Self {
        Self {
            reactions,
            ..Self::default()
        }
    }
}

impl From<InputReactions> for Vec<Reaction> {
    fn from(r: InputReactions) -> Self {
        r.reactions
    }
}
