//! Canonical BCS primitives for consumer-defined bounded codecs.
//!
//! This module does not implement Serde or arbitrary collection serialization.
//! Consumers define field order and collection limits in their own codecs.

use core::str::from_utf8;

/// Fail-closed canonical encoding or decoding error.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CodecError {
    /// Caller-owned output buffer has insufficient capacity.
    Capacity,
    /// Input ended before the complete value.
    Truncated,
    /// Non-minimal ULEB128 or invalid boolean.
    NonCanonical,
    /// Sequence length or integer does not fit the BCS representation.
    Overflow,
    /// Extra bytes remain after decoding a complete value.
    Trailing,
    /// String contains invalid UTF-8.
    Utf8,
}

/// Consumer-controlled canonical field encoding. Implementations must enforce
/// their own collection and recursion bounds and preserve BCS field order.
pub trait CanonicalCodec: Sized {
    /// Encodes one value, without allocation.
    /// # Errors
    /// Returns a codec error when output capacity or model bounds are exceeded.
    fn encode(&self, output: &mut Encoder<'_>) -> Result<(), CodecError>;
    /// Decodes one value, without allocation.
    /// # Errors
    /// Returns a codec error for invalid, noncanonical or out-of-bounds values.
    fn decode(input: &mut Decoder<'_>) -> Result<Self, CodecError>;
}

/// Bounded writer. Each primitive write either succeeds completely or leaves
/// the cursor and buffer unchanged. Failed composite consumer writes may leave
/// a partial encoding; callers must discard output on any error.
pub struct Encoder<'buffer> {
    buffer: &'buffer mut [u8],
    position: usize,
}
impl<'buffer> Encoder<'buffer> {
    /// Borrows the complete output capacity.
    pub const fn new(buffer: &'buffer mut [u8]) -> Self {
        Self {
            buffer,
            position: 0,
        }
    }
    /// Returns the bytes written so far.
    #[must_use]
    pub fn written(&self) -> &[u8] {
        self.buffer.get(..self.position).unwrap_or_default()
    }
    /// Writes an already canonical fixed-width field.
    /// # Errors
    /// Returns `Capacity` without modifying output if the field does not fit.
    pub fn fixed(&mut self, bytes: &[u8]) -> Result<(), CodecError> {
        let end = self
            .position
            .checked_add(bytes.len())
            .ok_or(CodecError::Capacity)?;
        self.buffer
            .get_mut(self.position..end)
            .ok_or(CodecError::Capacity)?
            .copy_from_slice(bytes);
        self.position = end;
        Ok(())
    }
    /// Writes a canonical boolean.
    /// # Errors
    /// Returns `Capacity` if no byte remains.
    pub fn boolean(&mut self, value: bool) -> Result<(), CodecError> {
        self.fixed(&[u8::from(value)])
    }
    /// Writes a little-endian u32.
    /// # Errors
    /// Returns `Capacity` if four bytes do not fit.
    pub fn u32(&mut self, value: u32) -> Result<(), CodecError> {
        self.fixed(&value.to_le_bytes())
    }
    /// Writes a little-endian u64.
    /// # Errors
    /// Returns `Capacity` if eight bytes do not fit.
    pub fn u64(&mut self, value: u64) -> Result<(), CodecError> {
        self.fixed(&value.to_le_bytes())
    }
    /// Writes a BCS sequence length or enum discriminator.
    /// # Errors
    /// Returns `Capacity` if the minimal ULEB128 encoding does not fit.
    pub fn uleb(&mut self, value: u32) -> Result<(), CodecError> {
        let (bytes, count) = uleb_bytes(value);
        self.fixed(bytes.get(..count).ok_or(CodecError::Overflow)?)
    }
    /// Writes a length-prefixed byte sequence.
    /// # Errors
    /// Returns `Overflow` if length exceeds u32 or `Capacity` if output is full.
    pub fn bytes(&mut self, value: &[u8]) -> Result<(), CodecError> {
        let length = u32::try_from(value.len()).map_err(|_overflow| CodecError::Overflow)?;
        let (prefix, count) = uleb_bytes(length);
        let total = count.checked_add(value.len()).ok_or(CodecError::Capacity)?;
        let end = self
            .position
            .checked_add(total)
            .ok_or(CodecError::Capacity)?;
        if end > self.buffer.len() {
            return Err(CodecError::Capacity);
        }
        self.fixed(prefix.get(..count).ok_or(CodecError::Overflow)?)?;
        self.fixed(value)
    }
    /// Writes a BCS UTF-8 string.
    /// # Errors
    /// Returns the same errors as `bytes`.
    pub fn string(&mut self, value: &str) -> Result<(), CodecError> {
        self.bytes(value.as_bytes())
    }
}

/// Borrowed reader with no heap storage or recursive dispatch.
pub struct Decoder<'input> {
    remaining: &'input [u8],
}
impl<'input> Decoder<'input> {
    /// Borrows the complete encoding.
    #[must_use]
    pub const fn new(input: &'input [u8]) -> Self {
        Self { remaining: input }
    }
    /// Requires the complete value to consume all input.
    /// # Errors
    /// Returns `Trailing` when bytes remain.
    pub const fn finish(&self) -> Result<(), CodecError> {
        if self.remaining.is_empty() {
            Ok(())
        } else {
            Err(CodecError::Trailing)
        }
    }
    /// Reads a borrowed fixed-width field.
    /// # Errors
    /// Returns `Truncated` if the field is incomplete.
    pub fn fixed(&mut self, count: usize) -> Result<&'input [u8], CodecError> {
        let bytes = self.remaining.get(..count).ok_or(CodecError::Truncated)?;
        self.remaining = self.remaining.get(count..).ok_or(CodecError::Truncated)?;
        Ok(bytes)
    }
    /// Reads a canonical boolean; only bytes zero and one are accepted.
    /// # Errors
    /// Returns `Truncated` or `NonCanonical`.
    pub fn boolean(&mut self) -> Result<bool, CodecError> {
        match self.fixed(1)?.first().copied() {
            Some(0) => Ok(false),
            Some(1) => Ok(true),
            _ => Err(CodecError::NonCanonical),
        }
    }
    /// Reads a little-endian u32.
    /// # Errors
    /// Returns `Truncated` for an incomplete field.
    pub fn u32(&mut self) -> Result<u32, CodecError> {
        let bytes = self
            .fixed(4)?
            .try_into()
            .map_err(|_length| CodecError::Truncated)?;
        Ok(u32::from_le_bytes(bytes))
    }
    /// Reads a little-endian u64.
    /// # Errors
    /// Returns `Truncated` for an incomplete field.
    pub fn u64(&mut self) -> Result<u64, CodecError> {
        let bytes = self
            .fixed(8)?
            .try_into()
            .map_err(|_length| CodecError::Truncated)?;
        Ok(u64::from_le_bytes(bytes))
    }
    /// Reads a minimally encoded BCS sequence length or enum discriminator.
    /// # Errors
    /// Rejects truncated, overflowing and non-minimal ULEB128.
    pub fn uleb(&mut self) -> Result<u32, CodecError> {
        let mut value = 0_u32;
        for shift in [0, 7, 14, 21, 28] {
            let byte = self
                .fixed(1)?
                .first()
                .copied()
                .ok_or(CodecError::Truncated)?;
            if shift == 28 && byte > 15 {
                return Err(CodecError::Overflow);
            }
            value |= u32::from(byte & 127).wrapping_shl(shift);
            if byte & 128 == 0 {
                if shift != 0 && byte == 0 {
                    return Err(CodecError::NonCanonical);
                }
                return Ok(value);
            }
        }
        Err(CodecError::Overflow)
    }
    /// Reads a borrowed BCS byte sequence subject to the consumer's bound.
    /// # Errors
    /// Rejects invalid lengths, over-limit sequences and truncated input.
    pub fn bytes(&mut self, maximum: usize) -> Result<&'input [u8], CodecError> {
        let count = usize::try_from(self.uleb()?).map_err(|_overflow| CodecError::Overflow)?;
        if count > maximum {
            return Err(CodecError::Capacity);
        }
        self.fixed(count)
    }
    /// Reads a borrowed bounded BCS string.
    /// # Errors
    /// Returns a sequence error or `Utf8`.
    pub fn string(&mut self, maximum: usize) -> Result<&'input str, CodecError> {
        from_utf8(self.bytes(maximum)?).map_err(|_utf8| CodecError::Utf8)
    }
}

pub(crate) fn uleb_bytes(mut value: u32) -> ([u8; 5], usize) {
    let mut bytes = [0; 5];
    let mut count = 0_usize;
    for byte in &mut bytes {
        *byte = (value & 127) as u8;
        value >>= 7;
        if value != 0 {
            *byte |= 128;
        }
        count = count.saturating_add(1);
        if value == 0 {
            break;
        }
    }
    (bytes, count)
}
