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

//! Built-in filters over [`CallbackQuery`] and [`InlineQuery`].
//!
//! These use the same [`BoxFilter`]/`&`/`|`/`!` combinator machinery as the
//! message filters in [`super::builtins`], just parameterized over a
//! different update type. See [`crate::filters::Router::on_callback_query`]
//! and [`crate::filters::Router::on_inline_query`] for how to register them.

use regex::Regex;

use super::core::{BoxFilter, make};
use crate::update::{CallbackQuery, InlineQuery};

/// Callback data matches exactly.
///
/// # Example
/// ```rust,no_run
/// use ferogram::filters::data;
/// let close = data("close");
/// ```
pub fn data(value: impl Into<String>) -> BoxFilter<CallbackQuery> {
    let value = value.into();
    make(move |q: &CallbackQuery| q.data() == Some(value.as_str()))
}

/// Callback data starts with `prefix` (case-sensitive).
///
/// # Example
/// ```rust,no_run
/// use ferogram::filters::data_prefix;
/// let page = data_prefix("page:");
/// ```
pub fn data_prefix(prefix: impl Into<String>) -> BoxFilter<CallbackQuery> {
    let prefix = prefix.into();
    make(move |q: &CallbackQuery| q.data().is_some_and(|d| d.starts_with(prefix.as_str())))
}

/// Callback data matches a regular expression.
///
/// The pattern is compiled once, at filter-construction time. Panics if
/// `pattern` is not a valid regex -- construct filters at startup, not
/// inside a hot path, so a bad pattern fails fast and loudly.
///
/// # Example
/// ```rust,no_run
/// use ferogram::filters::data_regex;
/// let item = data_regex(r"^item_\d+$");
/// ```
pub fn data_regex(pattern: &str) -> BoxFilter<CallbackQuery> {
    let re = Regex::new(pattern).expect("data_regex: invalid regular expression");
    make(move |q: &CallbackQuery| q.data().is_some_and(|d| re.is_match(d)))
}

/// Inline query text matches a regular expression.
///
/// Panics if `pattern` is not a valid regex, for the same reason as
/// [`data_regex`].
///
/// # Example
/// ```rust,no_run
/// use ferogram::filters::inline_query_matches;
/// let cats = inline_query_matches(r"^cat:");
/// ```
pub fn inline_query_matches(pattern: &str) -> BoxFilter<InlineQuery> {
    let re = Regex::new(pattern).expect("inline_query_matches: invalid regular expression");
    make(move |q: &InlineQuery| re.is_match(q.query()))
}
