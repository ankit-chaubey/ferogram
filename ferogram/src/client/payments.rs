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
    pub async fn answer_precheckout_query(
        &self,
        query_id: i64,
        ok: bool,
        error_message: Option<String>,
    ) -> Result<(), InvocationError> {
        let req = tl::functions::messages::SetBotPrecheckoutResults {
            success: ok,
            query_id,
            error: error_message,
        };
        self.rpc_write(&req).await
    }

    pub async fn answer_shipping_query(
        &self,
        query_id: i64,
        error: Option<String>,
        shipping_options: Option<Vec<tl::enums::ShippingOption>>,
    ) -> Result<(), InvocationError> {
        let req = tl::functions::messages::SetBotShippingResults {
            query_id,
            error,
            shipping_options,
        };
        self.rpc_write(&req).await
    }

    pub async fn send_invoice(
        &self,
        peer: impl Into<crate::PeerRef>,
        title: impl Into<String>,
        description: impl Into<String>,
        payload: impl Into<String>,
        options: crate::InvoiceOptions,
    ) -> Result<crate::update::IncomingMessage, InvocationError> {
        let peer = peer.into().resolve(self).await?;
        let input_peer = self.inner.peer_cache.read().await.peer_to_input(&peer)?;

        let label_prices: Vec<tl::enums::LabeledPrice> = options
            .prices
            .iter()
            .map(|(label, amount)| {
                tl::enums::LabeledPrice::LabeledPrice(tl::types::LabeledPrice {
                    label: label.clone(),
                    amount: *amount,
                })
            })
            .collect();

        let invoice = tl::enums::Invoice::Invoice(tl::types::Invoice {
            test: false,
            name_requested: options.need_name,
            phone_requested: options.need_phone,
            email_requested: options.need_email,
            shipping_address_requested: options.need_shipping_address,
            flexible: options.is_flexible,
            phone_to_provider: false,
            email_to_provider: false,
            recurring: false,
            currency: options.currency.clone(),
            prices: label_prices,
            max_tip_amount: None,
            suggested_tip_amounts: None,
            terms_url: None,
            subscription_period: None,
        });

        let media = tl::enums::InputMedia::Invoice(Box::new(tl::types::InputMediaInvoice {
            title: title.into(),
            description: description.into(),
            photo: options.photo_url.map(|url| {
                tl::enums::InputWebDocument::InputWebDocument(tl::types::InputWebDocument {
                    url,
                    size: 0,
                    mime_type: "image/jpeg".into(),
                    attributes: vec![],
                })
            }),
            invoice,
            payload: payload.into().into_bytes(),
            provider: None,
            provider_data: tl::enums::DataJson::DataJson(tl::types::DataJson { data: "{}".into() }),
            start_param: None,
            extended_media: None,
        }));

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
            random_id: crate::random_i64_pub(),
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
        let body = self.rpc_call_raw(&req).await?;
        Ok(self
            .parse_send_response(&body, &crate::InputMessage::text(""), &peer)
            .await)
    }
}
