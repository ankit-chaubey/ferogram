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

use std::sync::Arc;

use crate::update::IncomingMessage;

/// A composable, synchronous predicate over an update of type `T`.
///
/// Generic over the update kind so the same combinator machinery
/// (`&`, `|`, `!`) works for [`IncomingMessage`] (the default, used by
/// [`crate::filters::command`], [`crate::filters::private`],
/// [`crate::filters::text`], ...), [`crate::update::CallbackQuery`]
/// (via [`crate::filters::data`], [`crate::filters::data_prefix`],
/// [`crate::filters::data_regex`]), and [`crate::update::InlineQuery`]
/// (via [`crate::filters::inline_query_matches`]).
///
/// Use the built-in constructors and combine them with `&`, `|`, `!`
/// operators rather than implementing this trait directly. For arbitrary
/// logic use [`crate::filters::custom`].
pub trait Filter<T = IncomingMessage>: Send + Sync + 'static {
    fn check(&self, item: &T) -> bool;
}

impl<T: 'static> Filter<T> for Arc<dyn Filter<T>> {
    fn check(&self, item: &T) -> bool {
        (**self).check(item)
    }
}

/// A heap-allocated, cloneable, composable filter over updates of type `T`.
///
/// Defaults to `T = `[`IncomingMessage`] so every existing `BoxFilter`
/// usage (message filters, `Router`/`Dispatcher` message handlers) keeps
/// compiling unchanged. Returned by every built-in filter constructor.
/// Supports `&`, `|`, and `!` operators for building compound expressions.
pub struct BoxFilter<T = IncomingMessage>(Arc<dyn Filter<T>>);

impl<T> Clone for BoxFilter<T> {
    fn clone(&self) -> Self {
        BoxFilter(Arc::clone(&self.0))
    }
}

impl<T> BoxFilter<T> {
    pub(super) fn new<F: Filter<T>>(f: F) -> Self {
        BoxFilter(Arc::new(f))
    }
}

impl<T> Filter<T> for BoxFilter<T>
where
    T: 'static,
{
    fn check(&self, item: &T) -> bool {
        self.0.check(item)
    }
}

impl<T> std::ops::BitAnd for BoxFilter<T>
where
    T: 'static,
{
    type Output = BoxFilter<T>;
    fn bitand(self, rhs: BoxFilter<T>) -> BoxFilter<T> {
        BoxFilter::new(AndFilter(self, rhs))
    }
}

impl<T> std::ops::BitOr for BoxFilter<T>
where
    T: 'static,
{
    type Output = BoxFilter<T>;
    fn bitor(self, rhs: BoxFilter<T>) -> BoxFilter<T> {
        BoxFilter::new(OrFilter(self, rhs))
    }
}

impl<T> std::ops::Not for BoxFilter<T>
where
    T: 'static,
{
    type Output = BoxFilter<T>;
    fn not(self) -> BoxFilter<T> {
        BoxFilter::new(NotFilter(self))
    }
}

struct AndFilter<T>(BoxFilter<T>, BoxFilter<T>);
impl<T> Filter<T> for AndFilter<T>
where
    T: 'static,
{
    fn check(&self, m: &T) -> bool {
        self.0.check(m) && self.1.check(m)
    }
}

struct OrFilter<T>(BoxFilter<T>, BoxFilter<T>);
impl<T> Filter<T> for OrFilter<T>
where
    T: 'static,
{
    fn check(&self, m: &T) -> bool {
        self.0.check(m) || self.1.check(m)
    }
}

struct NotFilter<T>(BoxFilter<T>);
impl<T> Filter<T> for NotFilter<T>
where
    T: 'static,
{
    fn check(&self, m: &T) -> bool {
        !self.0.check(m)
    }
}

struct FnFilter<T>(Arc<dyn Fn(&T) -> bool + Send + Sync + 'static>);
impl<T> Filter<T> for FnFilter<T>
where
    T: 'static,
{
    fn check(&self, m: &T) -> bool {
        (self.0)(m)
    }
}

pub(super) fn make<T, F>(f: F) -> BoxFilter<T>
where
    T: 'static,
    F: Fn(&T) -> bool + Send + Sync + 'static,
{
    BoxFilter::new(FnFilter(Arc::new(f)))
}
