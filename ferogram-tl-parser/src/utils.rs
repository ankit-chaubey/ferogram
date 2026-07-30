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
