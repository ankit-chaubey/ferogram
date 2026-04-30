// Copyright (c) Ankit Chaubey <ankitchaubey.dev@gmail.com>
// SPDX-License-Identifier: MIT OR Apache-2.0
//
// ferogram: async Telegram MTProto client in Rust
// https://github.com/ankit-chaubey/ferogram
//
// If you use or modify this code, keep this notice at the top of your file
// and include the LICENSE-MIT or LICENSE-APACHE file from this repository:
// https://github.com/ankit-chaubey/ferogram

use ferogram_tl_types as tl;

// Button

/// A single inline keyboard button.
#[derive(Clone)]
pub struct Button {
    inner: tl::enums::KeyboardButton,
}

impl Button {
    /// A button that sends a callback data payload when pressed.
    pub fn callback(text: impl Into<String>, data: impl Into<Vec<u8>>) -> Self {
        Self {
            inner: tl::enums::KeyboardButton::Callback(tl::types::KeyboardButtonCallback {
                requires_password: false,
                text: text.into(),
                data: data.into(),
                style: None,
            }),
        }
    }

    /// A button that opens a URL in the browser.
    pub fn url(text: impl Into<String>, url: impl Into<String>) -> Self {
        Self {
            inner: tl::enums::KeyboardButton::Url(tl::types::KeyboardButtonUrl {
                text: text.into(),
                url: url.into(),
                style: None,
            }),
        }
    }

    /// A button that opens a user-profile or bot link in Telegram.
    pub fn url_auth(
        text: impl Into<String>,
        url: impl Into<String>,
        fwd_text: Option<String>,
        bot: tl::enums::InputUser,
    ) -> Self {
        Self {
            inner: tl::enums::KeyboardButton::InputKeyboardButtonUrlAuth(
                tl::types::InputKeyboardButtonUrlAuth {
                    request_write_access: false,
                    text: text.into(),
                    fwd_text,
                    url: url.into(),
                    bot,
                    style: None,
                },
            ),
        }
    }

    /// A button that switches to inline mode in the current chat.
    pub fn switch_inline(text: impl Into<String>, query: impl Into<String>) -> Self {
        Self {
            inner: tl::enums::KeyboardButton::SwitchInline(tl::types::KeyboardButtonSwitchInline {
                same_peer: true,
                peer_types: None,
                text: text.into(),
                query: query.into(),
                style: None,
            }),
        }
    }

    /// A plain text button (for reply keyboards, not inline).
    pub fn text(label: impl Into<String>) -> Self {
        Self {
            inner: tl::enums::KeyboardButton::KeyboardButton(tl::types::KeyboardButton {
                text: label.into(),
                style: None,
            }),
        }
    }

    /// A button that switches to inline mode in a different (user-chosen) chat.
    pub fn switch_elsewhere(text: impl Into<String>, query: impl Into<String>) -> Self {
        Self {
            inner: tl::enums::KeyboardButton::SwitchInline(tl::types::KeyboardButtonSwitchInline {
                same_peer: false,
                peer_types: None,
                text: text.into(),
                query: query.into(),
                style: None,
            }),
        }
    }

    /// A button that opens a mini-app (full WebView with JS bridge).
    pub fn mini_app(text: impl Into<String>, url: impl Into<String>) -> Self {
        Self {
            inner: tl::enums::KeyboardButton::WebView(tl::types::KeyboardButtonWebView {
                text: text.into(),
                url: url.into(),
                style: None,
            }),
        }
    }

    /// A button that opens a simple mini-app (no JS bridge, no query_id).
    pub fn mini_app_simple(text: impl Into<String>, url: impl Into<String>) -> Self {
        Self {
            inner: tl::enums::KeyboardButton::SimpleWebView(
                tl::types::KeyboardButtonSimpleWebView {
                    text: text.into(),
                    url: url.into(),
                    style: None,
                },
            ),
        }
    }

    /// A button that requests the user's phone number (reply keyboards only).
    pub fn request_phone(text: impl Into<String>) -> Self {
        Self {
            inner: tl::enums::KeyboardButton::RequestPhone(tl::types::KeyboardButtonRequestPhone {
                text: text.into(),
                style: None,
            }),
        }
    }

    /// A button that requests the user's location (reply keyboards only).
    pub fn request_geo(text: impl Into<String>) -> Self {
        Self {
            inner: tl::enums::KeyboardButton::RequestGeoLocation(
                tl::types::KeyboardButtonRequestGeoLocation {
                    text: text.into(),
                    style: None,
                },
            ),
        }
    }

    /// A button that requests the user to create/share a poll.
    pub fn request_poll(text: impl Into<String>) -> Self {
        Self {
            inner: tl::enums::KeyboardButton::RequestPoll(tl::types::KeyboardButtonRequestPoll {
                quiz: None,
                text: text.into(),
                style: None,
            }),
        }
    }

