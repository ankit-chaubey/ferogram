/*
 * Copyright (c) 2026 Ankit Chaubey <ankitchaubey.dev@gmail.com>
 * https://github.com/ankit-chaubey
 *
 * Project: ferogram
 * Website: https://ferogram.dev
 *
 * Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
 * https://www.apache.org/licenses/LICENSE-2.0> or the MIT license
 * <LICENSE-MIT or https://opensource.org/licenses/MIT>, at your option.
 * This file may not be copied, modified, or distributed except according
 * to those terms.
 */

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
