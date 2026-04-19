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

//! The [`Serializable`] trait and its implementations for primitive TL types.
//!
//! Encoding follows the [MTProto Binary Serialization] spec.
//!
//! [MTProto Binary Serialization]: https://core.telegram.org/mtproto/serialize

/// Serialize `self` into TL binary format.
pub trait Serializable {
    /// Appends the serialized form of `self` to `buf`.
    fn serialize(&self, buf: &mut impl Extend<u8>);

    /// Convenience: allocate a fresh `Vec<u8>` and serialize into it.
    fn to_bytes(&self) -> Vec<u8> {
        let mut v = Vec::new();
        self.serialize(&mut v);
        v
    }
}

// bool

/// `true`  → `boolTrue#997275b5`
/// `false` → `boolFalse#bc799737`
impl Serializable for bool {
    fn serialize(&self, buf: &mut impl Extend<u8>) {
        let id: u32 = if *self { 0x997275b5 } else { 0xbc799737 };
        id.serialize(buf);
    }
}

// integers

impl Serializable for i32 {
    fn serialize(&self, buf: &mut impl Extend<u8>) {
        buf.extend(self.to_le_bytes());
    }
}

impl Serializable for u32 {
    fn serialize(&self, buf: &mut impl Extend<u8>) {
        buf.extend(self.to_le_bytes());
    }
}

impl Serializable for i64 {
    fn serialize(&self, buf: &mut impl Extend<u8>) {
        buf.extend(self.to_le_bytes());
    }
}

impl Serializable for f64 {
    fn serialize(&self, buf: &mut impl Extend<u8>) {
        buf.extend(self.to_le_bytes());
    }
}

impl Serializable for [u8; 16] {
    fn serialize(&self, buf: &mut impl Extend<u8>) {
        buf.extend(self.iter().copied());
    }
}

impl Serializable for [u8; 32] {
    fn serialize(&self, buf: &mut impl Extend<u8>) {
        buf.extend(self.iter().copied());
    }
}

// strings / bytes

/// TL string encoding: a length-prefixed, 4-byte aligned byte string.
///
/// * If `len ≤ 253`: `[len as u8][data][0-padding to align to 4 bytes]`
/// * If `len ≥ 254`: `[0xfe][len as 3 LE bytes][data][0-padding]`
impl Serializable for &[u8] {
    fn serialize(&self, buf: &mut impl Extend<u8>) {
        let len = self.len();
        let (header_len, header): (usize, Vec<u8>) = if len <= 253 {
            (1, vec![len as u8])
        } else {
            (
                4,
                vec![
                    0xfe,
                    (len & 0xff) as u8,
                    ((len >> 8) & 0xff) as u8,
                    ((len >> 16) & 0xff) as u8,
                ],
            )
        };

        let total = header_len + len;
        let padding = (4 - (total % 4)) % 4;

        buf.extend(header);
        buf.extend(self.iter().copied());
        buf.extend(std::iter::repeat_n(0u8, padding));
    }
}

impl Serializable for Vec<u8> {
    fn serialize(&self, buf: &mut impl Extend<u8>) {
        self.as_slice().serialize(buf);
    }
}

impl Serializable for String {
    fn serialize(&self, buf: &mut impl Extend<u8>) {
        self.as_bytes().serialize(buf);
    }
}

// vectors

/// Boxed `Vector<T>`: prefixed with constructor ID `0x1cb5c415`.
impl<T: Serializable> Serializable for Vec<T> {
    fn serialize(&self, buf: &mut impl Extend<u8>) {
        0x1cb5c415u32.serialize(buf);
        (self.len() as i32).serialize(buf);
        for item in self {
            item.serialize(buf);
        }
    }
}

/// Bare `vector<T>`: just a count followed by items, no constructor ID.
impl<T: Serializable> Serializable for crate::RawVec<T> {
    fn serialize(&self, buf: &mut impl Extend<u8>) {
        (self.0.len() as i32).serialize(buf);
        for item in &self.0 {
            item.serialize(buf);
        }
    }
}

// Option

/// Optional parameters are handled by flags; when `Some`, serialize the value.
/// When `None`, nothing is written (the flags word already encodes absence).
impl<T: Serializable> Serializable for Option<T> {
    fn serialize(&self, buf: &mut impl Extend<u8>) {
        if let Some(v) = self {
            v.serialize(buf);
        }
    }
}
