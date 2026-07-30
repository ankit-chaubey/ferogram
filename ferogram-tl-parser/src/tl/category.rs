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

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
/// Whether a TL definition is a data constructor or an RPC function.
pub enum Category {
    /// A concrete data constructor (the section before `---functions---`).
    Types,
    /// An RPC function definition (the section after `---functions---`).
    Functions,
}
