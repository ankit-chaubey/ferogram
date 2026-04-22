// Copyright (c) Ankit Chaubey <ankitchaubey.dev@gmail.com>
// SPDX-License-Identifier: MIT OR Apache-2.0
//
// ferogram: async Telegram MTProto client in Rust
// https://github.com/ankit-chaubey/ferogram
//
// If you use or modify this code, keep this notice at the top of your file
// and include the LICENSE-MIT or LICENSE-APACHE file from this repository:
// https://github.com/ankit-chaubey/ferogram

use std::str::FromStr;

use crate::errors::ParseError;
use crate::tl::{Category, Definition};

pub(crate) struct TlIterator<'a> {
    lines: std::str::Lines<'a>,
    /// Current category context: flips when we see `---functions---`.
    category: Category,
    /// Accumulates multi-line definitions (lines without `;` terminator).
    pending: String,
}

impl<'a> TlIterator<'a> {
    pub(crate) fn new(src: &'a str) -> Self {
        Self {
            lines: src.lines(),
            category: Category::Types,
            pending: String::new(),
        }
    }

    fn handle_separator(&mut self, line: &str) -> bool {
        let trimmed = line.trim();
        match trimmed {
            "---functions---" => {
                self.category = Category::Functions;
                true
            }
            "---types---" => {
                self.category = Category::Types;
                true
            }
            _ => false,
        }
    }
}

impl<'a> Iterator for TlIterator<'a> {
    type Item = Result<Definition, ParseError>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            let line = self.lines.next()?;
            let trimmed = line.trim();

            // Skip blanks and comments
            if trimmed.is_empty() || trimmed.starts_with("//") {
                continue;
            }

            // Category separators
            if self.handle_separator(trimmed) {
                continue;
            }

            // Accumulate multi-line definitions
            self.pending.push(' ');
            self.pending.push_str(trimmed);

            // A definition ends with `;`
            if !trimmed.ends_with(';') {
                continue;
            }

            // We have a complete definition: parse it
            let raw = std::mem::take(&mut self.pending);
            let raw = raw.trim();

            // Strip the trailing `;`
            let raw = raw.trim_end_matches(';').trim();

            if raw.is_empty() {
                continue;
            }

            let result = Definition::from_str(raw).map(|mut d| {
                d.category = self.category;
                d
            });

            return Some(result);
        }
    }
}
