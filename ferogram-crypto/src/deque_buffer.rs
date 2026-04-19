// Copyright (c) Ankit Chaubey <ankitchaubey.dev@gmail.com>
// SPDX-License-Identifier: MIT OR Apache-2.0
//
// ferogram: async Telegram MTProto client in Rust
// https://github.com/ankit-chaubey/ferogram
//
//
// If you use or modify this code, keep this notice at the top of your file
// and include the LICENSE-MIT or LICENSE-APACHE file from this repository:
// https://github.com/ankit-chaubey/ferogram

//! A deque-like growable buffer that supports efficient prepend.

use std::ops::{Index, IndexMut};
use std::slice::SliceIndex;

/// Growable byte buffer that supports efficient front-extension.
#[derive(Clone, Debug)]
pub struct DequeBuffer {
    buf: Vec<u8>,
    head: usize,
    default_head: usize,
}

impl DequeBuffer {
    /// Create with reserved space for `back` bytes in the back and `front` in the front.
    pub fn with_capacity(back: usize, front: usize) -> Self {
        let mut buf = Vec::with_capacity(front + back);
        buf.resize(front, 0);
        Self {
            buf,
            head: front,
            default_head: front,
        }
    }

    /// Reset the buffer to empty (but keep allocation).
    pub fn clear(&mut self) {
        self.buf.truncate(self.default_head);
        self.buf[..self.head].fill(0);
        self.head = self.default_head;
    }

    /// Prepend `slice` to the front.
    pub fn extend_front(&mut self, slice: &[u8]) {
        if self.head >= slice.len() {
            self.head -= slice.len();
        } else {
            let shift = slice.len() - self.head;
            self.buf.extend(std::iter::repeat_n(0, shift));
            self.buf.rotate_right(shift);
            self.head = 0;
        }
        self.buf[self.head..self.head + slice.len()].copy_from_slice(slice);
    }

    /// Number of bytes in the buffer.
    pub fn len(&self) -> usize {
        self.buf.len() - self.head
    }

    /// True if empty.
    pub fn is_empty(&self) -> bool {
        self.head == self.buf.len()
    }
}

impl AsRef<[u8]> for DequeBuffer {
    fn as_ref(&self) -> &[u8] {
        &self.buf[self.head..]
    }
}
impl AsMut<[u8]> for DequeBuffer {
    fn as_mut(&mut self) -> &mut [u8] {
        &mut self.buf[self.head..]
    }
}
impl<I: SliceIndex<[u8]>> Index<I> for DequeBuffer {
    type Output = I::Output;
    fn index(&self, i: I) -> &Self::Output {
        self.as_ref().index(i)
    }
}
impl<I: SliceIndex<[u8]>> IndexMut<I> for DequeBuffer {
    fn index_mut(&mut self, i: I) -> &mut Self::Output {
        self.as_mut().index_mut(i)
    }
}
impl Extend<u8> for DequeBuffer {
    fn extend<T: IntoIterator<Item = u8>>(&mut self, iter: T) {
        self.buf.extend(iter);
    }
}
impl<'a> Extend<&'a u8> for DequeBuffer {
    fn extend<T: IntoIterator<Item = &'a u8>>(&mut self, iter: T) {
        self.buf.extend(iter);
    }
}
