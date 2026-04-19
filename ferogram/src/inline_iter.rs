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

//! Inline query support: two directions:
//!
//! ## Receiving (bot side)
//! [`InlineQueryIter`] streams live inline queries arriving from users typing
//! `@yourbot query`.  Backed by the update stream.
//!
//! [`InlineResultIter`] lets a *user* account call `messages.GetInlineBotResults`
//! and iterate the results, with a `.send()` helper to forward a chosen result.

use std::collections::VecDeque;

use ferogram_tl_types as tl;
use ferogram_tl_types::{Cursor, Deserializable};
use tokio::sync::mpsc;

use crate::update::{InlineQuery, Update};
use crate::{Client, InvocationError};

// InlineQueryIter (bot side: receive)

/// Async iterator over *incoming* inline queries (bot side).
/// Created by [`Client::iter_inline_queries`].
pub struct InlineQueryIter {
    rx: mpsc::UnboundedReceiver<InlineQuery>,
}

impl InlineQueryIter {
    /// Wait for the next inline query. Returns `None` when the stream ends.
    pub async fn next(&mut self) -> Option<InlineQuery> {
        self.rx.recv().await
    }
}

/// A single result returned by a bot for an inline query.
/// Obtained from [`InlineResultIter::next`].
pub struct InlineResult {
    client: Client,
    query_id: i64,
    /// The raw TL result variant.
    pub raw: tl::enums::BotInlineResult,
}

impl InlineResult {
    /// The result ID string.
    pub fn id(&self) -> &str {
        match &self.raw {
            tl::enums::BotInlineResult::BotInlineResult(r) => &r.id,
            tl::enums::BotInlineResult::BotInlineMediaResult(r) => &r.id,
        }
    }

    /// Title, if the result has one.
    pub fn title(&self) -> Option<&str> {
        match &self.raw {
            tl::enums::BotInlineResult::BotInlineResult(r) => r.title.as_deref(),
            tl::enums::BotInlineResult::BotInlineMediaResult(r) => r.title.as_deref(),
        }
    }

    /// Description, if present.
    pub fn description(&self) -> Option<&str> {
        match &self.raw {
            tl::enums::BotInlineResult::BotInlineResult(r) => r.description.as_deref(),
            tl::enums::BotInlineResult::BotInlineMediaResult(r) => r.description.as_deref(),
        }
    }

    /// Send this inline result to the given peer.
    pub async fn send(&self, peer: tl::enums::Peer) -> Result<(), InvocationError> {
        let input_peer = self
            .client
            .inner
            .peer_cache
            .read()
            .await
            .peer_to_input(&peer)?;
        let req = tl::functions::messages::SendInlineBotResult {
            silent: false,
            background: false,
            clear_draft: false,
            hide_via: false,
            peer: input_peer,
            reply_to: None,
            random_id: crate::random_i64_pub(),
            query_id: self.query_id,
            id: self.id().to_string(),
            schedule_date: None,
            send_as: None,
            quick_reply_shortcut: None,
            allow_paid_stars: None,
        };
        self.client.rpc_call_raw_pub(&req).await?;
        Ok(())
    }
}

/// Paginated iterator over results from a bot's inline mode.
/// Created by [`Client::inline_query`].
pub struct InlineResultIter {
    client: Client,
    request: tl::functions::messages::GetInlineBotResults,
    buffer: VecDeque<InlineResult>,
    last_chunk: bool,
}

impl InlineResultIter {
    fn new(client: Client, request: tl::functions::messages::GetInlineBotResults) -> Self {
        Self {
            client,
            request,
            buffer: VecDeque::new(),
            last_chunk: false,
        }
    }

    /// Override the context peer (some bots return different results per chat type).
    pub fn peer(mut self, peer: tl::enums::InputPeer) -> Self {
        self.request.peer = peer;
        self
    }

    /// Fetch the next result. Returns `None` when all results are consumed.
    pub async fn next(&mut self) -> Result<Option<InlineResult>, InvocationError> {
        if let Some(item) = self.buffer.pop_front() {
            return Ok(Some(item));
        }
        if self.last_chunk {
            return Ok(None);
        }

        let raw = self.client.rpc_call_raw_pub(&self.request).await?;
        let mut cur = Cursor::from_slice(&raw);
        let tl::enums::messages::BotResults::BotResults(r) =
            tl::enums::messages::BotResults::deserialize(&mut cur)?;

        let query_id = r.query_id;
        if let Some(offset) = r.next_offset {
            self.request.offset = offset;
        } else {
            self.last_chunk = true;
        }

        let client = self.client.clone();
        self.buffer
            .extend(r.results.into_iter().map(|raw| InlineResult {
                client: client.clone(),
                query_id,
                raw,
            }));

        Ok(self.buffer.pop_front())
    }
}

// Client extensions

impl Client {
    /// Return an iterator that yields every *incoming* inline query (bot side).
    pub fn iter_inline_queries(&self) -> InlineQueryIter {
        let (tx, rx) = mpsc::unbounded_channel();
        let client = self.clone();
        tokio::spawn(async move {
            let mut stream = client.stream_updates();
            loop {
                match stream.next().await {
                    Some(Update::InlineQuery(q)) => {
                        if tx.send(q).is_err() {
                            break;
                        }
                    }
                    Some(_) => {}
                    None => break,
                }
            }
        });
        InlineQueryIter { rx }
    }

    /// Query a bot's inline mode and return a paginated [`InlineResultIter`].
    ///
    /// Equivalent to typing `@bot_username query` in a Telegram app.
    ///
    /// # Example
    /// ```rust,no_run
    /// # async fn f(client: ferogram::Client, bot: ferogram_tl_types::enums::Peer,
    /// #            dest: ferogram_tl_types::enums::Peer) -> Result<(), ferogram::InvocationError> {
    /// let mut iter = client.inline_query(bot, "hello").await?;
    /// while let Some(r) = iter.next().await? {
    /// println!("{}", r.title().unwrap_or("(no title)"));
    /// }
    /// # Ok(()) }
    /// ```
    pub async fn inline_query(
        &self,
        bot: tl::enums::Peer,
        query: &str,
    ) -> Result<InlineResultIter, InvocationError> {
        let input_bot = {
            match self.inner.peer_cache.read().await.peer_to_input(&bot)? {
                tl::enums::InputPeer::User(u) => {
                    tl::enums::InputUser::InputUser(tl::types::InputUser {
                        user_id: u.user_id,
                        access_hash: u.access_hash,
                    })
                }
                _ => tl::enums::InputUser::Empty,
            }
        };
        let request = tl::functions::messages::GetInlineBotResults {
            bot: input_bot,
            peer: tl::enums::InputPeer::Empty,
            geo_point: None,
            query: query.to_string(),
            offset: String::new(),
        };
        Ok(InlineResultIter::new(self.clone(), request))
    }
}
