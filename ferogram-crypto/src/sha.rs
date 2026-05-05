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

#[macro_export]
macro_rules! sha1 {
    ( $( $x:expr ),+ ) => {{
        use sha1::{Digest, Sha1};
        let mut h = Sha1::new();
        $( h.update($x); )+
        let out: [u8; 20] = h.finalize().into();
        out
    }};
}

/// Calculate the SHA-256 hash of one or more byte slices concatenated.
#[macro_export]
macro_rules! sha256 {
    ( $( $x:expr ),+ ) => {{
        use sha2::{Digest, Sha256};
        let mut h = Sha256::new();
        $( h.update($x); )+
        let out: [u8; 32] = h.finalize().into();
        out
    }};
}
