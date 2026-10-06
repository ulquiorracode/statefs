//! Universal strongly typed value representation for StateFS nodes.

use alloc::borrow::ToOwned;
use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;
use core::fmt::{Display, Formatter, Result};

/// Universal strongly-typed value variant stored in a StateFS node.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Value {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    String(String),
    Bytes(Vec<u8>),
    Array(Vec<Value>),
    Map(BTreeMap<String, Value>),
}

impl Value {
    /// Returns `true` if the value is `Null`.
    #[inline]
    pub fn is_null(&self) -> bool {
        matches!(self, Self::Null)
    }

    /// Attempts to borrow as a boolean.
    #[inline]
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Self::Bool(b) => Some(*b),
            _ => None,
        }
    }

    /// Attempts to borrow as an integer.
    #[inline]
    pub fn as_int(&self) -> Option<i64> {
        match self {
            Self::Int(i) => Some(*i),
            _ => None,
        }
    }

    /// Attempts to borrow as a float (or converts integer to float).
    #[inline]
    pub fn as_float(&self) -> Option<f64> {
        match self {
            Self::Float(f) => Some(*f),
            Self::Int(i) => Some(*i as f64),
            _ => None,
        }
    }

    /// Attempts to borrow as a string slice.
    #[inline]
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Self::String(s) => Some(s.as_str()),
            _ => None,
        }
    }

    /// Attempts to borrow as raw byte slice.
    #[inline]
    pub fn as_bytes(&self) -> Option<&[u8]> {
        match self {
            Self::Bytes(b) => Some(b.as_slice()),
            _ => None,
        }
    }

    /// Attempts to borrow as an array of values.
    #[inline]
    pub fn as_array(&self) -> Option<&[Value]> {
        match self {
            Self::Array(a) => Some(a.as_slice()),
            _ => None,
        }
    }

    /// Attempts to borrow as a map.
    #[inline]
    pub fn as_map(&self) -> Option<&BTreeMap<String, Value>> {
        match self {
            Self::Map(m) => Some(m),
            _ => None,
        }
    }
}

impl Display for Value {
    fn fmt(&self, f: &mut Formatter<'_>) -> Result {
        match self {
            Self::Null => write!(f, "null"),
            Self::Bool(b) => write!(f, "{b}"),
            Self::Int(i) => write!(f, "{i}"),
            Self::Float(fl) => write!(f, "{fl}"),
            Self::String(s) => write!(f, "\"{s}\""),
            Self::Bytes(b) => write!(f, "<bytes len={}>", b.len()),
            Self::Array(a) => write!(f, "<array len={}>", a.len()),
            Self::Map(m) => write!(f, "<map len={}>", m.len()),
        }
    }
}

impl From<bool> for Value {
    fn from(b: bool) -> Self {
        Self::Bool(b)
    }
}

impl From<i32> for Value {
    fn from(i: i32) -> Self {
        Self::Int(i as i64)
    }
}

impl From<i64> for Value {
    fn from(i: i64) -> Self {
        Self::Int(i)
    }
}

impl From<u32> for Value {
    fn from(u: u32) -> Self {
        Self::Int(u as i64)
    }
}

impl From<f32> for Value {
    fn from(fl: f32) -> Self {
        Self::Float(fl as f64)
    }
}

impl From<f64> for Value {
    fn from(fl: f64) -> Self {
        Self::Float(fl)
    }
}

impl From<&str> for Value {
    fn from(s: &str) -> Self {
        Self::String(s.to_owned())
    }
}

impl From<String> for Value {
    fn from(s: String) -> Self {
        Self::String(s)
    }
}

impl From<Vec<u8>> for Value {
    fn from(b: Vec<u8>) -> Self {
        Self::Bytes(b)
    }
}

impl From<Vec<Value>> for Value {
    fn from(v: Vec<Value>) -> Self {
        Self::Array(v)
    }
}

impl From<BTreeMap<String, Value>> for Value {
    fn from(m: BTreeMap<String, Value>) -> Self {
        Self::Map(m)
    }
}

pub const VALUE_TAG_NULL: u8 = 0;
pub const VALUE_TAG_BOOL: u8 = 1;
pub const VALUE_TAG_INT: u8 = 2;
pub const VALUE_TAG_FLOAT: u8 = 3;
pub const VALUE_TAG_STRING: u8 = 4;
pub const VALUE_TAG_BYTES: u8 = 5;
pub const VALUE_TAG_ARRAY: u8 = 6;
pub const VALUE_TAG_MAP: u8 = 7;

const MAX_DECODE_DEPTH: usize = 32;
const MAX_CONTAINER_CAP: usize = 16384;

impl Value {
    /// Serializes value into a byte vector.
    pub fn encode_to_vec(&self) -> Vec<u8> {
        let mut buf = Vec::new();
        self.encode_into(&mut buf);
        buf
    }

