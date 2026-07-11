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
use super::{is_bool_true, random_i64};
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
    /// Open a Telegram Mini App (a web app) in `peer`'s context and get back
    /// a session you can use to interact with it. `app` picks whether you're
    /// opening the bot's main app or a specific URL it registered.
    pub async fn open_mini_app(
        &self,
        peer: impl Into<PeerRef>,

        app: MiniApp,
    ) -> Result<MiniAppSession, InvocationError> {
        let peer_ref = peer.into().resolve(self).await?;
        let input_peer = self
            .inner
            .peer_cache
            .read()
            .await
            .peer_to_input(&peer_ref)?;

        match app {
            MiniApp::Main => {
                let res = self
                    .invoke(&tl::functions::messages::RequestMainWebView {
                        compact: false,
                        fullscreen: false,
                        peer: input_peer.clone(),
                        bot: tl::enums::InputUser::UserSelf,
                        start_param: None,
                        theme_params: None,
                        platform: "android".into(),
                    })
                    .await
                    .map(|r: tl::enums::WebViewResult| r)?;
                MiniAppSession::from_result(self.clone(), input_peer, res)
            }
            MiniApp::Url(url) => {
                let res = self
                    .invoke(&tl::functions::messages::RequestWebView {
                        compact: false,
                        fullscreen: false,
                        from_bot_menu: false,
                        silent: false,
                        peer: input_peer.clone(),
                        bot: tl::enums::InputUser::UserSelf,
                        url: Some(url),
                        start_param: None,
                        theme_params: None,
                        platform: "android".into(),
                        reply_to: None,
                        send_as: None,
                    })
                    .await
                    .map(|r: tl::enums::WebViewResult| r)?;
                MiniAppSession::from_result(self.clone(), input_peer, res)
            }
            MiniApp::App {
                bot: _,
                app,
                start_param,
            } => {
                let res = self
                    .invoke(&tl::functions::messages::RequestAppWebView {
                        compact: false,
                        fullscreen: false,
                        write_allowed: false,
                        peer: input_peer.clone(),
                        app,
                        start_param,
                        theme_params: None,
                        platform: "android".into(),
                    })
                    .await
                    .map(|r: tl::enums::WebViewResult| r)?;
                MiniAppSession::from_result(self.clone(), input_peer, res)
            }
            MiniApp::Simple(url) => {
                let res = self
                    .invoke(&tl::functions::messages::RequestSimpleWebView {
                        compact: false,
                        fullscreen: false,
                        from_switch_webview: false,
                        from_side_menu: false,
                        bot: tl::enums::InputUser::UserSelf,
                        url: Some(url),
                        start_param: None,
                        theme_params: None,
                        platform: "android".into(),
                    })
                    .await
                    .map(|r: tl::enums::WebViewResult| r)?;
                Ok(MiniAppSession {
                    url: match res {
                        tl::enums::WebViewResult::Url(r) => r.url,
                    },
                    query_id: None,
                    client: self.clone(),
                    input_peer,
                })
            }
        }
    }

    /// Respond to a button tap from an inline keyboard. With `alert: true`,
    /// `text` shows as a popup the user has to dismiss; otherwise it's a brief
    /// toast. You should call this for every callback query you get, even
    /// with no text, or the button stays stuck in a loading state.
    pub async fn answer_callback_query(
        &self,
        query_id: i64,

        text: Option<&str>,
        alert: bool,
    ) -> Result<bool, InvocationError> {
        let req = tl::functions::messages::SetBotCallbackAnswer {
            alert,
            query_id,
            message: text.map(|s| s.to_string()),
            url: None,
            cache_time: 0,
        };
        let body: Vec<u8> = self.rpc_call_raw(&req).await?;
        Ok(body.len() >= 4 && u32::from_le_bytes(body[..4].try_into().unwrap()) == 0x997275b5)
    }

    /// Send results back for an `@your_bot query` inline query. `next_offset`
    /// lets the client ask for more results when the user scrolls, by passing
    /// it back to you as the next query's offset.
    pub async fn answer_inline_query(
        &self,
        query_id: i64,
        results: Vec<tl::enums::InputBotInlineResult>,

        cache_time: i32,
        is_personal: bool,

        next_offset: Option<String>,
    ) -> Result<bool, InvocationError> {
        let req = tl::functions::messages::SetInlineBotResults {
            gallery: false,
            private: is_personal,
            query_id,
            results,
            cache_time,
            next_offset,
            switch_pm: None,
            switch_webview: None,
        };
        let body: Vec<u8> = self.rpc_call_raw(&req).await?;
        Ok(body.len() >= 4 && u32::from_le_bytes(body[..4].try_into().unwrap()) == 0x997275b5)
    }

    /// Send `/start` to a bot with a deep-link parameter, as if the user
    /// tapped a `t.me/bot?start=...` link.
    pub async fn start_bot(
        &self,
        bot_user_id: i64,
        peer: impl Into<PeerRef>,
        start_param: impl Into<String>,
    ) -> Result<(), InvocationError> {
        let bot_hash = self
            .inner
            .peer_cache
            .read()
            .await
            .users
            .get(&bot_user_id)
            .copied()
            .unwrap_or(0);
        let peer = peer.into().resolve(self).await?;
        let input_peer = self.inner.peer_cache.read().await.peer_to_input(&peer)?;
        let req = tl::functions::messages::StartBot {
            bot: tl::enums::InputUser::InputUser(tl::types::InputUser {
                user_id: bot_user_id,
                access_hash: bot_hash,
            }),
            peer: input_peer,
            random_id: random_i64(),
            start_param: start_param.into(),
        };
        self.rpc_write(&req).await
    }

    /// Set a user's score for the game in a sent game message. With
    /// `force: true`, the score is set even if it's lower than the user's
    /// current one; with `edit_message: true`, the message's scoreboard is
    /// updated in place.
    pub async fn set_game_score(
        &self,
        peer: impl Into<PeerRef>,
        msg_id: i32,
        user_id: i64,
        score: i32,
        force: bool,
        edit_message: bool,
    ) -> Result<(), InvocationError> {
        let peer = peer.into().resolve(self).await?;
        let input_peer = self.inner.peer_cache.read().await.peer_to_input(&peer)?;
        let user_hash = self
            .inner
            .peer_cache
            .read()
            .await
            .users
            .get(&user_id)
            .copied()
            .unwrap_or(0);
        let req = tl::functions::messages::SetGameScore {
            edit_message,
            force,
            peer: input_peer,
            id: msg_id,
            user_id: tl::enums::InputUser::InputUser(tl::types::InputUser {
                user_id,
                access_hash: user_hash,
            }),
            score,
        };
        self.rpc_write(&req).await
    }

    /// Get the high score table for the game in a sent game message,
    /// centered around `user_id`.
    pub async fn get_game_high_scores(
        &self,
        peer: impl Into<PeerRef>,
        msg_id: i32,
        user_id: i64,
    ) -> Result<Vec<tl::types::HighScore>, InvocationError> {
        let peer = peer.into().resolve(self).await?;
        let input_peer = self.inner.peer_cache.read().await.peer_to_input(&peer)?;
        let user_hash = self
            .inner
            .peer_cache
            .read()
            .await
            .users
            .get(&user_id)
            .copied()
            .unwrap_or(0);
        let req = tl::functions::messages::GetGameHighScores {
            peer: input_peer,
            id: msg_id,
            user_id: tl::enums::InputUser::InputUser(tl::types::InputUser {
                user_id,
                access_hash: user_hash,
            }),
        };
        let body: Vec<u8> = self.rpc_call_raw(&req).await?;
        let mut cur = Cursor::from_slice(&body);
        let tl::enums::messages::HighScores::HighScores(result) =
            tl::enums::messages::HighScores::deserialize(&mut cur)?;
        self.cache_users_slice(&result.users).await;
        Ok(result
            .scores
            .into_iter()
            .map(|s| match s {
                tl::enums::HighScore::HighScore(h) => h,
            })
            .collect())
    }

    /// Edit a message that was sent via inline mode (one identified by an
    /// `InputBotInlineMessageId`, not a regular message ID).
    pub async fn edit_inline_message(
        &self,
        id: tl::enums::InputBotInlineMessageId,
        new_text: &str,
        reply_markup: Option<tl::enums::ReplyMarkup>,
    ) -> Result<bool, InvocationError> {
        let req = tl::functions::messages::EditInlineBotMessage {
            no_webpage: false,
            invert_media: false,
            id,
            message: Some(new_text.to_string()),
            media: None,
            reply_markup,
            entities: None,
            rich_message: None,
        };
        let body: Vec<u8> = self.rpc_call_raw(&req).await?;
        Ok(body.len() >= 4 && u32::from_le_bytes(body[..4].try_into().unwrap()) == 0x997275b5)
    }

    /// Set the bot's command list - the menu users see when they tap the
    /// `/` icon. `scope` controls where it applies (everywhere, a specific
    /// chat, just admins, etc); `None` means the default scope.
    pub async fn set_bot_commands(
        &self,
        commands: &[(&str, &str)],
        scope: Option<tl::enums::BotCommandScope>,
        lang_code: &str,
    ) -> Result<bool, InvocationError> {
        let bot_commands: Vec<tl::enums::BotCommand> = commands
            .iter()
            .map(|(cmd, desc)| {
                tl::enums::BotCommand::BotCommand(tl::types::BotCommand {
                    ephemeral: false,
                    command: cmd.to_string(),
                    description: desc.to_string(),
                })
            })
            .collect();
        let req = tl::functions::bots::SetBotCommands {
            scope: scope.unwrap_or(tl::enums::BotCommandScope::Default),
            lang_code: lang_code.to_string(),
            commands: bot_commands,
        };
        let body = self.rpc_call_raw(&req).await?;
        Ok(is_bool_true(&body))
    }

    /// Clear the bot's command list for a scope, falling back to whatever the
    /// next broader scope defines.
    pub async fn delete_bot_commands(
        &self,
        scope: Option<tl::enums::BotCommandScope>,
        lang_code: &str,
    ) -> Result<bool, InvocationError> {
        let req = tl::functions::bots::ResetBotCommands {
            scope: scope.unwrap_or(tl::enums::BotCommandScope::Default),
            lang_code: lang_code.to_string(),
        };
        let body = self.rpc_call_raw(&req).await?;
        Ok(is_bool_true(&body))
    }

    /// Set bot profile info.
    ///
    /// `bot`: pass the bot's peer when calling from a userbot that owns the
    /// bot. Pass `None` when calling from the bot session itself.
    ///
    /// All text fields are optional; only the ones you supply are changed.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # async fn f(client: ferogram::Client) -> Result<(), Box<dyn std::error::Error>> {
    /// // From a bot session: edit self.
    /// client.set_bot_info(None::<&str>, Some("My Bot"), Some("Short about."), Some("Start page text."), "en").await?;
    ///
    /// // From a userbot: edit an owned bot.
    /// client.set_bot_info(Some("@MyBot"), Some("My Bot"), None, None, "en").await?;
    /// # Ok(()) }
    /// ```
    pub async fn set_bot_info(
        &self,
        bot: Option<impl Into<PeerRef>>,
        name: Option<&str>,
        about: Option<&str>,
        description: Option<&str>,
        lang_code: &str,
    ) -> Result<bool, InvocationError> {
        let bot_input = if let Some(peer) = bot {
            let resolved = peer.into().resolve(self).await?;
            let input_peer = self
                .inner
                .peer_cache
                .read()
                .await
                .peer_to_input(&resolved)?;
            let input_user = match input_peer {
                tl::enums::InputPeer::User(u) => {
                    tl::enums::InputUser::InputUser(tl::types::InputUser {
                        user_id: u.user_id,
                        access_hash: u.access_hash,
                    })
                }
                tl::enums::InputPeer::PeerSelf => tl::enums::InputUser::UserSelf,
                _ => {
                    return Err(InvocationError::Deserialize(
                        "peer must resolve to a user (bot)".into(),
                    ));
                }
            };
            Some(input_user)
        } else {
            None
        };
        let req = tl::functions::bots::SetBotInfo {
            bot: bot_input,
            lang_code: lang_code.to_string(),
            name: name.map(|s| s.to_string()),
            about: about.map(|s| s.to_string()),
            description: description.map(|s| s.to_string()),
        };
        let body = self.rpc_call_raw(&req).await?;
        Ok(is_bool_true(&body))
    }

    /// Get bot profile info.
    ///
    /// `bot`: pass the bot's peer when calling from a userbot. Pass `None`
    /// when calling from the bot session itself.
    pub async fn get_bot_info(
        &self,
        bot: Option<impl Into<PeerRef>>,
        lang_code: &str,
    ) -> Result<tl::types::bots::BotInfo, InvocationError> {
        use ferogram_tl_types::{Cursor, Deserializable};
        let bot_input = if let Some(peer) = bot {
            let resolved = peer.into().resolve(self).await?;
            let input_peer = self
                .inner
                .peer_cache
                .read()
                .await
                .peer_to_input(&resolved)?;
            let input_user = match input_peer {
                tl::enums::InputPeer::User(u) => {
                    tl::enums::InputUser::InputUser(tl::types::InputUser {
                        user_id: u.user_id,
                        access_hash: u.access_hash,
                    })
                }
                tl::enums::InputPeer::PeerSelf => tl::enums::InputUser::UserSelf,
                _ => {
                    return Err(InvocationError::Deserialize(
                        "peer must resolve to a user (bot)".into(),
                    ));
                }
            };
            Some(input_user)
        } else {
            None
        };
        let req = tl::functions::bots::GetBotInfo {
            bot: bot_input,
            lang_code: lang_code.to_string(),
        };
        let body = self.rpc_call_raw(&req).await?;
        let mut cur = Cursor::from_slice(&body);
        let tl::enums::bots::BotInfo::BotInfo(result) =
            tl::enums::bots::BotInfo::deserialize(&mut cur)?;
        Ok(result)
    }
}
