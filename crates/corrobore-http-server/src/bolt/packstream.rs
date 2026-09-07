// Copyright (c) 2026 AreDee-Bangs
// SPDX-License-Identifier: MIT
//
// Permission is hereby granted, free of charge, to any person obtaining a copy
// of this software and associated documentation files (the "Software"), to deal
// in the Software without restriction, including without limitation the rights
// to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
// copies of the Software, and to permit persons to whom the Software is
// furnished to do so, subject to the following conditions:
//
// The above copyright notice and this permission notice shall be included in
// all copies or substantial portions of the Software.
//
// THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
// IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
// FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
// AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
// LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
// OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN
// THE SOFTWARE.
//! PackStream version 1, the binary value encoding Bolt carries.
//!
//! Module boundary: this module turns a [`Value`] into bytes and bytes into a
//! [`Value`]. It knows nothing about messages, connections, or Corrobore
//! records; those live in the session layer, which chooses what to encode.
//!
//! Decoding is bounded. A message is already length-limited by the framing
//! layer, and the decoder additionally caps nesting depth and collection sizes
//! so a hostile client cannot make the server allocate ahead of the bytes it
//! actually sent.

use std::fmt;

/// Maximum nesting of lists, dictionaries and structures accepted on decode.
pub const MAX_DEPTH: usize = 64;

/// A PackStream value.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    /// Null.
    Null,
    /// Boolean.
    Boolean(bool),
    /// 64-bit signed integer.
    Integer(i64),
    /// IEEE 754 double.
    Float(f64),
    /// Byte array.
    Bytes(Vec<u8>),
    /// UTF-8 string.
    String(String),
    /// Ordered list.
    List(Vec<Value>),
    /// Dictionary in insertion order; keys are unique by construction on the
    /// encode side and last-wins on the decode side, as the specification says.
    Dictionary(Vec<(String, Value)>),
    /// Tagged structure.
    Structure {
        /// Tag byte.
        tag: u8,
        /// Fields.
        fields: Vec<Value>,
    },
}

impl Value {
    /// Look a key up in a dictionary value.
    #[must_use]
    pub fn get(&self, key: &str) -> Option<&Value> {
        match self {
            Self::Dictionary(entries) => entries
                .iter()
                .rev()
                .find(|(candidate, _)| candidate == key)
                .map(|(_, value)| value),
            _ => None,
        }
    }

    /// The string inside a string value.
    #[must_use]
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Self::String(text) => Some(text),
            _ => None,
        }
    }

    /// The integer inside an integer value.
    #[must_use]
    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Self::Integer(value) => Some(*value),
            _ => None,
        }
    }
}

/// Why a byte sequence is not a PackStream value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PackStreamError {
    /// The input ended inside a value.
    UnexpectedEnd,
    /// A marker byte has no meaning in PackStream v1.
    UnknownMarker(u8),
    /// A string is not UTF-8.
    InvalidUtf8,
    /// Nesting exceeded [`MAX_DEPTH`].
    TooDeep,
    /// A declared size exceeds the bytes available.
    SizeExceedsInput,
    /// A dictionary key is not a string.
    NonStringKey,
}

impl fmt::Display for PackStreamError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnexpectedEnd => formatter.write_str("PackStream value ends unexpectedly"),
            Self::UnknownMarker(marker) => {
                write!(formatter, "unknown PackStream marker {marker:#04x}")
            }
            Self::InvalidUtf8 => formatter.write_str("PackStream string is not UTF-8"),
            Self::TooDeep => write!(formatter, "PackStream nesting exceeds {MAX_DEPTH}"),
            Self::SizeExceedsInput => {
                formatter.write_str("PackStream size prefix exceeds the available bytes")
            }
            Self::NonStringKey => formatter.write_str("PackStream dictionary key is not a string"),
        }
    }
}

impl std::error::Error for PackStreamError {}

