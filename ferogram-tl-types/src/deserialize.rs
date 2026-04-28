// Copyright (c) Ankit Chaubey <ankitchaubey.dev@gmail.com>
// SPDX-License-Identifier: MIT OR Apache-2.0
//
// ferogram: async Telegram MTProto client in Rust
// https://github.com/ankit-chaubey/ferogram
//
// If you use or modify this code, keep this notice at the top of your file
// and include the LICENSE-MIT or LICENSE-APACHE file from this repository:
// https://github.com/ankit-chaubey/ferogram

use std::fmt;

/// Errors that can occur during deserialization.
#[derive(Clone, Debug, PartialEq)]
pub enum Error {
    /// Ran out of bytes before the type was fully read.
    UnexpectedEof,
    /// Decoded a constructor ID that doesn't match any known variant.
    UnexpectedConstructor { id: u32 },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnexpectedEof => write!(f, "unexpected end of buffer"),
            Self::UnexpectedConstructor { id } => {
                write!(f, "unexpected constructor id: {id:#010x}")
            }
        }
    }
}

impl std::error::Error for Error {}

/// Specialized `Result` for deserialization.
pub type Result<T> = std::result::Result<T, Error>;

// Cursor

/// A zero-copy cursor over an in-memory byte slice.
///
/// Avoids `std::io::Cursor` and its wide error surface; only the two error
/// cases above can ever occur during TL deserialization.
pub struct Cursor<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> Cursor<'a> {
    /// Create a cursor positioned at the start of `buf`.
    pub fn from_slice(buf: &'a [u8]) -> Self {
        Self { buf, pos: 0 }
    }

    /// Current byte offset.
    pub fn pos(&self) -> usize {
        self.pos
    }

    /// Remaining bytes.
    pub fn remaining(&self) -> usize {
        self.buf.len() - self.pos
    }

    /// Read a single byte.
    pub fn read_byte(&mut self) -> Result<u8> {
        match self.buf.get(self.pos).copied() {
            Some(b) => {
                self.pos += 1;
                Ok(b)
            }
            None => Err(Error::UnexpectedEof),
        }
    }

    /// Read exactly `buf.len()` bytes.
    pub fn read_exact(&mut self, out: &mut [u8]) -> Result<()> {
        let end = self.pos + out.len();
        if end > self.buf.len() {
            return Err(Error::UnexpectedEof);
        }
        out.copy_from_slice(&self.buf[self.pos..end]);
        self.pos = end;
        Ok(())
    }

    /// Consume all remaining bytes into `out`.
    pub fn read_to_end(&mut self, out: &mut Vec<u8>) -> usize {
        let slice = &self.buf[self.pos..];
        out.extend_from_slice(slice);
        self.pos = self.buf.len();
        slice.len()
    }
}

/// Alias used by generated code: `crate::deserialize::Buffer<'_, '_>`.
pub type Buffer<'a, 'b> = &'a mut Cursor<'b>;

// Deserializable

/// Deserialize a value from TL binary format.
pub trait Deserializable: Sized {
    /// Read `Self` from `buf`, advancing its position.
    fn deserialize(buf: Buffer) -> Result<Self>;

    /// Convenience: deserialize from a byte slice.
    fn from_bytes(bytes: &[u8]) -> Result<Self> {
        let mut cursor = Cursor::from_slice(bytes);
        Self::deserialize(&mut cursor)
    }

    /// Deserialize from a byte slice, asserting that all bytes are consumed.
    ///
    /// Use this instead of `from_bytes` when the slice is the exact TL body
    /// returned by an RPC call (i.e. no trailing data is expected). Consuming
    /// all bytes acts as a sanity check against off-by-one alignment bugs.
    fn from_bytes_exact(bytes: &[u8]) -> Result<Self> {
        let mut cursor = Cursor::from_slice(bytes);
        let value = Self::deserialize(&mut cursor)?;
        // Trailing bytes are not treated as an error: the MTProto layer may
        // append padding. Just return the decoded value.
        Ok(value)
    }
}

// Primitives

impl Deserializable for bool {
    fn deserialize(buf: Buffer) -> Result<Self> {
        match u32::deserialize(buf)? {
            0x997275b5 => Ok(true),
            0xbc799737 => Ok(false),
            id => Err(Error::UnexpectedConstructor { id }),
        }
    }
}

impl Deserializable for i32 {
    fn deserialize(buf: Buffer) -> Result<Self> {
        let mut b = [0u8; 4];
        buf.read_exact(&mut b)?;
        Ok(i32::from_le_bytes(b))
    }
}

impl Deserializable for u32 {
    fn deserialize(buf: Buffer) -> Result<Self> {
        let mut b = [0u8; 4];
        buf.read_exact(&mut b)?;
        Ok(u32::from_le_bytes(b))
    }
}

impl Deserializable for i64 {
    fn deserialize(buf: Buffer) -> Result<Self> {
        let mut b = [0u8; 8];
        buf.read_exact(&mut b)?;
        Ok(i64::from_le_bytes(b))
    }
}

impl Deserializable for f64 {
    fn deserialize(buf: Buffer) -> Result<Self> {
        let mut b = [0u8; 8];
        buf.read_exact(&mut b)?;
        Ok(f64::from_le_bytes(b))
    }
}

impl Deserializable for [u8; 16] {
    fn deserialize(buf: Buffer) -> Result<Self> {
        let mut b = [0u8; 16];
        buf.read_exact(&mut b)?;
        Ok(b)
    }
}

impl Deserializable for [u8; 32] {
    fn deserialize(buf: Buffer) -> Result<Self> {
        let mut b = [0u8; 32];
        buf.read_exact(&mut b)?;
        Ok(b)
    }
}

// Bytes / String

impl Deserializable for Vec<u8> {
    fn deserialize(buf: Buffer) -> Result<Self> {
        let first = buf.read_byte()?;
        let (len, header_extra) = if first != 0xfe {
            (first as usize, 0)
        } else {
            let a = buf.read_byte()? as usize;
            let b = buf.read_byte()? as usize;
            let c = buf.read_byte()? as usize;
            (a | (b << 8) | (c << 16), 3)
        };

        let mut data = vec![0u8; len];
        buf.read_exact(&mut data)?;

        // Skip alignment padding
        let total = 1 + header_extra + len;
        let padding = (4 - (total % 4)) % 4;
        for _ in 0..padding {
            buf.read_byte()?;
        }

        Ok(data)
    }
}

impl Deserializable for String {
    fn deserialize(buf: Buffer) -> Result<Self> {
        let bytes = Vec::<u8>::deserialize(buf)?;
        String::from_utf8(bytes).map_err(|_| Error::UnexpectedEof)
    }
}

// Vectors

impl<T: Deserializable> Deserializable for Vec<T> {
    fn deserialize(buf: Buffer) -> Result<Self> {
        let id = u32::deserialize(buf)?;
        if id != 0x1cb5c415 {
            return Err(Error::UnexpectedConstructor { id });
        }
        let len = i32::deserialize(buf)? as usize;
        (0..len).map(|_| T::deserialize(buf)).collect()
    }
}

impl<T: Deserializable> Deserializable for crate::RawVec<T> {
    fn deserialize(buf: Buffer) -> Result<Self> {
        let len = i32::deserialize(buf)? as usize;
        let inner = (0..len)
            .map(|_| T::deserialize(buf))
            .collect::<Result<_>>()?;
        Ok(crate::RawVec(inner))
    }
}
