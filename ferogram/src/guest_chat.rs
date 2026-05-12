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

use std::ops::Deref;

use ferogram_tl_types as tl;

use crate::{Client, InvocationError as Error, update::IncomingMessage};

/// A guest-chat inline query sent to the bot (`updateBotGuestChatQuery`).
#[derive(Debug, Clone)]
pub struct GuestChatQuery {
    pub query_id: i64,
    pub message: IncomingMessage,
    pub reference_messages: Vec<IncomingMessage>,
    pub qts: i32,
}

impl GuestChatQuery {
    /// Begin building an answer for this query.
    pub fn answer(&self) -> GuestChatAnswer<'_> {
        GuestChatAnswer {
            query_id: self.query_id,
            result_kind: None,
            id: String::new(),
            description: None,
            url: None,
            thumb: None,
            caption: None,
            message: None,
            reply_markup: None,
            entities: None,
            _marker: std::marker::PhantomData,
        }
    }

    /// The peer that originally triggered this guest-chat query, if Telegram
    /// included it in the message (`guestchat_via_from`).
    ///
    /// Present when the bot is acting as an intermediary and Telegram
    /// wants it to know the original requester.
    pub fn via_from(&self) -> Option<&tl::enums::Peer> {
        match &self.message.raw {
            tl::enums::Message::Message(m) => m.guestchat_via_from.as_ref(),
            _ => None,
        }
    }
}

impl Deref for GuestChatQuery {
    type Target = IncomingMessage;
    fn deref(&self) -> &Self::Target {
        &self.message
    }
}

/// The kind of inline result being built.
#[derive(Debug, Clone)]
enum ResultKind {
    Article {
        title: String,
    },
    Photo {
        photo: tl::enums::InputPhoto,
    },
    Document {
        document: tl::enums::InputDocument,
        title: Option<String>,
    },
    Game {
        short_name: String,
    },
    Location {
        geo: tl::enums::InputGeoPoint,
        live_period: Option<i32>,
        heading: Option<i32>,
        proximity_alert: Option<i32>,
    },
    Venue {
        geo: tl::enums::InputGeoPoint,
        title: String,
        address: String,
        provider: String,
        venue_id: String,
        venue_type: String,
    },
    Contact {
        phone: String,
        first_name: String,
        last_name: String,
        vcard: Option<String>,
    },
    Webpage {
        url: String,
    },
    Invoice {
        title: String,
        description: String,
        payload: Vec<u8>,
        provider: String,
        provider_data: String,
        currency: String,
        prices: Vec<tl::enums::LabeledPrice>,
    },
    Raw(Box<tl::enums::InputBotInlineResult>),
}

/// Fluent builder returned by [`GuestChatQuery::answer`].
pub struct GuestChatAnswer<'a> {
    query_id: i64,
    result_kind: Option<ResultKind>,
    id: String,
    description: Option<String>,
    url: Option<String>,
    thumb: Option<tl::enums::InputWebDocument>,
    caption: Option<String>,
    // InputBotInlineMessage variant fields
    message: Option<MessageKind>,
    reply_markup: Option<tl::enums::ReplyMarkup>,
    entities: Option<Vec<tl::enums::MessageEntity>>,
    _marker: std::marker::PhantomData<&'a ()>,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
enum MessageKind {
    Text {
        text: String,
        no_webpage: bool,
        invert_media: bool,
        force_large_media: bool,
        force_small_media: bool,
        optional: bool,
    },
    Webpage {
        url: String,
        message: String,
        invert_media: bool,
        force_large_media: bool,
        force_small_media: bool,
        optional: bool,
    },
}

impl<'a> GuestChatAnswer<'a> {
    pub fn article(mut self, title: impl Into<String>) -> Self {
        self.result_kind = Some(ResultKind::Article {
            title: title.into(),
        });
        self
    }

