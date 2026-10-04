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

/// Ingests a raw TOML string into a [`MemStore`] under the specified base path prefix.
pub fn ingest_toml_str(
    store: &mut MemStore,
    prefix: &str,
    raw_toml: &str,
) -> Result<(), TomlCodecError> {
    let toml_val: toml::Value = toml::from_str(raw_toml)?;
    let p = Path::parse(prefix);
    flatten_toml_value(store, &p, &toml_val)?;
    Ok(())
}

fn flatten_toml_value(
    store: &mut MemStore,
    current_path: &Path,
    val: &toml::Value,
) -> Result<(), StoreError> {
    match val {
        toml::Value::Table(table) => {
            for (k, v) in table {
                let sub_path = current_path.join(k);
                flatten_toml_value(store, &sub_path, v)?;
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
        toml::Value::Array(arr) => {
            let vals: Vec<Value> = arr
                .iter()
                .filter_map(|v| match v {
                    toml::Value::String(s) => Some(Value::from(s.as_str())),
                    toml::Value::Integer(i) => Some(Value::from(*i)),
                    toml::Value::Boolean(b) => Some(Value::from(*b)),
                    _ => None,
                })
                .collect();
            store.insert(current_path, Value::Array(vals))?;
        }
        toml::Value::Datetime(dt) => {
            store.insert(current_path, Value::from(dt.to_string()))?;
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
}
