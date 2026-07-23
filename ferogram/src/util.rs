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

/// E.164 caps a phone number at 15 digits - a hard, universal ceiling, not
/// a guess. Anything longer cannot be a valid phone number under any
/// numbering plan, so it's rejected here rather than spending a real
/// `contacts.importContacts` call (which has a side effect - see
/// [`PeerRef::Phone`]) on input that structurally cannot succeed.
///
/// [`PeerRef::Phone`]: crate::PeerRef::Phone
const MAX_PHONE_DIGITS: usize = 15;

/// Floor below which a `+`-prefixed string isn't worth trying as a phone
/// number. 5 covers real short numbers too, e.g. Telegram's own service
/// notification account, `+42777`.
const MIN_PHONE_DIGITS: usize = 5;

/// Normalize a phone number to canonical `+<digits>` form.
///
/// Accepts input with or without a leading `+`, and tolerates spaces,
/// hyphens, dots, and parentheses as separators (what people actually paste
/// when copying a number from another app). Returns `None` if any other
/// character is present, or if the digit count falls outside
/// `MIN_PHONE_DIGITS..=MAX_PHONE_DIGITS`.
///
/// This is the single source of truth for phone normalization: classifying
/// a `PeerRef::Phone` from user input, indexing `PeerCache::phone_to_user`
/// from Telegram's own `User.phone` field (digits only, no `+`), and the
/// `contacts.importContacts` RPC call all key off the same string, so they
/// must all normalize through this function to stay consistent.
pub(crate) fn normalize_phone(s: &str) -> Option<String> {
    let rest = s.strip_prefix('+').unwrap_or(s);

    // Already clean (the common case: no separators to strip) - one
    // allocation instead of filtering into a scratch buffer and
    // reformatting.
    if rest.bytes().all(|b| b.is_ascii_digit()) {
        return if (MIN_PHONE_DIGITS..=MAX_PHONE_DIGITS).contains(&rest.len()) {
            Some(format!("+{rest}"))
        } else {
            None
        };
    }

    // Messy input (spaces/hyphens/dots/parens present): build the "+digits"
    // result directly in one buffer instead of a scratch buffer that then
    // gets copied again into a formatted string.
    let mut out = String::with_capacity(rest.len() + 1);
    out.push('+');
    for c in rest.chars() {
        match c {
            '0'..='9' => out.push(c),
            ' ' | '-' | '.' | '(' | ')' => {}
            _ => return None,
        }
    }

    // out is "+" + digits, so digit count is out.len() - 1.
    let digit_count = out.len() - 1;
    if (MIN_PHONE_DIGITS..=MAX_PHONE_DIGITS).contains(&digit_count) {
        Some(out)
    } else {
        None
    }
}
