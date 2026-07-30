/*
 * Copyright (c) 2026 Ankit Chaubey <ankitchaubey.dev@gmail.com>
 * https://github.com/ankit-chaubey
 *
 * Project: ferogram
 * Website: https://ferogram.dev
 *
 * Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
 * https://www.apache.org/licenses/LICENSE-2.0> or the MIT license
 * <LICENSE-MIT or https://opensource.org/licenses/MIT>, at your option.
 * This file may not be copied, modified, or distributed except according
 * to those terms.
 */

// Re-export everything from the dedicated crate.
pub use ferogram_fsm::*;

use crate::update::{CallbackQuery, IncomingMessage};

impl ferogram_fsm::MessageLike for IncomingMessage {
    fn sender_user_id(&self) -> Option<i64> {
        IncomingMessage::sender_user_id(self)
    }

    fn chat_id(&self) -> i64 {
        IncomingMessage::chat_id(self)
    }
}

impl ferogram_fsm::MessageLike for CallbackQuery {
    fn sender_user_id(&self) -> Option<i64> {
        Some(self.user_id)
    }

    fn chat_id(&self) -> i64 {
        CallbackQuery::chat_id(self)
    }
}
