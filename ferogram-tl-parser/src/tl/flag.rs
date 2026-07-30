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

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
/// A conditional-field flag reference: the flags field name and the bit index.
pub struct Flag {
    /// The name of the flags field that holds this bit (usually `"flags"`).
    pub name: String,
    /// The bit index (0-based).
    pub index: u32,
}
