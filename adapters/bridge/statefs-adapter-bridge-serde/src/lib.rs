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
    subtree_to_value_path(store, &path)
}

fn subtree_to_value_path(store: &MemStore, path: &Path) -> Option<Value> {
    let children = store.list_children(path);
    let self_val = store.get(path).map(|n| n.value.clone());

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
        if let Some(segment) = child_path.segments().last()
            && let Some(child_val) = subtree_to_value_path(store, &child_path)
        {
            map.insert(segment.to_string(), child_val);
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

    fn as_i64_val(&self) -> Result<i64, DeError> {
        match &self.value {
            Value::Int(i) => Ok(*i),
            Value::Float(f) => Ok(*f as i64),
            Value::String(s) => s
                .parse::<i64>()
                .map_err(|e| DeError::TypeMismatch(e.to_string())),
            _ => Err(DeError::TypeMismatch("Expected integer".to_string())),
        }
    }

    fn as_u64_val(&self) -> Result<u64, DeError> {
        match &self.value {
            Value::Int(i) if *i >= 0 => Ok(*i as u64),
            Value::Float(f) if *f >= 0.0 => Ok(*f as u64),
            Value::String(s) => s
                .parse::<u64>()
                .map_err(|e| DeError::TypeMismatch(e.to_string())),
            _ => Err(DeError::TypeMismatch(
                "Expected unsigned integer".to_string(),
            )),
        }
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

    fn deserialize_i8<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
        let i = self.as_i64_val()?;
        let val = i8::try_from(i).map_err(|e| DeError::TypeMismatch(format!("i8 overflow: {e}")))?;
        visitor.visit_i8(val)
    }

    fn deserialize_i16<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
        let i = self.as_i64_val()?;
        let val = i16::try_from(i).map_err(|e| DeError::TypeMismatch(format!("i16 overflow: {e}")))?;
        visitor.visit_i16(val)
    }

    fn deserialize_i32<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
        let i = self.as_i64_val()?;
        let val = i32::try_from(i).map_err(|e| DeError::TypeMismatch(format!("i32 overflow: {e}")))?;
        visitor.visit_i32(val)
    }

    fn deserialize_u8<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
        let u = self.as_u64_val()?;
        let val = u8::try_from(u).map_err(|e| DeError::TypeMismatch(format!("u8 overflow: {e}")))?;
        visitor.visit_u8(val)
    }

    fn deserialize_u16<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
        let u = self.as_u64_val()?;
        let val = u16::try_from(u).map_err(|e| DeError::TypeMismatch(format!("u16 overflow: {e}")))?;
        visitor.visit_u16(val)
    }

    fn deserialize_u32<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
        let u = self.as_u64_val()?;
        let val = u32::try_from(u).map_err(|e| DeError::TypeMismatch(format!("u32 overflow: {e}")))?;
        visitor.visit_u32(val)
    }

    fn deserialize_f32<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
        match self.value {
            Value::Float(f) => visitor.visit_f32(f as f32),
            Value::Int(i) => visitor.visit_f32(i as f32),
            Value::String(s) => s
                .parse::<f32>()
                .map_err(|e| DeError::TypeMismatch(e.to_string()))
                .and_then(|f| visitor.visit_f32(f)),
            _ => Err(DeError::TypeMismatch("Expected float".to_string())),
        }
    }

    fn deserialize_unit<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
        match self.value {
            Value::Null => visitor.visit_unit(),
            _ => Err(DeError::TypeMismatch("Expected null/unit".to_string())),
        }
    }

    fn deserialize_unit_struct<V: Visitor<'de>>(
        self,
        _name: &'static str,
        visitor: V,
    ) -> Result<V::Value, Self::Error> {
        self.deserialize_unit(visitor)
    }

    fn deserialize_newtype_struct<V: Visitor<'de>>(
        self,
        _name: &'static str,
        visitor: V,
    ) -> Result<V::Value, Self::Error> {
        visitor.visit_newtype_struct(self)
    }

    fn deserialize_enum<V: Visitor<'de>>(
        self,
        _name: &'static str,
        _variants: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, Self::Error> {
        match self.value {
            Value::String(s) => visitor.visit_enum(s.into_deserializer()),
            Value::Map(map) => {
                if map.len() == 1 {
                    let (variant, val) = map.into_iter().next().unwrap();
                    visitor.visit_enum(EnumMapAccess { variant, val })
                } else {
                    Err(DeError::TypeMismatch(
                        "Expected map with single variant key for enum".to_string(),
                    ))
                }
            }
            _ => Err(DeError::TypeMismatch("Expected string or map for enum".to_string())),
        }
    }

    serde::forward_to_deserialize_any! {
        char bytes byte_buf tuple tuple_struct identifier ignored_any
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

struct EnumMapAccess {
    variant: String,
    val: Value,
}

impl<'de> de::EnumAccess<'de> for EnumMapAccess {
    type Error = DeError;
    type Variant = VariantDeserializer;

    fn variant_seed<V: DeserializeSeed<'de>>(
        self,
        seed: V,
    ) -> Result<(V::Value, Self::Variant), Self::Error> {
        let variant = seed.deserialize(self.variant.into_deserializer())?;
        Ok((variant, VariantDeserializer { val: self.val }))
    }
}