    pub fn photo(mut self, photo: tl::enums::InputPhoto) -> Self {
        self.result_kind = Some(ResultKind::Photo { photo });
        self
    }

    pub fn document(mut self, document: tl::enums::InputDocument, title: Option<String>) -> Self {
        self.result_kind = Some(ResultKind::Document { document, title });
        self
    }

    pub fn game(mut self, short_name: impl Into<String>) -> Self {
        self.result_kind = Some(ResultKind::Game {
            short_name: short_name.into(),
        });
        self
    }

    pub fn location(mut self, lat: f64, long: f64) -> Self {
        self.result_kind = Some(ResultKind::Location {
            geo: tl::enums::InputGeoPoint::InputGeoPoint(tl::types::InputGeoPoint {
                lat,
                long,
                accuracy_radius: None,
            }),
            live_period: None,
            heading: None,
            proximity_alert: None,
        });
        self
    }

    #[allow(clippy::too_many_arguments)]
    pub fn venue(
        mut self,
        lat: f64,
        long: f64,
        title: impl Into<String>,
        address: impl Into<String>,
        provider: impl Into<String>,
        venue_id: impl Into<String>,
        venue_type: impl Into<String>,
    ) -> Self {
        self.result_kind = Some(ResultKind::Venue {
            geo: tl::enums::InputGeoPoint::InputGeoPoint(tl::types::InputGeoPoint {
                lat,
                long,
                accuracy_radius: None,
            }),
            title: title.into(),
            address: address.into(),
            provider: provider.into(),
            venue_id: venue_id.into(),
            venue_type: venue_type.into(),
        });
        self
    }

    pub fn contact(
        mut self,
        phone: impl Into<String>,
        first_name: impl Into<String>,
        last_name: impl Into<String>,
    ) -> Self {
        self.result_kind = Some(ResultKind::Contact {
            phone: phone.into(),
            first_name: first_name.into(),
            last_name: last_name.into(),
            vcard: None,
        });
        self
    }

    pub fn webpage(mut self, url: impl Into<String>) -> Self {
        self.result_kind = Some(ResultKind::Webpage { url: url.into() });
        self
    }

    #[allow(clippy::too_many_arguments)]
    pub fn invoice(
        mut self,
        title: impl Into<String>,
        description: impl Into<String>,
        payload: Vec<u8>,
        provider: impl Into<String>,
        provider_data: impl Into<String>,
        currency: impl Into<String>,
        prices: Vec<tl::enums::LabeledPrice>,
    ) -> Self {
        self.result_kind = Some(ResultKind::Invoice {
            title: title.into(),
            description: description.into(),
            payload,
            provider: provider.into(),
            provider_data: provider_data.into(),
            currency: currency.into(),
            prices,
        });
        self
    }

    /// Pass a fully-constructed `InputBotInlineResult` directly.
    pub fn raw(mut self, result: tl::enums::InputBotInlineResult) -> Self {
        self.result_kind = Some(ResultKind::Raw(Box::new(result)));
        self
    }

    pub fn id(mut self, id: impl Into<String>) -> Self {
        self.id = id.into();
        self
    }

    pub fn description(mut self, text: impl Into<String>) -> Self {
        self.description = Some(text.into());
        self
    }

    pub fn url(mut self, url: impl Into<String>) -> Self {
        self.url = Some(url.into());
        self
    }

    pub fn thumb(mut self, doc: tl::enums::InputWebDocument) -> Self {
        self.thumb = Some(doc);
        self
    }

    pub fn caption(mut self, text: impl Into<String>) -> Self {
        self.caption = Some(text.into());
        self
    }

    pub fn text(mut self, text: impl Into<String>) -> Self {
        let t = text.into();
        self.message = Some(match self.message.take() {
            Some(MessageKind::Text {
                no_webpage,
                invert_media,
                force_large_media,
                force_small_media,
                optional,
                ..
            }) => MessageKind::Text {
                text: t,
                no_webpage,
                invert_media,
                force_large_media,
                force_small_media,
                optional,
            },
            _ => MessageKind::Text {
                text: t,
                no_webpage: false,
                invert_media: false,
                force_large_media: false,
                force_small_media: false,
                optional: false,
            },
        });
        self
    }

