//! # StateFS Serde Deserialization Bridge
//!
//! Provides strongly-typed deserialization from StateFS in-memory trees into Rust structs
//! via `serde::Deserialize`.

extern crate alloc;

use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use core::fmt;

use serde::de::{
    self, Deserialize, DeserializeSeed, Deserializer, IntoDeserializer, MapAccess, SeqAccess,
    Visitor,
};
use statefs_core::{MemStore, Path, Store, Value};

/// Errors encountered while deserializing StateFS state into typed Rust structs.
#[derive(Debug)]
pub enum DeError {
    Custom(String),
    NotFound(String),
    TypeMismatch(String),
}

impl fmt::Display for DeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Custom(msg) => write!(f, "Deserialization error: {msg}"),
            Self::NotFound(path) => write!(f, "Node not found at path: {path}"),
            Self::TypeMismatch(msg) => write!(f, "Type mismatch: {msg}"),
        }
    }
}

impl std::error::Error for DeError {}

impl de::Error for DeError {
    fn custom<T: fmt::Display>(msg: T) -> Self {
        Self::Custom(msg.to_string())
    }
}

/// Converts a StateFS tree starting at `path` into a hierarchical `Value::Map` or scalar `Value`.
pub fn subtree_to_value(store: &MemStore, path_str: &str) -> Option<Value> {
    let path = Path::parse(path_str);
    let children = store.list_children(&path);

    let self_val = store.get(&path).map(|n| n.value.clone());

    if children.is_empty() {
        return self_val;
    }

    let mut map = BTreeMap::new();
    if let Some(val) = self_val
        && !val.is_null()
    {
        map.insert("_value".to_string(), val);
    }

    for child_path in children {
        if let Some(segment) = child_path.segments().last() {
            let child_str = child_path.to_string();
            if let Some(child_val) = subtree_to_value(store, &child_str) {
                map.insert(segment.to_string(), child_val);
            }
        }
    }

    Some(Value::Map(map))
}

/// Deserializes a typed Rust struct `T` from the StateFS store at the given path.
pub fn extract<T: for<'de> Deserialize<'de>>(store: &MemStore, path: &str) -> Result<T, DeError> {
    let val = subtree_to_value(store, path).ok_or_else(|| DeError::NotFound(path.to_string()))?;
    T::deserialize(ValueDeserializer::new(val))
}

/// Extension trait granting `extract` directly on `MemStore`.
pub trait StoreExtractExt {
    /// Extracts a strongly-typed struct `T` from the tree at `path`.
    fn extract<T: for<'de> Deserialize<'de>>(&self, path: &str) -> Result<T, DeError>;
}

impl StoreExtractExt for MemStore {
    fn extract<T: for<'de> Deserialize<'de>>(&self, path: &str) -> Result<T, DeError> {
        extract(self, path)
    }
}

/// Deserializer wrapping a StateFS [`Value`].
pub struct ValueDeserializer {
    value: Value,
}

impl ValueDeserializer {
    pub fn new(value: Value) -> Self {
        Self { value }
    }
}

impl<'de> Deserializer<'de> for ValueDeserializer {
    type Error = DeError;