    /// Serializes value into an existing byte buffer.
    pub fn encode_into(&self, buf: &mut Vec<u8>) {
        match self {
            Self::Null => {
                buf.push(VALUE_TAG_NULL);
            }
            Self::Bool(b) => {
                buf.push(VALUE_TAG_BOOL);
                buf.push(if *b { 1 } else { 0 });
            }
            Self::Int(i) => {
                buf.push(VALUE_TAG_INT);
                buf.extend_from_slice(&i.to_le_bytes());
            }
            Self::Float(f) => {
                buf.push(VALUE_TAG_FLOAT);
                buf.extend_from_slice(&f.to_le_bytes());
            }
            Self::String(s) => {
                buf.push(VALUE_TAG_STRING);
                let bytes = s.as_bytes();
                let len = bytes.len() as u32;
                buf.extend_from_slice(&len.to_le_bytes());
                buf.extend_from_slice(bytes);
            }
            Self::Bytes(b) => {
                buf.push(VALUE_TAG_BYTES);
                let len = b.len() as u32;
                buf.extend_from_slice(&len.to_le_bytes());
                buf.extend_from_slice(b);
            }
            Self::Array(items) => {
                buf.push(VALUE_TAG_ARRAY);
                let count = items.len() as u32;
                buf.extend_from_slice(&count.to_le_bytes());
                for item in items {
                    item.encode_into(buf);
                }
            }
            Self::Map(entries) => {
                buf.push(VALUE_TAG_MAP);
                let count = entries.len() as u32;
                buf.extend_from_slice(&count.to_le_bytes());
                for (k, v) in entries {
                    let k_bytes = k.as_bytes();
                    let k_len = k_bytes.len() as u16;
                    buf.extend_from_slice(&k_len.to_le_bytes());
                    buf.extend_from_slice(k_bytes);
                    v.encode_into(buf);
                }
            }
        }
    }

    /// Deserializes a Value from a byte slice at the given offset.
    pub fn decode_from(buf: &[u8], offset: &mut usize) -> Option<Self> {
        Self::decode_from_depth(buf, offset, 0)
    }

    fn decode_from_depth(buf: &[u8], offset: &mut usize, depth: usize) -> Option<Self> {
        if depth > MAX_DECODE_DEPTH || *offset >= buf.len() {
            return None;
        }

        let tag = buf[*offset];
        *offset += 1;

        match tag {
            VALUE_TAG_NULL => Some(Self::Null),
            VALUE_TAG_BOOL => {
                if *offset >= buf.len() {
                    return None;
                }
                let b = buf[*offset] != 0;
                *offset += 1;
                Some(Self::Bool(b))
            }
            VALUE_TAG_INT => {
                if *offset + 8 > buf.len() {
                    return None;
                }
                let i = i64::from_le_bytes(buf[*offset..*offset + 8].try_into().ok()?);
                *offset += 8;
                Some(Self::Int(i))
            }
            VALUE_TAG_FLOAT => {
                if *offset + 8 > buf.len() {
                    return None;
                }
                let f = f64::from_le_bytes(buf[*offset..*offset + 8].try_into().ok()?);
                *offset += 8;
                Some(Self::Float(f))
            }
            VALUE_TAG_STRING => {
                if *offset + 4 > buf.len() {
                    return None;
                }
                let s_len = u32::from_le_bytes(buf[*offset..*offset + 4].try_into().ok()?) as usize;
                *offset += 4;
                if *offset + s_len > buf.len() {
                    return None;
                }
                let s = core::str::from_utf8(&buf[*offset..*offset + s_len])
                    .ok()?
                    .to_owned();
                *offset += s_len;
                Some(Self::String(s))
            }
            VALUE_TAG_BYTES => {
                if *offset + 4 > buf.len() {
                    return None;
                }
                let b_len = u32::from_le_bytes(buf[*offset..*offset + 4].try_into().ok()?) as usize;
                *offset += 4;
                if *offset + b_len > buf.len() {
                    return None;
                }
                let b = buf[*offset..*offset + b_len].to_vec();
                *offset += b_len;
                Some(Self::Bytes(b))
            }
            VALUE_TAG_ARRAY => {
                if *offset + 4 > buf.len() {
                    return None;
                }
                let count = u32::from_le_bytes(buf[*offset..*offset + 4].try_into().ok()?) as usize;
                *offset += 4;
                if count > MAX_CONTAINER_CAP {
                    return None;
                }
                let mut items = Vec::with_capacity(count);
                for _ in 0..count {
                    items.push(Self::decode_from_depth(buf, offset, depth + 1)?);
                }
                Some(Self::Array(items))
            }
            VALUE_TAG_MAP => {
                if *offset + 4 > buf.len() {
                    return None;
                }
                let count = u32::from_le_bytes(buf[*offset..*offset + 4].try_into().ok()?) as usize;
                *offset += 4;
                if count > MAX_CONTAINER_CAP {
                    return None;
                }
                let mut map = BTreeMap::new();
                for _ in 0..count {
                    if *offset + 2 > buf.len() {
                        return None;
                    }
                    let k_len =
                        u16::from_le_bytes(buf[*offset..*offset + 2].try_into().ok()?) as usize;
                    *offset += 2;
                    if *offset + k_len > buf.len() {
                        return None;
                    }
                    let k = core::str::from_utf8(&buf[*offset..*offset + k_len])
                        .ok()?
                        .to_owned();
                    *offset += k_len;
                    let v = Self::decode_from_depth(buf, offset, depth + 1)?;
                    map.insert(k, v);
                }
                Some(Self::Map(map))
            }
            _ => None,
        }
    }
}