    /// A button that requests the user to create/share a quiz.
    pub fn request_quiz(text: impl Into<String>) -> Self {
        Self {
            inner: tl::enums::KeyboardButton::RequestPoll(tl::types::KeyboardButtonRequestPoll {
                quiz: Some(true),
                text: text.into(),
                style: None,
            }),
        }
    }

    /// A button that launches a game (bots only).
    pub fn game(text: impl Into<String>) -> Self {
        Self {
            inner: tl::enums::KeyboardButton::Game(tl::types::KeyboardButtonGame {
                text: text.into(),
                style: None,
            }),
        }
    }

    /// A buy button for payments (bots only).
    pub fn buy(text: impl Into<String>) -> Self {
        Self {
            inner: tl::enums::KeyboardButton::Buy(tl::types::KeyboardButtonBuy {
                text: text.into(),
                style: None,
            }),
        }
    }

    /// A copy-to-clipboard button.
    pub fn copy_text(text: impl Into<String>, copy_text: impl Into<String>) -> Self {
        Self {
            inner: tl::enums::KeyboardButton::Copy(tl::types::KeyboardButtonCopy {
                text: text.into(),
                copy_text: copy_text.into(),
                style: None,
            }),
        }
    }

    /// Consume into the raw TL type.
    pub fn into_raw(self) -> tl::enums::KeyboardButton {
        self.inner
    }
}

// InlineKeyboard

/// Builder for an inline keyboard reply markup.
///
/// Each call to [`row`](InlineKeyboard::row) adds a new horizontal row of
/// buttons. Rows are displayed top-to-bottom.
///
/// # Example
/// ```rust,no_run
/// use ferogram::keyboard::{InlineKeyboard, Button};
///
/// let kb = InlineKeyboard::new()
/// .row([Button::callback("Option A", b"a"),
///       Button::callback("Option B", b"b")])
/// .row([Button::url("More info", "https://example.com")]);
/// ```
#[derive(Clone, Default)]
pub struct InlineKeyboard {
    rows: Vec<Vec<Button>>,
}

impl InlineKeyboard {
    /// Create an empty keyboard. Add rows with [`row`](Self::row).
    pub fn new() -> Self {
        Self::default()
    }

    /// Append a row of buttons.
    pub fn row(mut self, buttons: impl IntoIterator<Item = Button>) -> Self {
        self.rows.push(buttons.into_iter().collect());
        self
    }

    /// Convert to the `ReplyMarkup` TL type expected by message-sending functions.
    pub fn into_markup(self) -> tl::enums::ReplyMarkup {
        let rows = self
            .rows
            .into_iter()
            .map(|row| {
                tl::enums::KeyboardButtonRow::KeyboardButtonRow(tl::types::KeyboardButtonRow {
                    buttons: row.into_iter().map(Button::into_raw).collect(),
                })
            })
            .collect();

        tl::enums::ReplyMarkup::ReplyInlineMarkup(tl::types::ReplyInlineMarkup { rows })
    }
}

impl From<InlineKeyboard> for tl::enums::ReplyMarkup {
    fn from(kb: InlineKeyboard) -> Self {
        kb.into_markup()
    }
}

// ReplyKeyboard

/// Builder for a reply keyboard (shown below the message input box).
#[derive(Clone, Default)]
pub struct ReplyKeyboard {
    rows: Vec<Vec<Button>>,
    resize: bool,
    single_use: bool,
    selective: bool,
}

impl ReplyKeyboard {
    /// Create a new empty reply keyboard.
    pub fn new() -> Self {
        Self::default()
    }

    /// Append a row of text buttons.
    pub fn row(mut self, buttons: impl IntoIterator<Item = Button>) -> Self {
        self.rows.push(buttons.into_iter().collect());
        self
    }

    /// Resize keyboard to fit its content (recommended).
    pub fn resize(mut self) -> Self {
        self.resize = true;
        self
    }

    /// Hide keyboard after a single press.
    pub fn single_use(mut self) -> Self {
        self.single_use = true;
        self
    }

    /// Show keyboard only to mentioned/replied users.
    pub fn selective(mut self) -> Self {
        self.selective = true;
        self
    }

    /// Convert to `ReplyMarkup`.
    pub fn into_markup(self) -> tl::enums::ReplyMarkup {
        let rows = self
            .rows
            .into_iter()
            .map(|row| {
                tl::enums::KeyboardButtonRow::KeyboardButtonRow(tl::types::KeyboardButtonRow {
                    buttons: row.into_iter().map(Button::into_raw).collect(),
                })
            })
            .collect();

        tl::enums::ReplyMarkup::ReplyKeyboardMarkup(tl::types::ReplyKeyboardMarkup {
            resize: self.resize,
            single_use: self.single_use,
            selective: self.selective,
            persistent: false,
            rows,
            placeholder: None,
        })
    }
}

impl From<ReplyKeyboard> for tl::enums::ReplyMarkup {
    fn from(kb: ReplyKeyboard) -> Self {
        kb.into_markup()
    }
}