struct VariantDeserializer {
    val: Value,
}

impl<'de> de::VariantAccess<'de> for VariantDeserializer {
    type Error = DeError;

    fn unit_variant(self) -> Result<(), Self::Error> {
        match self.val {
            Value::Null => Ok(()),
            _ => Err(DeError::TypeMismatch("Expected unit variant".to_string())),
        }
    }

    fn newtype_variant_seed<T: DeserializeSeed<'de>>(self, seed: T) -> Result<T::Value, Self::Error> {
        seed.deserialize(ValueDeserializer::new(self.val))
    }

    fn tuple_variant<V: Visitor<'de>>(self, _len: usize, visitor: V) -> Result<V::Value, Self::Error> {
        match self.val {
            Value::Array(arr) => visitor.visit_seq(SeqDeserializer::new(arr)),
            _ => Err(DeError::TypeMismatch("Expected tuple variant sequence".to_string())),
        }
    }

    fn struct_variant<V: Visitor<'de>>(
        self,
        _fields: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, Self::Error> {
        match self.val {
            Value::Map(map) => visitor.visit_map(MapDeserializer::new(map)),
            _ => Err(DeError::TypeMismatch("Expected struct variant map".to_string())),
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

    #[derive(Debug, PartialEq, Deserialize)]
    enum ServerMode {
        Deathmatch,
        Competitive,
        Custom(String),
    }

    #[derive(Debug, PartialEq, Deserialize)]
    struct ComplexModel {
        id: u8,
        mode: ServerMode,
        opt_field: Option<String>,
        none_field: Option<String>,
        unit_val: (),
        tags: Vec<String>,
    }

    #[test]
    fn test_adversarial_enum_and_optional_and_unit() {
        let mut store = MemStore::new();
        store.insert(&Path::parse("/c/id"), Value::from(255)).unwrap();
        store.insert(&Path::parse("/c/mode"), Value::from("Deathmatch")).unwrap();
        store.insert(&Path::parse("/c/opt_field"), Value::from("present")).unwrap();
        store.insert(&Path::parse("/c/none_field"), Value::Null).unwrap();
        store.insert(&Path::parse("/c/unit_val"), Value::Null).unwrap();
        store.insert(&Path::parse("/c/tags"), Value::Array(vec![Value::from("tag1"), Value::from("tag2")])).unwrap();

        let model: ComplexModel = store.extract("/c").expect("extract complex model");
        assert_eq!(
            model,
            ComplexModel {
                id: 255,
                mode: ServerMode::Deathmatch,
                opt_field: Some("present".to_string()),
                none_field: None,
                unit_val: (),
                tags: vec!["tag1".to_string(), "tag2".to_string()],
            }
        );
    }

    #[test]
    fn test_adversarial_numeric_overflow_detection() {
        let mut store = MemStore::new();
        store.insert(&Path::parse("/num/u8"), Value::from(300)).unwrap();
        let res: Result<ComplexModel, _> = store.extract("/num");
        assert!(res.is_err(), "Must reject u8 overflow");

        store.insert(&Path::parse("/num/u8"), Value::from(-1)).unwrap();
        let res2: Result<ComplexModel, _> = store.extract("/num");
        assert!(res2.is_err(), "Must reject negative value for u8");
    }

    #[test]
    fn test_adversarial_deeply_nested_tree() {
        let mut store = MemStore::new();
        let mut current_path = String::from("/root");
        for i in 0..10 {
            current_path = format!("{current_path}/level_{i}");
        }
        store.insert(&Path::parse(&current_path), Value::from("deep_leaf")).unwrap();

        let val = subtree_to_value(&store, "/root").expect("extract deeply nested tree as value");
        let mut cur = &val;
        for i in 0..10 {
            let map = cur.as_map().expect("must be map");
            let key = format!("level_{i}");
            cur = map.get(&key).expect("level must exist");
        }
        assert_eq!(cur.as_str(), Some("deep_leaf"));
    }

    #[test]
    fn test_adversarial_special_characters_and_empty_keys() {
        let mut store = MemStore::new();
        store.insert(&Path::parse("/weird/key with spaces"), Value::from(1)).unwrap();
        store.insert(&Path::parse("/weird/key.with.dots"), Value::from(2)).unwrap();
        store.insert(&Path::parse("/weird/@#$%^&*"), Value::from(3)).unwrap();

        use std::collections::HashMap;
        let map: HashMap<String, i64> = store.extract("/weird").expect("extract weird keys map");
        assert_eq!(map.get("key with spaces"), Some(&1));
        assert_eq!(map.get("key.with.dots"), Some(&2));
        assert_eq!(map.get("@#$%^&*"), Some(&3));
    }
}
