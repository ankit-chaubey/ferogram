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
    /// Get a sticker set's contents - title, stickers, and so on.
    pub async fn get_sticker_set(
        &self,
        stickerset: tl::enums::InputStickerSet,
    ) -> Result<tl::types::messages::StickerSet, InvocationError> {
        let req = tl::functions::messages::GetStickerSet {
            stickerset,
            hash: 0,
        };
        let body = self.rpc_call_raw(&req).await?;
        let mut cur = Cursor::from_slice(&body);
        let tl::enums::messages::StickerSet::StickerSet(result) =
            tl::enums::messages::StickerSet::deserialize(&mut cur)?
        else {
            return Err(InvocationError::Deserialize(
                "unexpected StickerSet variant".into(),
            ));
        };
        Ok(result)
    }

    /// Install or uninstall a sticker set. `install: true` installs, `install: false` uninstalls.
    pub async fn toggle_stickers(
        &self,
        stickerset: tl::enums::InputStickerSet,
        install: bool,
    ) -> Result<Option<tl::enums::messages::StickerSetInstallResult>, InvocationError> {
        if install {
            let req = tl::functions::messages::InstallStickerSet {
                stickerset,
                archived: false,
            };
            let body = self.rpc_call_raw(&req).await?;
            let mut cur = Cursor::from_slice(&body);
            Ok(Some(
                tl::enums::messages::StickerSetInstallResult::deserialize(&mut cur)?,
            ))
        } else {
            let req = tl::functions::messages::UninstallStickerSet { stickerset };
            self.rpc_write(&req).await?;
            Ok(None)
        }
    }

    /// List every sticker set you have installed. Pass the `hash` from a
    /// previous call to get back `None` cheaply when nothing's changed,
    /// instead of the full list again; `0` always gets the full list.
    pub async fn get_all_stickers(
        &self,
        hash: i64,
    ) -> Result<Option<Vec<tl::types::StickerSet>>, InvocationError> {
        let req = tl::functions::messages::GetAllStickers { hash };
        let body = self.rpc_call_raw(&req).await?;
        let mut cur = Cursor::from_slice(&body);
        match tl::enums::messages::AllStickers::deserialize(&mut cur)? {
            tl::enums::messages::AllStickers::AllStickers(s) => Ok(Some(
                s.sets
                    .into_iter()
                    .map(|s| match s {
                        tl::enums::StickerSet::StickerSet(ss) => ss,
                    })
                    .collect(),
            )),
            tl::enums::messages::AllStickers::NotModified => Ok(None),
        }
    }

    /// Look up custom emoji by their document IDs, to get the actual emoji
    /// documents (image/animation data) behind them.
    pub async fn get_custom_emoji_documents(
        &self,
        document_ids: Vec<i64>,
    ) -> Result<Vec<tl::enums::Document>, InvocationError> {
        let req = tl::functions::messages::GetCustomEmojiDocuments {
            document_id: document_ids,
        };
        let body = self.rpc_call_raw(&req).await?;
        let mut cur = Cursor::from_slice(&body);
        Ok(Vec::<tl::enums::Document>::deserialize(&mut cur)?)
    }
}
