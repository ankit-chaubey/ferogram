// Copyright (c) Ankit Chaubey <ankitchaubey.dev@gmail.com>
// SPDX-License-Identifier: MIT OR Apache-2.0
//
// ferogram: async Telegram MTProto client in Rust
// https://github.com/ankit-chaubey/ferogram
//
// If you use or modify this code, keep this notice at the top of your file
// and include the LICENSE-MIT or LICENSE-APACHE file from this repository:
// https://github.com/ankit-chaubey/ferogram

#[macro_export]
macro_rules! dispatch {
    // Entry point: client, update, then one or more arms
    ($client:expr, $update:expr, $( $pattern:tt )+ ) => {
        match $update {
            $crate::__dispatch_arms!($client; $( $pattern )+ )
        }
    };
}

/// Internal helper: do not use directly.
#[macro_export]
#[doc(hidden)]
macro_rules! __dispatch_arms {
    // Catch-all arm
    ($client:expr; _ => $body:block $( , $( $rest:tt )* )? ) => {
        _ => $body
    };

    // Variant arm WITH guard
    ($client:expr;
        $variant:ident ( $binding:pat ) if $guard:expr => $body:block
        $( , $( $rest:tt )* )?
    ) => {
        $crate::update::Update::$variant($binding) if $guard => $body,
        $( $crate::__dispatch_arms!($client; $( $rest )* ) )?
    };

    // Variant arm WITHOUT guard
    ($client:expr;
        $variant:ident ( $binding:pat ) => $body:block
        $( , $( $rest:tt )* )?
    ) => {
        $crate::update::Update::$variant($binding) => $body,
        $( $crate::__dispatch_arms!($client; $( $rest )* ) )?
    };

    // Trailing comma / empty: emit wildcard to make sure exhaustiveness
    ($client:expr; $(,)?) => {
        _ => {}
    };
}
