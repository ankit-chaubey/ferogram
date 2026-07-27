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

//! Thin indirection over the `metrics` crate so call sites don't need
//! per-site `#[cfg(feature = "metrics")]`. With the feature off, every
//! macro here expands to a zero-cost no-op handle.

#[cfg(feature = "metrics")]
macro_rules! counter {
    ($($arg:tt)*) => {
        ::metrics::counter!($($arg)*)
    };
}
#[cfg(feature = "metrics")]
macro_rules! histogram {
    ($($arg:tt)*) => {
        ::metrics::histogram!($($arg)*)
    };
}
#[cfg(feature = "metrics")]
macro_rules! gauge {
    ($($arg:tt)*) => {
        ::metrics::gauge!($($arg)*)
    };
}

#[cfg(not(feature = "metrics"))]
pub(crate) struct NoopMetric;

#[cfg(not(feature = "metrics"))]
impl NoopMetric {
    #[inline(always)]
    pub(crate) fn increment(&self, _value: u64) {}
    #[inline(always)]
    pub(crate) fn record(&self, _value: f64) {}
    #[inline(always)]
    pub(crate) fn set(&self, _value: f64) {}
}

#[cfg(not(feature = "metrics"))]
macro_rules! counter {
    ($($arg:tt)*) => {
        $crate::metrics_shim::NoopMetric
    };
}
#[cfg(not(feature = "metrics"))]
macro_rules! histogram {
    ($($arg:tt)*) => {
        $crate::metrics_shim::NoopMetric
    };
}
#[cfg(not(feature = "metrics"))]
macro_rules! gauge {
    ($($arg:tt)*) => {
        $crate::metrics_shim::NoopMetric
    };
}

pub(crate) use counter;
pub(crate) use gauge;
pub(crate) use histogram;