/// Append the encoding of `value` to `out`.
pub fn encode(value: &Value, out: &mut Vec<u8>) {
    match value {
        Value::Null => out.push(0xC0),
        Value::Boolean(false) => out.push(0xC2),
        Value::Boolean(true) => out.push(0xC3),
        Value::Integer(value) => encode_integer(*value, out),
        Value::Float(value) => {
            out.push(0xC1);
            out.extend_from_slice(&value.to_bits().to_be_bytes());
        }
        Value::Bytes(bytes) => {
            encode_size(bytes.len(), [0xCC, 0xCD, 0xCE], None, out);
            out.extend_from_slice(bytes);
        }
        Value::String(text) => {
            encode_size(text.len(), [0xD0, 0xD1, 0xD2], Some(0x80), out);
            out.extend_from_slice(text.as_bytes());
        }
        Value::List(items) => {
            encode_size(items.len(), [0xD4, 0xD5, 0xD6], Some(0x90), out);
            for item in items {
                encode(item, out);
            }
        }
        Value::Dictionary(entries) => {
            encode_size(entries.len(), [0xD8, 0xD9, 0xDA], Some(0xA0), out);
            for (key, value) in entries {
                encode(&Value::String(key.clone()), out);
                encode(value, out);
            }
        }
        Value::Structure { tag, fields } => {
            // Structures carry at most 15 fields in PackStream v1; every Bolt
            // message and graph structure fits, so a larger one is a caller bug.
            debug_assert!(
                fields.len() <= 15,
                "PackStream structure has at most 15 fields"
            );
            out.push(0xB0 | (fields.len() as u8 & 0x0F));
            out.push(*tag);
            for field in fields {
                encode(field, out);
            }
        }
    }
}

fn encode_integer(value: i64, out: &mut Vec<u8>) {
    if (-16..=127).contains(&value) {
        out.push(value as i8 as u8);
    } else if let Ok(value) = i8::try_from(value) {
        out.push(0xC8);
        out.push(value as u8);
    } else if let Ok(value) = i16::try_from(value) {
        out.push(0xC9);
        out.extend_from_slice(&value.to_be_bytes());
    } else if let Ok(value) = i32::try_from(value) {
        out.push(0xCA);
        out.extend_from_slice(&value.to_be_bytes());
    } else {
        out.push(0xCB);
        out.extend_from_slice(&value.to_be_bytes());
    }
}

/// Write a size prefix: the tiny marker when the size fits in a nibble and one
/// exists for the type, otherwise the 8, 16 or 32-bit marker.
fn encode_size(size: usize, markers: [u8; 3], tiny: Option<u8>, out: &mut Vec<u8>) {
    match (tiny, size) {
        (Some(tiny), size) if size < 16 => out.push(tiny | size as u8),
        (_, size) if size <= u8::MAX as usize => {
            out.push(markers[0]);
            out.push(size as u8);
        }
        (_, size) if size <= u16::MAX as usize => {
            out.push(markers[1]);
            out.extend_from_slice(&(size as u16).to_be_bytes());
        }
        (_, size) => {
            out.push(markers[2]);
            out.extend_from_slice(&(u32::try_from(size).unwrap_or(u32::MAX)).to_be_bytes());
        }
    }
}

/// Decode one value from the front of `bytes`.
///
/// Returns the value and the number of bytes it occupied, so a caller can check
/// that a message held exactly one value.
///
/// # Errors
/// [`PackStreamError`] for truncated, malformed or too deeply nested input.
pub fn decode(bytes: &[u8]) -> Result<(Value, usize), PackStreamError> {
    let mut cursor = Cursor { bytes, position: 0 };
    let value = cursor.value(0)?;
    Ok((value, cursor.position))
}

struct Cursor<'a> {
    bytes: &'a [u8],
    position: usize,
}

