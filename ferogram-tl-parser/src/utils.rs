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

pub(crate) fn tl_id(definition: &str) -> u32 {
    // Strip the explicit #hexid tag if present (e.g. `boolFalse#bc799737 = Bool`
    // → `boolFalse = Bool`), but keep the type annotation after `=`.
    let cleaned = if let Some(hash_pos) = definition.find('#') {
        let after_hash = &definition[hash_pos + 1..];
        let id_len = after_hash
            .find(|c: char| !c.is_ascii_hexdigit())
            .unwrap_or(after_hash.len());
        let rest = &after_hash[id_len..];
        format!("{}{}", definition[..hash_pos].trim_end(), rest)
    } else {
        definition.to_owned()
    };
    crc32(cleaned.trim())
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
