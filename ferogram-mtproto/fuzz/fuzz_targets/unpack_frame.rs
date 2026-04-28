//! Fuzz target: ferogram-mtproto EncryptedSession::unpack
//!
//! Exercises the full encrypted-frame unpack path including
//! auth_key_id extraction, msg_key derivation, and AES-IGE decryption.
//!
//! Run with:
//!   cargo fuzz run unpack_frame

#![no_main]

use libfuzzer_sys::fuzz_target;
use ferogram_mtproto::EncryptedSession;

fuzz_target!(|data: &[u8]| {
    let auth_key = [0u8; 256];
    let session_id: i64 = 0xdeadbeef_cafebabe_u64 as i64;
    // unpack takes &mut [u8]; must not panic on any input.
    let mut buf = data.to_vec();
    let mut sess = EncryptedSession::with_seen(auth_key, 0, 0, ferogram_mtproto::new_seen_msg_ids());
    let _ = sess.unpack(&mut buf);
});
