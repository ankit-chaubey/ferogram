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

//! Transport abstraction between the E2E layer and a Telegram client.
//!
//! `ferogram-e2e` does not depend on the `ferogram` client crate. It only needs
//! to send RPCs, so any client can drive it by implementing [`E2eTransport`].
//! `ferogram` implements it for `ferogram::Client` when its `e2e` feature is on.

use std::future::Future;

use ferogram_tl_types::RemoteCall;

use crate::error::E2eError;

/// Minimal RPC surface the secret chat layer needs from a Telegram client.
///
/// Implementations must be cheap to clone (typically a handle around an `Arc`).
/// Failures are reported as [`E2eError::Transport`].
pub trait E2eTransport: Clone + Send + Sync + 'static {
    /// Invoke `req` on the client's home DC.
    fn invoke<R>(&self, req: &R) -> impl Future<Output = Result<R::Return, E2eError>> + Send
    where
        R: RemoteCall + Send + Sync;

    /// Invoke `req` on the datacenter `dc_id`.
    ///
    /// Secret chat files live on the DC named in `EncryptedFile::dc_id`, which
    /// is not necessarily the home DC.
    fn invoke_on_dc<R>(
        &self,
        dc_id: i32,
        req: &R,
    ) -> impl Future<Output = Result<R::Return, E2eError>> + Send
    where
        R: RemoteCall + Send + Sync;
}