impl Cursor<'_> {
    fn byte(&mut self) -> Result<u8, PackStreamError> {
        let byte = *self
            .bytes
            .get(self.position)
            .ok_or(PackStreamError::UnexpectedEnd)?;
        self.position += 1;
        Ok(byte)
    }

    fn take(&mut self, length: usize) -> Result<&[u8], PackStreamError> {
        let end = self
            .position
            .checked_add(length)
            .ok_or(PackStreamError::SizeExceedsInput)?;
        if end > self.bytes.len() {
            return Err(PackStreamError::SizeExceedsInput);
        }
        let slice = &self.bytes[self.position..end];
        self.position = end;
        Ok(slice)
    }

    fn array<const N: usize>(&mut self) -> Result<[u8; N], PackStreamError> {
        // A fixed-width payload that is cut short is a truncated value, not a
        // hostile size prefix.
        if self.position + N > self.bytes.len() {
            return Err(PackStreamError::UnexpectedEnd);
        }
        let slice = self.take(N)?;
        let mut array = [0u8; N];
        array.copy_from_slice(slice);
        Ok(array)
    }

    fn size(&mut self, marker: u8, base: u8) -> Result<usize, PackStreamError> {
        Ok(match marker - base {
            0 => usize::from(self.byte()?),
            1 => usize::from(u16::from_be_bytes(self.array()?)),
            _ => usize::try_from(u32::from_be_bytes(self.array()?))
                .map_err(|_| PackStreamError::SizeExceedsInput)?,
        })
    }

    /// A collection size is only believable if that many single-byte values
    /// could still follow; anything larger is a hostile prefix.
    fn bounded(&self, size: usize) -> Result<usize, PackStreamError> {
        if size > self.bytes.len().saturating_sub(self.position) {
            return Err(PackStreamError::SizeExceedsInput);
        }
        Ok(size)
    }

    fn string(&mut self, length: usize) -> Result<String, PackStreamError> {
        let slice = self.take(length)?;
        String::from_utf8(slice.to_vec()).map_err(|_| PackStreamError::InvalidUtf8)
    }

    fn value(&mut self, depth: usize) -> Result<Value, PackStreamError> {
        if depth > MAX_DEPTH {
            return Err(PackStreamError::TooDeep);
        }
        let marker = self.byte()?;
        let value = match marker {
            0x00..=0x7F => Value::Integer(i64::from(marker)),
            0xF0..=0xFF => Value::Integer(i64::from(marker as i8)),
            0x80..=0x8F => Value::String(self.string(usize::from(marker & 0x0F))?),
            0x90..=0x9F => self.list(usize::from(marker & 0x0F), depth)?,
            0xA0..=0xAF => self.dictionary(usize::from(marker & 0x0F), depth)?,
            0xB0..=0xBF => {
                let count = usize::from(marker & 0x0F);
                let tag = self.byte()?;
                let mut fields = Vec::with_capacity(count);
                for _ in 0..count {
                    fields.push(self.value(depth + 1)?);
                }
                Value::Structure { tag, fields }
            }
            0xC0 => Value::Null,
            0xC1 => Value::Float(f64::from_bits(u64::from_be_bytes(self.array()?))),
            0xC2 => Value::Boolean(false),
            0xC3 => Value::Boolean(true),
            0xC8 => Value::Integer(i64::from(self.byte()? as i8)),
            0xC9 => Value::Integer(i64::from(i16::from_be_bytes(self.array()?))),
            0xCA => Value::Integer(i64::from(i32::from_be_bytes(self.array()?))),
            0xCB => Value::Integer(i64::from_be_bytes(self.array()?)),
            0xCC..=0xCE => {
                let length = self.size(marker, 0xCC)?;
                Value::Bytes(self.take(length)?.to_vec())
            }
            0xD0..=0xD2 => {
                let length = self.size(marker, 0xD0)?;
                Value::String(self.string(length)?)
            }
            0xD4..=0xD6 => {
                let count = self.size(marker, 0xD4)?;
                self.list(count, depth)?
            }
            0xD8..=0xDA => {
                let count = self.size(marker, 0xD8)?;
                self.dictionary(count, depth)?
            }
            other => return Err(PackStreamError::UnknownMarker(other)),
        };
        Ok(value)
    }

    fn list(&mut self, count: usize, depth: usize) -> Result<Value, PackStreamError> {
        let count = self.bounded(count)?;
        let mut items = Vec::with_capacity(count);
        for _ in 0..count {
            items.push(self.value(depth + 1)?);
        }
        Ok(Value::List(items))
    }

    fn dictionary(&mut self, count: usize, depth: usize) -> Result<Value, PackStreamError> {
        let count = self.bounded(count)?;
        let mut entries: Vec<(String, Value)> = Vec::with_capacity(count);
        for _ in 0..count {
            let Value::String(key) = self.value(depth + 1)? else {
                return Err(PackStreamError::NonStringKey);
            };
            let value = self.value(depth + 1)?;
            // Last value wins for a repeated key, and the dictionary keeps one
            // entry per key so lookups stay unambiguous.
            if let Some(existing) = entries.iter_mut().find(|(existing, _)| *existing == key) {
                existing.1 = value;
            } else {
                entries.push((key, value));
            }
        }
        Ok(Value::Dictionary(entries))
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    fn round_trip(value: Value) {
        let mut bytes = Vec::new();
        encode(&value, &mut bytes);
        let (decoded, consumed) = decode(&bytes).unwrap();
        assert_eq!(consumed, bytes.len());
        assert_eq!(decoded, value);
    }

    #[test]
    fn integers_use_the_smallest_encoding() {
        let cases: [(i64, Vec<u8>); 7] = [
            (0, vec![0x00]),
            (127, vec![0x7F]),
            (-16, vec![0xF0]),
            (-17, vec![0xC8, 0xEF]),
            (128, vec![0xC9, 0x00, 0x80]),
            (-32_769, vec![0xCA, 0xFF, 0xFF, 0x7F, 0xFF]),
            (2_147_483_648, vec![0xCB, 0, 0, 0, 0, 0x80, 0, 0, 0]),
        ];
        for (value, expected) in cases {
            let mut bytes = Vec::new();
            encode(&Value::Integer(value), &mut bytes);
            assert_eq!(bytes, expected, "{value}");
            round_trip(Value::Integer(value));
        }
    }

    #[test]
    fn strings_lists_dictionaries_and_structures_round_trip_across_size_classes() {
        round_trip(Value::String(String::new()));
        round_trip(Value::String("é".repeat(10)));
        round_trip(Value::String("x".repeat(300)));
        round_trip(Value::String("x".repeat(70_000)));
        round_trip(Value::Bytes((0..=255).collect()));
        round_trip(Value::List((0..20).map(Value::Integer).collect()));
        round_trip(Value::Dictionary(
            (0..300)
                .map(|index| (format!("k{index}"), Value::Boolean(index % 2 == 0)))
                .collect(),
        ));
        round_trip(Value::Structure {
            tag: b'N',
            fields: vec![
                Value::Integer(1),
                Value::List(vec![Value::String("Label".into())]),
                Value::Dictionary(vec![("k".into(), Value::Float(1.5))]),
                Value::String("node--1".into()),
            ],
        });
        round_trip(Value::Null);
        round_trip(Value::Float(-0.0));
    }

    #[test]
    fn decoding_is_bounded_and_reports_malformed_input() {
        assert_eq!(decode(&[]), Err(PackStreamError::UnexpectedEnd));
        assert_eq!(decode(&[0xC9, 0x00]), Err(PackStreamError::UnexpectedEnd));
        assert_eq!(decode(&[0xC7]), Err(PackStreamError::UnknownMarker(0xC7)));
        assert_eq!(decode(&[0x81, 0xFF]), Err(PackStreamError::InvalidUtf8));
        // A list claiming four billion entries with two bytes left is refused
        // before any allocation.
        assert_eq!(
            decode(&[0xD6, 0xFF, 0xFF, 0xFF, 0xFF, 0x01]),
            Err(PackStreamError::SizeExceedsInput)
        );
        assert_eq!(
            decode(&[0xA1, 0x01, 0x01]),
            Err(PackStreamError::NonStringKey)
        );
        let mut nested = vec![0x91u8; MAX_DEPTH + 2];
        nested.push(0x01);
        assert_eq!(decode(&nested), Err(PackStreamError::TooDeep));
    }

    #[test]
    fn repeated_dictionary_keys_keep_the_last_value() {
        let bytes = [0xA2, 0x81, b'k', 0x01, 0x81, b'k', 0x02];
        let (value, _) = decode(&bytes).unwrap();
        assert_eq!(
            value,
            Value::Dictionary(vec![("k".into(), Value::Integer(2))])
        );
        assert_eq!(value.get("k"), Some(&Value::Integer(2)));
    }
}
