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

//! Fuzz target: ferogram-mtproto frame decryption
//!
//! Verifies that no arbitrary input to `EncryptedSession::decrypt_frame` can
//! cause a panic; only `Err` returns are permitted.
//!
//! Run with:
//!   cargo fuzz run decrypt_frame
//!
//! Requires the `cargo-fuzz` toolchain:
//!   cargo install cargo-fuzz

#![no_main]

use libfuzzer_sys::fuzz_target;
use ferogram_mtproto::EncryptedSession;

/// A fixed all-zero auth key used purely to exercise parsing paths.
/// Real auth keys are 256 bytes of high-entropy DH output; zero is fine for fuzzing.
fn dummy_auth_key() -> [u8; 256] {
    [0u8; 256]
}

fuzz_target!(|data: &[u8]| {
    // Must never panic; only return Ok(_) or Err(_).
    let key = dummy_auth_key();
    let session_id: i64 = 0x1234_5678_9abc_def0;
    let mut buf = data.to_vec();
    let _ = EncryptedSession::decrypt_frame(&key, session_id, &mut buf);
});
