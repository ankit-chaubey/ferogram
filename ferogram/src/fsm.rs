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

// Re-export everything from the dedicated crate.
pub use ferogram_fsm::*;

use crate::update::IncomingMessage;

impl ferogram_fsm::MessageLike for IncomingMessage {
    fn sender_user_id(&self) -> Option<i64> {
        IncomingMessage::sender_user_id(self)
    }

    fn chat_id(&self) -> i64 {
        IncomingMessage::chat_id(self)
    }
}
