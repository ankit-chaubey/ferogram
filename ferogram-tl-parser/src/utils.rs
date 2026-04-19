// Copyright (c) Ankit Chaubey <ankitchaubey.dev@gmail.com>
// SPDX-License-Identifier: MIT OR Apache-2.0
//
// ferogram: async Telegram MTProto client in Rust
// https://github.com/ankit-chaubey/ferogram
//
//
// If you use or modify this code, keep this notice at the top of your file
// and include the LICENSE-MIT or LICENSE-APACHE file from this repository:
// https://github.com/ankit-chaubey/ferogram

//! Utility to derive a TL constructor ID via CRC32 when no explicit `#id` is given.

/// Compute the CRC32-based TL constructor ID for a definition string.
///
/// Follows Telegram's TL naming algorithm: strips the `= ReturnType` suffix,
/// normalise whitespace, then CRC32 the result.
pub(crate) fn tl_id(definition: &str) -> u32 {
    // Strip everything from ` = ` onward (as Telegram does)
    let cleaned = match definition.split_once('=') {
        Some((lhs, _)) => lhs.trim().to_owned(),
        None => definition.trim().to_owned(),
    };
    crc32(&cleaned)
}

/// Standard CRC-32 (ISO 3309 / ITU-T V.42).
fn crc32(data: &str) -> u32 {
    let mut crc: u32 = 0xFFFF_FFFF;
    for byte in data.bytes() {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            if crc & 1 != 0 {
                crc = (crc >> 1) ^ 0xEDB8_8320;
            } else {
                crc >>= 1;
            }
        }
    }
    !crc
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_id() {
        // boolFalse#bc799737 = Bool: id must match when absent
        let def = "boolFalse = Bool";
        assert_eq!(tl_id(def), 0xbc799737);
    }
}
