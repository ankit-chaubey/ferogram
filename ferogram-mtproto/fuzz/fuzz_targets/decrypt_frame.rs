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
