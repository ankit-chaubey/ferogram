// Copyright (c) Ankit Chaubey <ankitchaubey.dev@gmail.com>
// SPDX-License-Identifier: MIT OR Apache-2.0
//
// ferogram: async Telegram MTProto client in Rust
// https://github.com/ankit-chaubey/ferogram

//! Pluggable session storage backends  - re-exported from [`ferogram_session`].

pub use ferogram_session::{
    BinaryFileBackend, InMemoryBackend, SessionBackend, StringSessionBackend, UpdateStateChange,
};

#[cfg(feature = "sqlite-session")]
#[cfg_attr(docsrs, doc(cfg(feature = "sqlite-session")))]
pub use ferogram_session::SqliteBackend;

#[cfg(feature = "libsql-session")]
#[cfg_attr(docsrs, doc(cfg(feature = "libsql-session")))]
pub use ferogram_session::LibSqlBackend;
