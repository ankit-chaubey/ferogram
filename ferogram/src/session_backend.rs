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

pub use ferogram_session::{
    BinaryFileBackend, InMemoryBackend, PersistedSession, SessionBackend, StringSessionBackend,
    UpdateStateChange,
};

#[cfg(feature = "sqlite-session")]
#[cfg_attr(docsrs, doc(cfg(feature = "sqlite-session")))]
pub use ferogram_session::SqliteBackend;

#[cfg(feature = "libsql-session")]
#[cfg_attr(docsrs, doc(cfg(feature = "libsql-session")))]
pub use ferogram_session::LibSqlBackend;