    fn deserialize_any<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
        match self.value {
            Value::Null => visitor.visit_none(),
            Value::Bool(b) => visitor.visit_bool(b),
            Value::Int(i) => visitor.visit_i64(i),
            Value::Float(f) => visitor.visit_f64(f),
            Value::String(s) => visitor.visit_string(s),
            Value::Bytes(b) => visitor.visit_byte_buf(b),
            Value::Array(arr) => visitor.visit_seq(SeqDeserializer::new(arr)),
            Value::Map(map) => visitor.visit_map(MapDeserializer::new(map)),
        }
    }

    fn deserialize_bool<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
        match self.value {
            Value::Bool(b) => visitor.visit_bool(b),
            Value::Int(i) => visitor.visit_bool(i != 0),
            Value::String(s) => match s.to_lowercase().as_str() {
                "true" | "1" => visitor.visit_bool(true),
                "false" | "0" => visitor.visit_bool(false),
                _ => Err(DeError::TypeMismatch(format!(
                    "Cannot parse bool from '{s}'"
                ))),
            },
            _ => Err(DeError::TypeMismatch("Expected boolean".to_string())),
        }
    }

    fn deserialize_i64<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
        match self.value {
            Value::Int(i) => visitor.visit_i64(i),
            Value::Float(f) => visitor.visit_i64(f as i64),
            Value::String(s) => s
                .parse::<i64>()
                .map_err(|e| DeError::TypeMismatch(e.to_string()))
                .and_then(|i| visitor.visit_i64(i)),
            _ => Err(DeError::TypeMismatch("Expected integer".to_string())),
        }
    }

    fn deserialize_u64<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
        match self.value {
            Value::Int(i) if i >= 0 => visitor.visit_u64(i as u64),
            Value::Float(f) if f >= 0.0 => visitor.visit_u64(f as u64),
            Value::String(s) => s
                .parse::<u64>()
                .map_err(|e| DeError::TypeMismatch(e.to_string()))
                .and_then(|u| visitor.visit_u64(u)),
            _ => Err(DeError::TypeMismatch(
                "Expected unsigned integer".to_string(),
            )),
        }
    }

    fn deserialize_f64<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
        match self.value {
            Value::Float(f) => visitor.visit_f64(f),
            Value::Int(i) => visitor.visit_f64(i as f64),
            Value::String(s) => s
                .parse::<f64>()
                .map_err(|e| DeError::TypeMismatch(e.to_string()))
                .and_then(|f| visitor.visit_f64(f)),
            _ => Err(DeError::TypeMismatch("Expected float".to_string())),
        }
    }

    fn deserialize_str<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
        match self.value {
            Value::String(s) => visitor.visit_string(s),
            Value::Int(i) => visitor.visit_string(i.to_string()),
            Value::Float(f) => visitor.visit_string(f.to_string()),
            Value::Bool(b) => visitor.visit_string(b.to_string()),
            _ => Err(DeError::TypeMismatch("Expected string".to_string())),
        }
    }

    fn deserialize_string<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
        self.deserialize_str(visitor)
    }

    fn deserialize_option<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
        match self.value {
            Value::Null => visitor.visit_none(),
            _ => visitor.visit_some(self),
        }
    }

    fn deserialize_seq<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
        match self.value {
            Value::Array(arr) => visitor.visit_seq(SeqDeserializer::new(arr)),
            _ => Err(DeError::TypeMismatch("Expected sequence".to_string())),
        }
    }

    fn deserialize_map<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
        match self.value {
            Value::Map(map) => visitor.visit_map(MapDeserializer::new(map)),
            _ => Err(DeError::TypeMismatch("Expected map".to_string())),
        }
    }

    fn deserialize_struct<V: Visitor<'de>>(
        self,
        _name: &'static str,
        _fields: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, Self::Error> {
        self.deserialize_map(visitor)
    }

    serde::forward_to_deserialize_any! {
        i8 i16 i32 u8 u16 u32 f32 char bytes byte_buf unit unit_struct
        newtype_struct tuple tuple_struct enum identifier ignored_any
    }
}

struct SeqDeserializer {
    iter: alloc::vec::IntoIter<Value>,
}

impl SeqDeserializer {
    fn new(seq: Vec<Value>) -> Self {
        Self {
            iter: seq.into_iter(),
        }
    }
}

impl<'de> SeqAccess<'de> for SeqDeserializer {
    type Error = DeError;

    fn next_element_seed<T: DeserializeSeed<'de>>(
        &mut self,
        seed: T,
    ) -> Result<Option<T::Value>, Self::Error> {
        match self.iter.next() {
            Some(v) => seed.deserialize(ValueDeserializer::new(v)).map(Some),
            None => Ok(None),
        }
    }
}

struct MapDeserializer {
    iter: alloc::collections::btree_map::IntoIter<String, Value>,
    current_val: Option<Value>,
}

impl MapDeserializer {
    fn new(map: BTreeMap<String, Value>) -> Self {
        Self {
            iter: map.into_iter(),
            current_val: None,
        }
    }
}

impl<'de> MapAccess<'de> for MapDeserializer {
    type Error = DeError;

    fn next_key_seed<K: DeserializeSeed<'de>>(
        &mut self,
        seed: K,
    ) -> Result<Option<K::Value>, Self::Error> {
        match self.iter.next() {
            Some((k, v)) => {
                self.current_val = Some(v);
                seed.deserialize(k.into_deserializer()).map(Some)
            }
            None => Ok(None),
        }
    }

    fn next_value_seed<V: DeserializeSeed<'de>>(
        &mut self,
        seed: V,
    ) -> Result<V::Value, Self::Error> {
        match self.current_val.take() {
            Some(v) => seed.deserialize(ValueDeserializer::new(v)),
            None => Err(DeError::Custom(
                "Unexpected missing value in map".to_string(),
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;

    #[derive(Debug, PartialEq, Deserialize)]
    struct ServerConfig {
        name: String,
        tickrate: u32,
        max_players: i64,
        secure: bool,
    }

    #[derive(Debug, PartialEq, Deserialize)]
    struct RootConfig {
        server: ServerConfig,
    }

    #[test]
    fn test_extract_struct_roundtrip() {
        let mut store = MemStore::new();
        store
            .insert(&Path::parse("/server/name"), Value::from("CS 1.6 Server"))
            .unwrap();
        store
            .insert(&Path::parse("/server/tickrate"), Value::from(128))
            .unwrap();
        store
            .insert(&Path::parse("/server/max_players"), Value::from(32))
            .unwrap();
        store
            .insert(&Path::parse("/server/secure"), Value::from(true))
            .unwrap();

        let cfg: ServerConfig = store.extract("/server").expect("extract server config");
        assert_eq!(
            cfg,
            ServerConfig {
                name: "CS 1.6 Server".to_string(),
                tickrate: 128,
                max_players: 32,
                secure: true,
            }
        );

        let root: RootConfig = store.extract("/").expect("extract root config");
        assert_eq!(root.server.tickrate, 128);
    }
}
