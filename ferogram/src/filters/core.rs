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

/// A composable, synchronous predicate over an [`IncomingMessage`].
///
/// Use the built-in constructors ([`crate::filters::command`], [`crate::filters::private`], [`crate::filters::text`], ...) and
/// combine them with `&`, `|`, `!` operators rather than implementing this
/// trait directly. For arbitrary logic use [`crate::filters::custom`].
pub trait Filter: Send + Sync + 'static {
    fn check(&self, msg: &IncomingMessage) -> bool;
}

impl Filter for Arc<dyn Filter> {
    fn check(&self, msg: &IncomingMessage) -> bool {
        (**self).check(msg)
    }
}

/// A heap-allocated, cloneable, composable filter.
///
/// Returned by every built-in filter constructor. Supports `&`, `|`, and `!`
/// operators for building compound expressions.
#[derive(Clone)]
pub struct BoxFilter(Arc<dyn Filter>);

impl BoxFilter {
    pub(super) fn new<F: Filter>(f: F) -> Self {
        BoxFilter(Arc::new(f))
    }
}

impl Filter for BoxFilter {
    fn check(&self, msg: &IncomingMessage) -> bool {
        self.0.check(msg)
    }
}

impl std::ops::BitAnd for BoxFilter {
    type Output = BoxFilter;
    fn bitand(self, rhs: BoxFilter) -> BoxFilter {
        BoxFilter::new(AndFilter(self, rhs))
    }
}

impl std::ops::BitOr for BoxFilter {
    type Output = BoxFilter;
    fn bitor(self, rhs: BoxFilter) -> BoxFilter {
        BoxFilter::new(OrFilter(self, rhs))
    }
}

impl std::ops::Not for BoxFilter {
    type Output = BoxFilter;
    fn not(self) -> BoxFilter {
        BoxFilter::new(NotFilter(self))
    }
}

struct AndFilter(BoxFilter, BoxFilter);
impl Filter for AndFilter {
    fn check(&self, m: &IncomingMessage) -> bool {
        self.0.check(m) && self.1.check(m)
    }
}

struct OrFilter(BoxFilter, BoxFilter);
impl Filter for OrFilter {
    fn check(&self, m: &IncomingMessage) -> bool {
        self.0.check(m) || self.1.check(m)
    }
}

struct NotFilter(BoxFilter);
impl Filter for NotFilter {
    fn check(&self, m: &IncomingMessage) -> bool {
        !self.0.check(m)
    }
}

struct FnFilter(Arc<dyn Fn(&IncomingMessage) -> bool + Send + Sync + 'static>);
impl Filter for FnFilter {
    fn check(&self, m: &IncomingMessage) -> bool {
        (self.0)(m)
    }
}

pub(super) fn make<F>(f: F) -> BoxFilter
where
    F: Fn(&IncomingMessage) -> bool + Send + Sync + 'static,
{
    BoxFilter::new(FnFilter(Arc::new(f)))
}
