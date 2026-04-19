// Copyright (c) Ankit Chaubey <ankitchaubey.dev@gmail.com>
// SPDX-License-Identifier: MIT OR Apache-2.0
//
// ferogram: async Telegram MTProto client in Rust
// https://github.com/ankit-chaubey/ferogram

//! Session persistence types  - re-exported from [`ferogram_session`].

pub use ferogram_session::{
    CachedMinPeer, CachedPeer, DcEntry, DcFlags, PersistedSession, UpdatesStateSnap,
    default_dc_addresses,
};
