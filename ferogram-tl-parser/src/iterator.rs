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

use std::str::FromStr;

use crate::errors::ParseError;
use crate::tl::{Category, Definition};

pub(crate) struct TlIterator<'a> {
    lines: std::str::Lines<'a>,
    /// Current category context: flips when we see `---functions---`.
    category: Category,
    /// Accumulates multi-line definitions (lines without `;` terminator).
    pending: String,
    /// Fully-split definitions ready to emit (result of splitting on `;`).
    ready: std::collections::VecDeque<Result<Definition, ParseError>>,
    /// Set to true once `lines` is exhausted, so we flush `pending` once.
    eof: bool,
}

impl<'a> TlIterator<'a> {
    pub(crate) fn new(src: &'a str) -> Self {
        Self {
            lines: src.lines(),
            category: Category::Types,
            pending: String::new(),
            ready: std::collections::VecDeque::new(),
            eof: false,
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

    /// Strip `//` inline comments from a raw line, then trim whitespace.
    fn strip_comment(line: &str) -> &str {
        let code = if let Some(idx) = line.find("//") {
            &line[..idx]
        } else {
            line
        };
        code.trim()
    }

    /// Flush `pending` as a complete definition string (no trailing `;` required).
    /// Splits on `;` so multi-def strings are each emitted separately.
    fn flush_pending(&mut self) {
        let raw = std::mem::take(&mut self.pending);
        for part in raw.split(';') {
            let part = part.trim();
            if part.is_empty() {
                continue;
            }
            let category = self.category;
            let result = Definition::from_str(part).map(|mut d| {
                d.category = category;
                d
            });
            self.ready.push_back(result);
        }
    }
}

impl<'a> Iterator for TlIterator<'a> {
    type Item = Result<Definition, ParseError>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            // Drain any definitions that are already parsed and queued.
            if let Some(item) = self.ready.pop_front() {
                return Some(item);
            }

            // Once EOF is reached, flush any leftover pending text once.
            if self.eof {
                if !self.pending.trim().is_empty() {
                    self.flush_pending();
                    continue; // loop back to drain ready
                }
                return None;
            }

            let line = match self.lines.next() {
                Some(l) => l,
                None => {
                    self.eof = true;
                    continue; // loop back to flush pending
                }
            };

            // Strip inline `//` comment, then trim whitespace.
            let trimmed = Self::strip_comment(line);

            // Skip blanks.
            if trimmed.is_empty() {
                continue;
            }

            // Category separators.
            if self.handle_separator(trimmed) {
                continue;
            }

            // Accumulate into pending.
            if !self.pending.is_empty() {
                self.pending.push(' ');
            }
            self.pending.push_str(trimmed);

            // If the accumulated text contains a `;`, flush it now so that
            // multi-def lines (e.g. `foo = A; bar = B;`) are split correctly.
            if self.pending.contains(';') {
                // Keep any trailing fragment after the last `;` in pending.
                let raw = std::mem::take(&mut self.pending);
                let mut parts = raw.split(';').peekable();
                while let Some(part) = parts.next() {
                    let part = part.trim();
                    if parts.peek().is_none() {
                        // Last segment: may be incomplete (no `;` yet), keep it.
                        if !part.is_empty() {
                            self.pending = part.to_string();
                        }
                    } else if !part.is_empty() {
                        let category = self.category;
                        let result = Definition::from_str(part).map(|mut d| {
                            d.category = category;
                            d
                        });
                        self.ready.push_back(result);
                    }
                }
                // Loop back to drain ready queue.
            }
        }
    }
}
