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

impl Client {
    /// Fluent search builder for in-chat message search.
    pub fn search(&self, peer: impl Into<PeerRef>, query: &str) -> SearchBuilder {
        SearchBuilder::new(peer.into(), query.to_string())
    }

    /// Fluent builder for global cross-chat search.
    pub fn search_global(&self, query: &str) -> GlobalSearchBuilder {
        GlobalSearchBuilder::new(query.to_string())
    }
}
