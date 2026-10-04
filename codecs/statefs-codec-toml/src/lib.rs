//! # StateFS TOML Codec
//!
//! Encodes and decodes hierarchical state trees to and from TOML format.

use statefs_core::{MemStore, Path, Store, Value};

/// Ingests a raw TOML string into a [`MemStore`] under the specified base path prefix.
pub fn ingest_toml_str(
    store: &mut MemStore,
    prefix: &str,
    raw_toml: &str,
) -> Result<(), toml::de::Error> {
    let toml_val: toml::Value = toml::from_str(raw_toml)?;
    let p = Path::parse(prefix);
    flatten_toml_value(store, &p, &toml_val);
    Ok(())
}

fn flatten_toml_value(store: &mut MemStore, current_path: &Path, val: &toml::Value) {
    match val {
        toml::Value::Table(table) => {
            for (k, v) in table {
                let sub_path = current_path.join(k);
                flatten_toml_value(store, &sub_path, v);
            }
        }
        toml::Value::String(s) => {
            store.insert(current_path, Value::from(s.as_str())).unwrap();
        }
        toml::Value::Integer(i) => {
            store.insert(current_path, Value::from(*i)).unwrap();
        }
        toml::Value::Float(f) => {
            store.insert(current_path, Value::from(*f)).unwrap();
        }
        toml::Value::Boolean(b) => {
            store.insert(current_path, Value::from(*b)).unwrap();
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
            store.insert(current_path, Value::Array(vals)).unwrap();
        }
        toml::Value::Datetime(dt) => {
            store
                .insert(current_path, Value::from(dt.to_string()))
                .unwrap();
        }
    }
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
