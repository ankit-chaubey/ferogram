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

#![deny(unsafe_code)]

mod errors;
mod pool;
mod retry;
mod sender;

pub use errors::{InvocationError, RpcError};
pub use pool::{ConnSlot, DcPool};
pub use retry::{AutoSleep, CircuitBreaker, NoRetries, RetryContext, RetryLoop, RetryPolicy};
pub use sender::DcConnection;