    pub fn no_webpage(mut self, v: bool) -> Self {
        if let Some(MessageKind::Text { no_webpage, .. }) = &mut self.message {
            *no_webpage = v;
        }
        self
    }

    pub fn invert_media(mut self, v: bool) -> Self {
        match &mut self.message {
            Some(MessageKind::Text { invert_media, .. }) => *invert_media = v,
            Some(MessageKind::Webpage { invert_media, .. }) => *invert_media = v,
            _ => {}
        }
        self
    }

    pub fn force_large_media(mut self, v: bool) -> Self {
        match &mut self.message {
            Some(MessageKind::Text {
                force_large_media, ..
            }) => *force_large_media = v,
            Some(MessageKind::Webpage {
                force_large_media, ..
            }) => *force_large_media = v,
            _ => {}
        }
        self
    }

    pub fn force_small_media(mut self, v: bool) -> Self {
        match &mut self.message {
            Some(MessageKind::Text {
                force_small_media, ..
            }) => *force_small_media = v,
            Some(MessageKind::Webpage {
                force_small_media, ..
            }) => *force_small_media = v,
            _ => {}
        }
        self
    }

    pub fn optional(mut self, v: bool) -> Self {
        match &mut self.message {
            Some(MessageKind::Text { optional, .. }) => *optional = v,
            Some(MessageKind::Webpage { optional, .. }) => *optional = v,
            _ => {}
        }
        self
    }

    pub fn live_period(mut self, secs: i32) -> Self {
        if let Some(ResultKind::Location { live_period, .. }) = &mut self.result_kind {
            *live_period = Some(secs);
        }
        self
    }

    pub fn heading(mut self, deg: i32) -> Self {
        if let Some(ResultKind::Location { heading, .. }) = &mut self.result_kind {
            *heading = Some(deg);
        }
        self
    }

    pub fn proximity_alert(mut self, meters: i32) -> Self {
        if let Some(ResultKind::Location {
            proximity_alert, ..
        }) = &mut self.result_kind
        {
            *proximity_alert = Some(meters);
        }
        self
    }

    pub fn vcard(mut self, vcard: impl Into<String>) -> Self {
        if let Some(ResultKind::Contact { vcard: v, .. }) = &mut self.result_kind {
            *v = Some(vcard.into());
        }
        self
    }

    pub fn reply_markup(mut self, markup: tl::enums::ReplyMarkup) -> Self {
        self.reply_markup = Some(markup);
        self
    }

    pub fn entities(mut self, ents: Vec<tl::enums::MessageEntity>) -> Self {
        self.entities = Some(ents);
        self
    }

    /// Send this answer to Telegram.
    ///
    /// On success returns the `InputBotInlineMessageID` of the sent message,
    /// which you can use later with `messages.editInlineBotMessage` or
    /// `messages.setInlineGameScore` if needed.
    pub async fn send(self, client: &Client) -> Result<tl::enums::InputBotInlineMessageId, Error> {
        let req = tl::functions::messages::SetBotGuestChatResult {
            query_id: self.query_id,
            result: self.build_result(),
        };
        client.invoke(&req).await
    }

