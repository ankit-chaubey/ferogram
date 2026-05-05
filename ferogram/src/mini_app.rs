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

use crate::Client;
use crate::errors::InvocationError;

/// Which kind of mini-app to open.
///
/// Passed to [`Client::open_mini_app`].
pub enum MiniApp {
    /// The bot's main mini-app (no inline button required).
    Main,
    /// A URL-based WebView mini-app (full JS bridge, generates a `query_id`).
    Url(String),
    /// A registered bot app opened by its TL handle (deeplink style).
    App {
        /// The bot that owns the app.
        bot: tl::enums::InputUser,
        /// The app handle returned by `messages.GetBotApp`.
        app: tl::enums::InputBotApp,
        /// Optional start parameter forwarded to the app.
        start_param: Option<String>,
    },
    /// A simple WebView with no JS bridge and no `query_id`.
    Simple(String),
}

/// An active mini-app session returned by [`Client::open_mini_app`].
///
/// Call [`prolong`](MiniAppSession::prolong) periodically to keep the session
/// alive while the user interacts with the WebView.
pub struct MiniAppSession {
    /// The URL the WebView should load.
    pub url: String,
    /// The `query_id` for answering `web_app_data` callbacks (`None` for Simple).
    pub query_id: Option<i64>,
    pub(crate) client: Client,
    pub(crate) input_peer: tl::enums::InputPeer,
}

impl MiniAppSession {
    #[allow(dead_code)]
    pub(crate) fn from_result(
        client: Client,
        input_peer: tl::enums::InputPeer,
        res: tl::enums::WebViewResult,
    ) -> Result<Self, InvocationError> {
        match res {
            tl::enums::WebViewResult::Url(r) => Ok(Self {
                url: r.url,
                query_id: r.query_id,
                client,
                input_peer,
            }),
        }
    }

    /// Extend the session lifetime.
    ///
    /// Telegram requires this to be called every ~25 seconds while the user is
    /// actively using the mini-app.
    pub async fn prolong(&self) -> Result<(), InvocationError> {
        self.client
            .invoke(&tl::functions::messages::ProlongWebView {
                silent: false,
                peer: self.input_peer.clone(),
                bot: tl::enums::InputUser::UserSelf,
                query_id: self.query_id.unwrap_or(0),
                reply_to: None,
                send_as: None,
            })
            .await
            .map(|_: bool| ())
    }
}
