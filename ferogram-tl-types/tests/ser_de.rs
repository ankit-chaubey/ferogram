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

use ferogram_tl_types::{Cursor, Deserializable, Serializable};

// Primitive round-trips

#[test]
fn roundtrip_i32() {
    for v in [0i32, -1, i32::MAX, i32::MIN, 42] {
        let bytes = v.to_bytes();
        assert_eq!(i32::from_bytes(&bytes).unwrap(), v);
    }
}

#[test]
fn roundtrip_i64() {
    for v in [0i64, -1, i64::MAX, i64::MIN, 1_234_567_890] {
        let bytes = v.to_bytes();
        assert_eq!(i64::from_bytes(&bytes).unwrap(), v);
    }
}

#[test]
fn roundtrip_bool_true() {
    let bytes = true.to_bytes();
    assert_eq!(bytes, 0x997275b5u32.to_le_bytes());
    assert_eq!(bool::from_bytes(&bytes).unwrap(), true);
}

#[test]
fn roundtrip_bool_false() {
    let bytes = false.to_bytes();
    assert_eq!(bytes, 0xbc799737u32.to_le_bytes());
    assert_eq!(bool::from_bytes(&bytes).unwrap(), false);
}

// String / bytes

#[test]
fn roundtrip_empty_string() {
    let s = String::new();
    let bytes = s.to_bytes();
    assert_eq!(String::from_bytes(&bytes).unwrap(), s);
}

#[test]
fn roundtrip_short_string() {
    let s = "hello world".to_owned();
    let bytes = s.to_bytes();
    assert_eq!(bytes.len() % 4, 0, "must be 4-byte aligned");
    assert_eq!(String::from_bytes(&bytes).unwrap(), s);
}

#[test]
fn roundtrip_long_string() {
    // >253 bytes triggers the 4-byte length header path
    let s = "x".repeat(300);
    let bytes = s.clone().to_bytes();
    assert_eq!(String::from_bytes(&bytes).unwrap(), s);
}

#[test]
fn roundtrip_bytes_vec() {
    let v: Vec<u8> = (0u8..=255).collect();
    let bytes = v.clone().to_bytes();
    assert_eq!(Vec::<u8>::from_bytes(&bytes).unwrap(), v);
}

// Vectors

#[test]
fn roundtrip_vec_i32() {
    let v: Vec<i32> = vec![1, 2, 3, -99];
    let bytes = v.to_bytes();
    assert_eq!(Vec::<i32>::from_bytes(&bytes).unwrap(), vec![1, 2, 3, -99]);
}

#[test]
fn roundtrip_empty_vec() {
    let v: Vec<i64> = vec![];
    let bytes = v.to_bytes();
    assert_eq!(Vec::<i64>::from_bytes(&bytes).unwrap(), Vec::<i64>::new());
}

// Fixed-size arrays

#[test]
fn roundtrip_int128() {
    let v: [u8; 16] = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16];
    assert_eq!(<[u8; 16]>::from_bytes(&v.to_bytes()).unwrap(), v);
}

#[test]
fn roundtrip_int256() {
    let v: [u8; 32] = core::array::from_fn(|i| i as u8);
    assert_eq!(<[u8; 32]>::from_bytes(&v.to_bytes()).unwrap(), v);
}

// Cursor EOF detection

#[test]
fn deserialize_truncated_returns_eof() {
    use ferogram_tl_types::deserialize::Error;
    let result = i32::from_bytes(&[0x01, 0x02]); // only 2 bytes, need 4
    assert_eq!(result, Err(Error::UnexpectedEof));
}

// Option passthrough

#[test]
fn option_none_writes_nothing() {
    let v: Option<i32> = None;
    assert_eq!(v.to_bytes(), b"");
}

#[test]
fn option_some_writes_inner() {
    let v: Option<i32> = Some(42);
    assert_eq!(v.to_bytes(), 42i32.to_bytes());
}
