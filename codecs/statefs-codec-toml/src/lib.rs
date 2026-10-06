//! # StateFS TOML Codec
//!
//! Encodes and decodes hierarchical state trees to and from TOML format.

use statefs_core::{MemStore, Path, Store, StoreError, Value};

/// Errors occurring during TOML ingestion or parsing.
#[derive(Debug)]
pub enum TomlCodecError {
    De(toml::de::Error),
    Store(StoreError),
}

impl core::fmt::Display for TomlCodecError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::De(e) => write!(f, "TOML deserialization error: {e}"),
            Self::Store(e) => write!(f, "StateFS store error: {e}"),
        }
    }
}

impl std::error::Error for TomlCodecError {}

impl From<toml::de::Error> for TomlCodecError {
    fn from(e: toml::de::Error) -> Self {
        Self::De(e)
    }
}

impl From<StoreError> for TomlCodecError {
    fn from(e: StoreError) -> Self {
        Self::Store(e)
    }
}

extern crate alloc;

use alloc::string::ToString;
use alloc::vec::Vec;

/// Maximum recursion depth allowed during TOML traversal to prevent stack exhaustion.
const MAX_TOML_DEPTH: usize = 32;

/// Ingests a raw TOML string into a [`MemStore`] under the specified base path prefix.
pub fn ingest_toml_str(
    store: &mut MemStore,
    prefix: &str,
    raw_toml: &str,
) -> Result<(), TomlCodecError> {
    let toml_val: toml::Value = toml::from_str(raw_toml)?;
    let p = Path::parse(prefix);
    flatten_toml_value(store, &p, &toml_val, 0)?;
    Ok(())
}

fn toml_to_value(val: &toml::Value, depth: usize) -> Option<Value> {
    if depth > MAX_TOML_DEPTH {
        return None;
    }
    match val {
        toml::Value::String(s) => Some(Value::from(s.as_str())),
        toml::Value::Integer(i) => Some(Value::from(*i)),
        toml::Value::Float(f) => Some(Value::from(*f)),
        toml::Value::Boolean(b) => Some(Value::from(*b)),
        toml::Value::Datetime(dt) => Some(Value::from(dt.to_string())),
        toml::Value::Array(arr) => {
            let mut items = Vec::with_capacity(arr.len());
            for item in arr {
                if let Some(v) = toml_to_value(item, depth + 1) {
                    items.push(v);
                }
            }
            Some(Value::Array(items))
        }
        toml::Value::Table(table) => {
            let mut btree = alloc::collections::BTreeMap::new();
            for (k, v) in table {
                if let Some(val) = toml_to_value(v, depth + 1) {
                    btree.insert(k.clone(), val);
                }
            }
            Some(Value::Map(btree))
        }
    }
}

fn flatten_toml_value(
    store: &mut MemStore,
    current_path: &Path,
    val: &toml::Value,
    depth: usize,
) -> Result<(), StoreError> {
    if depth > MAX_TOML_DEPTH {
        return Ok(());
    }

    match val {
        toml::Value::Table(table) => {
            if let Some(scalar_toml) = table.get("_value")
                && let Some(scalar_val) = toml_to_value(scalar_toml, depth + 1)
            {
                store.insert(current_path, scalar_val)?;
            }
            for (k, v) in table {
                if k == "_value" {
                    continue;
                }
                let sub_path = current_path.join(k);
                flatten_toml_value(store, &sub_path, v, depth + 1)?;
            }
        }
        toml::Value::String(s) => {
            store.insert(current_path, Value::from(s.as_str()))?;
        }
        toml::Value::Integer(i) => {
            store.insert(current_path, Value::from(*i))?;
        }
        toml::Value::Float(f) => {
            store.insert(current_path, Value::from(*f))?;
        }
        toml::Value::Boolean(b) => {
            store.insert(current_path, Value::from(*b))?;
        }
        toml::Value::Datetime(dt) => {
            store.insert(current_path, Value::from(dt.to_string()))?;
        }
        toml::Value::Array(_) => {
            if let Some(v) = toml_to_value(val, depth) {
                store.insert(current_path, v)?;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ingest_toml_roundtrip() {
        let mut store = MemStore::new();
        let sample = r#"
            [server]
            tickrate = 128
            name = "Test"
        "#;

        ingest_toml_str(&mut store, "core", sample).unwrap();
        assert_eq!(
            store
                .get_str("core/server/tickrate")
                .unwrap()
                .value
                .as_int(),
            Some(128)
        );
    }

    #[test]
    fn test_toml_array_types_and_value_coexistence() {
        let mut store = MemStore::new();
        let sample = r#"
            [graphics]
            _value = "high_quality"
            resolutions = [1080, 1440]
            rates = [59.94, 120.0]

            [graphics.advanced]
            shadows = true
        "#;

        ingest_toml_str(&mut store, "cfg", sample).unwrap();

        // Branch and scalar coexistence
        assert_eq!(
            store.get_str("cfg/graphics").unwrap().value,
            Value::from("high_quality")
        );
        assert_eq!(
            store
                .get_str("cfg/graphics/advanced/shadows")
                .unwrap()
                .value,
            Value::from(true)
        );

        // Floats inside arrays preserved
        let rates = store.get_str("cfg/graphics/rates").unwrap();
        match &rates.value {
            Value::Array(items) => {
                assert_eq!(items.len(), 2);
                assert_eq!(items[0], Value::Float(59.94));
                assert_eq!(items[1], Value::Float(120.0));
            }
            other => panic!("expected array, got {other:?}"),
        }
    }
}