    fn build_result(self) -> tl::enums::InputBotInlineResult {
        if let Some(ResultKind::Raw(r)) = self.result_kind {
            return *r;
        }

        // Build the send_message before we destructure self
        let send_message = {
            let markup = self.reply_markup.clone();
            let entities = self.entities.clone().unwrap_or_default();

            match &self.message {
                Some(MessageKind::Webpage {
                    url,
                    message,
                    invert_media,
                    force_large_media,
                    force_small_media,
                    optional,
                }) => tl::enums::InputBotInlineMessage::MediaWebPage(
                    tl::types::InputBotInlineMessageMediaWebPage {
                        invert_media: *invert_media,
                        force_large_media: *force_large_media,
                        force_small_media: *force_small_media,
                        optional: *optional,
                        message: message.clone(),
                        entities: if entities.is_empty() {
                            None
                        } else {
                            Some(entities)
                        },
                        url: url.clone(),
                        reply_markup: markup,
                    },
                ),
                Some(MessageKind::Text {
                    text,
                    no_webpage,
                    invert_media,
                    ..
                }) => {
                    tl::enums::InputBotInlineMessage::Text(tl::types::InputBotInlineMessageText {
                        no_webpage: *no_webpage,
                        invert_media: *invert_media,
                        message: text.clone(),
                        entities: if entities.is_empty() {
                            None
                        } else {
                            Some(entities)
                        },
                        reply_markup: markup,
                    })
                }
                None => tl::enums::InputBotInlineMessage::MediaAuto(
                    tl::types::InputBotInlineMessageMediaAuto {
                        invert_media: false,
                        message: self.caption.clone().unwrap_or_default(),
                        entities: if entities.is_empty() {
                            None
                        } else {
                            Some(entities)
                        },
                        reply_markup: markup,
                    },
                ),
            }
        };

        let id = if self.id.is_empty() {
            uuid_str()
        } else {
            self.id.clone()
        };

        match self.result_kind {
            Some(ResultKind::Photo { photo }) => {
                tl::enums::InputBotInlineResult::Photo(tl::types::InputBotInlineResultPhoto {
                    id,
                    r#type: "photo".into(),
                    photo,
                    send_message,
                })
            }
            Some(ResultKind::Document { document, title }) => {
                tl::enums::InputBotInlineResult::Document(tl::types::InputBotInlineResultDocument {
                    id,
                    r#type: "document".into(),
                    title,
                    description: self.description.clone(),
                    document,
                    send_message,
                })
            }
            Some(ResultKind::Game { short_name }) => {
                tl::enums::InputBotInlineResult::Game(tl::types::InputBotInlineResultGame {
                    id,
                    short_name,
                    send_message,
                })
            }
            Some(ResultKind::Location {
                geo,
                live_period,
                heading,
                proximity_alert,
            }) => {
                let msg = tl::enums::InputBotInlineMessage::MediaGeo(
                    tl::types::InputBotInlineMessageMediaGeo {
                        geo_point: geo,
                        heading,
                        period: live_period,
                        proximity_notification_radius: proximity_alert,
                        reply_markup: self.reply_markup.clone(),
                    },
                );
                tl::enums::InputBotInlineResult::InputBotInlineResult(
                    tl::types::InputBotInlineResult {
                        id,
                        r#type: "location".into(),
                        title: None,
                        description: self.description.clone(),
                        url: self.url.clone(),
                        thumb: self.thumb.clone(),
                        content: None,
                        send_message: msg,
                    },
                )
            }
            Some(ResultKind::Venue {
                geo,
                title,
                address,
                provider,
                venue_id,
                venue_type,
            }) => {
                let msg = tl::enums::InputBotInlineMessage::MediaVenue(
                    tl::types::InputBotInlineMessageMediaVenue {
                        geo_point: geo,
                        title: title.clone(),
                        address,
                        provider,
                        venue_id,
                        venue_type,
                        reply_markup: self.reply_markup.clone(),
                    },
                );
                tl::enums::InputBotInlineResult::InputBotInlineResult(
                    tl::types::InputBotInlineResult {
                        id,
                        r#type: "venue".into(),
                        title: Some(title),
                        description: self.description.clone(),
                        url: self.url.clone(),
                        thumb: self.thumb.clone(),
                        content: None,
                        send_message: msg,
                    },
                )
            }
            Some(ResultKind::Contact {
                phone,
                first_name,
                last_name,
                vcard,
            }) => {
                let msg = tl::enums::InputBotInlineMessage::MediaContact(
                    tl::types::InputBotInlineMessageMediaContact {
                        phone_number: phone.clone(),
                        first_name: first_name.clone(),
                        last_name: last_name.clone(),
                        vcard: vcard.unwrap_or_default(),
                        reply_markup: self.reply_markup.clone(),
                    },
                );
                tl::enums::InputBotInlineResult::InputBotInlineResult(
                    tl::types::InputBotInlineResult {
                        id,
                        r#type: "contact".into(),
                        title: Some(first_name),
                        description: self.description.clone(),
                        url: self.url.clone(),
                        thumb: self.thumb.clone(),
                        content: None,
                        send_message: msg,
                    },
                )
            }
            Some(ResultKind::Invoice {
                title,
                description,
                payload,
                provider,
                provider_data,
                currency,
                prices,
            }) => {
                let invoice = build_invoice(currency, prices);
                let msg = tl::enums::InputBotInlineMessage::MediaInvoice(
                    tl::types::InputBotInlineMessageMediaInvoice {
                        title: title.clone(),
                        description: description.clone(),
                        photo: None,
                        invoice,
                        payload,
                        provider,
                        provider_data: tl::enums::DataJson::DataJson(tl::types::DataJson {
                            data: provider_data,
                        }),
                        reply_markup: self.reply_markup.clone(),
                    },
                );
                tl::enums::InputBotInlineResult::InputBotInlineResult(
                    tl::types::InputBotInlineResult {
                        id,
                        r#type: "article".into(),
                        title: Some(title),
                        description: Some(description),
                        url: self.url.clone(),
                        thumb: self.thumb.clone(),
                        content: None,
                        send_message: msg,
                    },
                )
            }
            Some(ResultKind::Webpage { url }) => {
                // Webpage: build as article with a MediaWebPage send_message
                let msg = tl::enums::InputBotInlineMessage::MediaWebPage(
                    tl::types::InputBotInlineMessageMediaWebPage {
                        invert_media: false,
                        force_large_media: false,
                        force_small_media: false,
                        optional: false,
                        message: self.caption.clone().unwrap_or_default(),
                        entities: self.entities.clone().filter(|e| !e.is_empty()),
                        url: url.clone(),
                        reply_markup: self.reply_markup.clone(),
                    },
                );
                tl::enums::InputBotInlineResult::InputBotInlineResult(
                    tl::types::InputBotInlineResult {
                        id,
                        r#type: "article".into(),
                        title: None,
                        description: self.description.clone(),
                        url: Some(url),
                        thumb: self.thumb.clone(),
                        content: None,
                        send_message: msg,
                    },
                )
            }
            // Article or fallback
            _ => tl::enums::InputBotInlineResult::InputBotInlineResult(
                tl::types::InputBotInlineResult {
                    id,
                    r#type: "article".into(),
                    title: if let Some(ResultKind::Article { ref title }) = self.result_kind {
                        Some(title.clone())
                    } else {
                        None
                    },
                    description: self.description.clone(),
                    url: self.url.clone(),
                    thumb: self.thumb.clone(),
                    content: None,
                    send_message,
                },
            ),
        }
    }
}

fn uuid_str() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0);
    format!("{nanos:08x}")
}

fn build_invoice(currency: String, prices: Vec<tl::enums::LabeledPrice>) -> tl::enums::Invoice {
    tl::enums::Invoice::Invoice(tl::types::Invoice {
        test: false,
        name_requested: false,
        phone_requested: false,
        email_requested: false,
        shipping_address_requested: false,
        flexible: false,
        phone_to_provider: false,
        email_to_provider: false,
        recurring: false,
        currency,
        prices,
        max_tip_amount: None,
        suggested_tip_amounts: None,
        terms_url: None,
        subscription_period: None,
    })
}
