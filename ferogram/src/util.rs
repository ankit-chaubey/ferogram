// Copyright (c) Ankit Chaubey <ankitchaubey.dev@gmail.com>
// SPDX-License-Identifier: MIT OR Apache-2.0

use ferogram_tl_types::Deserializable;

use crate::InvocationError;

/// Strict TL decode helper.
///
/// Deserializes `T` from `body` and returns an error if:
/// - deserialization itself fails (wrong constructor, truncated data, etc.)
/// - any trailing bytes remain after decoding (misaligned schema)
///
/// This prevents:
/// - partial successful decode hiding schema mismatches
/// - trailing unread bytes causing fake random constructor failures later
/// - silent `.ok()` swallowing the real bug
///
/// Never use `tl::deserialize(...).ok()` for core MTProto paths; use this instead.
pub fn decode_checked<T>(name: &str, body: &[u8]) -> Result<T, InvocationError>
where
    T: Deserializable,
{
    let mut cursor = ferogram_tl_types::Cursor::from_slice(body);

    let value = T::deserialize(&mut cursor)
        .map_err(|e| InvocationError::Deserialize(format!("{name} deserialize error: {e}")))?;

    if cursor.remaining() != 0 {
        return Err(InvocationError::Deserialize(format!(
            "{name} trailing bytes after decode: {} byte(s) left",
            cursor.remaining()
        )));
    }

    Ok(value)
}
