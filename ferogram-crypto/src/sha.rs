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
