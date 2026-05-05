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
