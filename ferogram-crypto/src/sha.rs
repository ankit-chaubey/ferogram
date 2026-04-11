// Copyright (c) Ankit Chaubey <ankitchaubey.dev@gmail.com>
// SPDX-License-Identifier: MIT OR Apache-2.0
//
// ferogram: async Telegram MTProto client in Rust
// https://github.com/ankit-chaubey/ferogram
//
// Based on layer: https://github.com/ankit-chaubey/layer
// Follows official Telegram client behaviour (tdesktop, TDLib).
//
// If you use or modify this code, keep this notice at the top of your file
// and include the LICENSE-MIT or LICENSE-APACHE file from this repository:
// https://github.com/ankit-chaubey/ferogram

/// Calculate the SHA-1 hash of one or more byte slices concatenated.
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
